//! F4 workbench contracts. This module intentionally plans and validates patches; it does not
//! execute arbitrary commands or modify a workspace.

use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerNetwork {
    Denied,
}

/// F4 "Isolated worker bootstrap": how a worker's view of the workspace handles writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceWriteMode {
    /// Writes land on the real host workspace — required whenever the worker's result must
    /// persist (`git apply`: the whole point is that an approved patch reaches disk).
    Direct,
    /// Writes go to an invisible tmpfs overlay layer (`bwrap --overlay-src <root> --tmp-overlay
    /// <root>`) and vanish the moment the worker exits — the real workspace is never touched, not
    /// even transiently. Appropriate for a worker whose writes should never persist (arbitrary
    /// allowlisted test/build commands: a `cargo test` run has no business leaving artifacts in,
    /// or being able to corrupt, the user's real source tree). Verified live (2026-08-16): the
    /// overlay's tmpfs backing is real Linux memory, which counts against cgroup memory
    /// accounting exactly like any other page — a worker that writes past `WorkerLimits.
    /// max_memory_bytes` into the overlay gets OOM-killed by the *same* `MemoryMax=` cgroup that
    /// already bounds the worker's own allocations, giving a real, kernel-enforced total-write
    /// budget "for free" without a separate disk-quota mechanism.
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLimits {
    pub max_runtime_seconds: u32,
    pub max_output_bytes: u64,
    pub max_changed_files: usize,
    pub max_diff_bytes: usize,
    /// F4 "Resource kontrolü": `setrlimit` on the child right before `exec` — needs no special
    /// privilege, genuinely enforced by the kernel. Bounds one process's own address space; the
    /// `MemoryMax=` cgroup limit (`cgroup_scope_prefix`) is the whole-process-tree backstop on
    /// top of this, catching a worker that spawns children instead of allocating itself.
    pub max_memory_bytes: u64,
    /// F4 "Resource kontrolü": real cgroup v2 CPU bandwidth cap (`systemd-run --user --scope -p
    /// CPUQuota=`), applied to the *whole* worker process tree — unlike `RLIMIT_CPU` (a hard
    /// CPU-seconds ceiling on one process before it's killed), this throttles continuously and
    /// covers every process the worker spawns, not just its own. 400 = 4 cores' worth by default.
    pub max_cpu_percent: u32,
    pub network: WorkerNetwork,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        Self {
            max_runtime_seconds: 120,
            max_output_bytes: 1_000_000,
            max_changed_files: 12,
            max_diff_bytes: 256 * 1024,
            max_memory_bytes: 512 * 1024 * 1024,
            max_cpu_percent: 400,
            network: WorkerNetwork::Denied,
        }
    }
}

/// A cooperative cancel signal a caller can flip from another thread (e.g. the TUI reading a
/// `/cancel` command while a worker runs on a background thread). Distinct from the
/// `max_runtime_seconds` deadline: that is a quota the worker itself is bound by; this is the
/// user asking to stop a specific in-flight run early. Both paths converge on the same
/// terminate-then-kill machinery in `wait_with_deadline_and_cancellation`.
pub type CancelFlag = Arc<AtomicBool>;

pub fn new_cancel_flag() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerStopReason {
    /// The caller flipped the cancel flag while the child was still running.
    UserCancelled,
    /// `max_runtime_seconds` elapsed before the child exited on its own.
    RuntimeQuotaExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingPlan {
    pub schema_version: u16,
    pub plan_id: String,
    pub workspace_root: PathBuf,
    pub request_summary: String,
    pub affected_files: Vec<PathBuf>,
    pub test_plan: Vec<String>,
    pub risk_notes: Vec<String>,
    pub limits: WorkerLimits,
    /// F4 "Coding plan UX": şeyler modelin isteği yorumlarken sessizce varsaydığı noktalar (ör.
    /// "hangi dilde" gibi belirtilmemiş bir tercih). `create_read_only_coding_plan` bunu her zaman
    /// boş bırakır (salt-okunur temel kurucu hâlâ isteğe özgü bir yorum yapmıyor) —
    /// `draft_coding_plan_with_provider` modelin ürettiği varsayımları buraya dolduruyor.
    pub assumptions: Vec<String>,
    /// F4 "Coding plan UX": modelin isteği tam olarak çözemediği, kullanıcıya sorulması gereken
    /// noktalar. Boşsa model isteği yeterince net bulmuş demektir.
    pub open_questions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchProposal {
    pub schema_version: u16,
    pub proposal_id: String,
    pub plan_id: String,
    pub unified_diff: String,
    pub diff_sha256: String,
    pub affected_files: Vec<PathBuf>,
}

/// A scope-bound approval receipt. It binds one reviewed proposal ID to one exact diff hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedPatch {
    pub proposal_id: String,
    pub diff_sha256: String,
    pub approved_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSnapshot {
    pub snapshot_root: PathBuf,
    pub workspace_root: PathBuf,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchApplication {
    pub proposal_id: String,
    pub diff_sha256: String,
    pub changed_files: Vec<PathBuf>,
    pub snapshot: PatchSnapshot,
    pub verifier_evidence: Vec<String>,
}

pub fn create_read_only_coding_plan(
    workspace_root: impl AsRef<Path>,
    request_summary: impl Into<String>,
    affected_files: Vec<PathBuf>,
    test_plan: Vec<String>,
) -> Result<CodingPlan, String> {
    let workspace_root = std::fs::canonicalize(workspace_root.as_ref())
        .map_err(|error| format!("workspace root cannot be resolved: {error}"))?;
    if !workspace_root.is_dir() {
        return Err("workspace root must be a directory".into());
    }
    let request_summary = request_summary.into();
    if request_summary.trim().is_empty() {
        return Err("coding plan requires a request summary".into());
    }
    let limits = WorkerLimits::default();
    if affected_files.len() > limits.max_changed_files {
        return Err("coding plan exceeds maximum affected file count".into());
    }
    for path in &affected_files {
        validate_workspace_relative_path(path)?;
    }
    let plan_hash = hash_fields(&[
        workspace_root.to_string_lossy().as_ref(),
        &request_summary,
        &affected_files
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n"),
    ]);
    Ok(CodingPlan {
        schema_version: 1,
        plan_id: format!("plan-{}", &plan_hash[..16]),
        workspace_root,
        request_summary,
        affected_files,
        test_plan,
        risk_notes: vec![
            "Worker network is denied by default.".into(),
            "No patch is applied without a separate, scope-bound approval.".into(),
            "Only workspace-relative source paths are eligible.".into(),
        ],
        limits,
        assumptions: Vec::new(),
        open_questions: Vec::new(),
    })
}

pub fn create_patch_proposal(
    plan: &CodingPlan,
    unified_diff: impl Into<String>,
    affected_files: Vec<PathBuf>,
) -> Result<PatchProposal, String> {
    validate_coding_plan(plan)?;
    let unified_diff = unified_diff.into();
    if unified_diff.trim().is_empty() || !unified_diff.starts_with("diff --git ") {
        return Err("patch proposal must contain a unified git diff".into());
    }
    if unified_diff.len() > plan.limits.max_diff_bytes {
        return Err("patch proposal exceeds diff byte limit".into());
    }
    if affected_files.is_empty() || affected_files.len() > plan.limits.max_changed_files {
        return Err("patch proposal has an invalid affected file count".into());
    }
    for path in &affected_files {
        validate_workspace_relative_path(path)?;
        if !plan.affected_files.contains(path) {
            return Err("patch proposal changes a file outside its approved plan".into());
        }
        let rendered = path.to_string_lossy();
        if !unified_diff.contains(rendered.as_ref()) {
            return Err("patch proposal file list does not match its diff".into());
        }
    }
    let diff_sha256 = hash_fields(&[&unified_diff]);
    Ok(PatchProposal {
        schema_version: 1,
        proposal_id: format!("patch-{}", &diff_sha256[..16]),
        plan_id: plan.plan_id.clone(),
        unified_diff,
        diff_sha256,
        affected_files,
    })
}

pub fn approve_patch(
    proposal: &PatchProposal,
    user_approved: bool,
) -> Result<ApprovedPatch, String> {
    if !user_approved {
        return Err("patch application requires explicit user approval".into());
    }
    validate_patch_proposal(proposal)?;
    let approved_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    Ok(ApprovedPatch {
        proposal_id: proposal.proposal_id.clone(),
        diff_sha256: proposal.diff_sha256.clone(),
        approved_at,
    })
}

/// Applies one reviewable patch after a scope-bound approval. The patch is checked first, a
/// per-file snapshot is made outside the workspace, and any apply failure restores that snapshot.
/// The worker has no shell, no inherited special environment, and no network operation.
pub fn apply_approved_patch(
    plan: &CodingPlan,
    proposal: &PatchProposal,
    approval: &ApprovedPatch,
) -> Result<PatchApplication, String> {
    validate_coding_plan(plan)?;
    validate_patch_proposal(proposal)?;
    if approval.proposal_id != proposal.proposal_id || approval.diff_sha256 != proposal.diff_sha256
    {
        return Err("patch approval does not match the reviewed proposal hash".into());
    }
    if proposal.plan_id != plan.plan_id {
        return Err("patch proposal does not belong to this coding plan".into());
    }
    let snapshot = snapshot_files(plan, &proposal.affected_files)?;
    if let Err(error) = run_git_apply(
        &plan.workspace_root,
        &proposal.unified_diff,
        true,
        &plan.limits,
    ) {
        let _ = fs::remove_dir_all(&snapshot.snapshot_root);
        return Err(error);
    }
    if let Err(error) = run_git_apply(
        &plan.workspace_root,
        &proposal.unified_diff,
        false,
        &plan.limits,
    ) {
        let restore = restore_patch_snapshot(&snapshot);
        let cleanup = fs::remove_dir_all(&snapshot.snapshot_root);
        return Err(match (restore, cleanup) {
            (Err(restore_error), _) => format!(
                "patch apply failed: {error}; automatic snapshot restore also failed: {restore_error}"
            ),
            (_, Err(cleanup_error)) => {
                format!("patch apply failed: {error}; snapshot cleanup failed: {cleanup_error}")
            }
            _ => format!("patch apply failed and original files were restored: {error}"),
        });
    }
    let mut verifier_evidence = Vec::new();
    for relative_path in &proposal.affected_files {
        let path = plan.workspace_root.join(relative_path);
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "patch verifier cannot read {}: {error}",
                relative_path.display()
            )
        })?;
        verifier_evidence.push(format!(
            "file.sha256:{}:{}",
            relative_path.display(),
            hash_bytes(&bytes)
        ));
    }
    Ok(PatchApplication {
        proposal_id: proposal.proposal_id.clone(),
        diff_sha256: proposal.diff_sha256.clone(),
        changed_files: proposal.affected_files.clone(),
        snapshot,
        verifier_evidence,
    })
}

/// Restores exactly the files captured before an approved patch. It is exposed for an explicit
/// user-requested rollback; automatic restoration is used only when the apply step itself fails.
pub fn restore_patch_snapshot(snapshot: &PatchSnapshot) -> Result<(), String> {
    if !snapshot.workspace_root.is_absolute() || !snapshot.snapshot_root.is_absolute() {
        return Err("patch snapshot paths must be absolute".into());
    }
    for relative_path in &snapshot.files {
        validate_workspace_relative_path(relative_path)?;
        let source = snapshot.snapshot_root.join(relative_path);
        let destination = snapshot.workspace_root.join(relative_path);
        let bytes = fs::read(&source).map_err(|error| {
            format!(
                "patch snapshot is missing {}: {error}",
                relative_path.display()
            )
        })?;
        let parent = destination
            .parent()
            .ok_or_else(|| "patch destination has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("patch rollback cannot create parent: {error}"))?;
        fs::write(&destination, bytes)
            .map_err(|error| format!("patch rollback cannot restore file: {error}"))?;
    }
    Ok(())
}

pub fn discard_patch_snapshot(snapshot: PatchSnapshot) -> Result<(), String> {
    if !snapshot.snapshot_root.starts_with(std::env::temp_dir()) {
        return Err("refusing to discard a snapshot outside the temporary directory".into());
    }
    fs::remove_dir_all(&snapshot.snapshot_root)
        .map_err(|error| format!("patch snapshot cleanup failed: {error}"))
}

/// F4 "Patch generator": computes the *real* unified diff for one file deterministically, from
/// two full-content strings, via `git diff --no-index`. The reason this exists at all — a model
/// is not asked to emit diff syntax itself. Small local models are unreliable at exact hunk
/// line-number bookkeeping; asking for the *whole new file content* instead and having the
/// machine compute the true diff removes an entire class of "the model's diff doesn't apply"
/// failures, at the cost of a bigger prompt/response per file (an accepted trade for this
/// project's file-size ceiling, see `MAX_WORKSPACE_DOCUMENT_BYTES`).
///
/// Neither string ever touches the real workspace or any sandbox: both are written to an
/// ephemeral scratch directory removed before this returns, and `git diff --no-index` only reads
/// its own two temporary files — there is nothing here for an isolation boundary to add, unlike
/// `run_git_apply`, which is the one place a diff actually lands on the real workspace.
/// `Ok(None)` means the two contents are identical (a legitimate "no change needed for this
/// file" outcome, not an error).
pub fn generate_unified_diff_for_file(
    relative_path: &Path,
    old_content: &str,
    new_content: &str,
) -> Result<Option<String>, String> {
    validate_workspace_relative_path(relative_path)?;
    if old_content == new_content {
        return Ok(None);
    }
    let git = Path::new("/usr/bin/git");
    if !git.is_file() {
        return Err("patch generator requires /usr/bin/git".into());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let scratch_root = std::env::temp_dir().join(format!("jarvis-diffgen-{nonce}"));
    let relative_a = Path::new("a").join(relative_path);
    let relative_b = Path::new("b").join(relative_path);
    let old_path = scratch_root.join(&relative_a);
    let new_path = scratch_root.join(&relative_b);
    let write_scratch_file = |path: &Path, content: &str| -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "diff scratch path has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("diff scratch directory cannot be created: {error}"))?;
        fs::write(path, content)
            .map_err(|error| format!("diff scratch file cannot be written: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Both sides must share one file mode, or `git diff` emits an "old mode"/"new mode"
            // line that `diff_header_files` (deliberately) treats as an unsupported diff shape.
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o644));
        }
        Ok(())
    };
    if let Err(error) = write_scratch_file(&old_path, old_content) {
        let _ = fs::remove_dir_all(&scratch_root);
        return Err(error);
    }
    if let Err(error) = write_scratch_file(&new_path, new_content) {
        let _ = fs::remove_dir_all(&scratch_root);
        return Err(error);
    }
    let output = Command::new(git)
        .current_dir(&scratch_root)
        .arg("diff")
        .arg("--no-index")
        .arg("--no-prefix")
        .arg("--")
        .arg(&relative_a)
        .arg(&relative_b)
        .output();
    let _ = fs::remove_dir_all(&scratch_root);
    let output =
        output.map_err(|error| format!("patch generator could not run git diff: {error}"))?;
    // `git diff --no-index` uses exit code 1 to mean "a diff was produced" (not an error) and 0
    // to mean "no difference" — only anything else is a genuine failure.
    match output.status.code() {
        Some(0) => Ok(None),
        Some(1) => {
            let diff = String::from_utf8_lossy(&output.stdout).into_owned();
            if diff.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(diff))
            }
        }
        _ => Err(format!(
            "patch generator's git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )),
    }
}

pub fn validate_coding_plan(plan: &CodingPlan) -> Result<(), String> {
    if plan.schema_version != 1 {
        return Err("unsupported coding plan schema version".into());
    }
    if plan.plan_id.trim().is_empty() || plan.request_summary.trim().is_empty() {
        return Err("coding plan requires an id and request summary".into());
    }
    if !plan.workspace_root.is_absolute() || !plan.workspace_root.is_dir() {
        return Err("coding plan workspace root must be an existing absolute directory".into());
    }
    if plan.limits.network != WorkerNetwork::Denied {
        return Err("coding worker network must remain denied".into());
    }
    if plan.affected_files.len() > plan.limits.max_changed_files {
        return Err("coding plan exceeds maximum affected file count".into());
    }
    for path in &plan.affected_files {
        validate_workspace_relative_path(path)?;
    }
    Ok(())
}

pub fn validate_patch_proposal(proposal: &PatchProposal) -> Result<(), String> {
    if proposal.schema_version != 1 {
        return Err("unsupported patch proposal schema version".into());
    }
    if proposal.proposal_id.trim().is_empty()
        || proposal.plan_id.trim().is_empty()
        || proposal.diff_sha256.len() != 64
    {
        return Err("patch proposal requires ids and a SHA-256 diff hash".into());
    }
    if hash_fields(&[&proposal.unified_diff]) != proposal.diff_sha256 {
        return Err("patch proposal diff hash does not match its content".into());
    }
    let diff_files = diff_header_files(&proposal.unified_diff)?;
    if diff_files != proposal.affected_files {
        return Err("patch proposal file list does not exactly match diff headers".into());
    }
    Ok(())
}

pub(crate) fn validate_workspace_relative_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("workbench path must be non-empty and workspace-relative".into());
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("workbench path traversal is not allowed".into());
    }
    Ok(())
}

fn diff_header_files(diff: &str) -> Result<Vec<PathBuf>, String> {
    if diff.contains("new file mode")
        || diff.contains("deleted file mode")
        || diff.contains("rename from")
        || diff.contains("rename to")
        || diff.contains("GIT binary patch")
        || diff.contains("old mode")
        || diff.contains("new mode")
    {
        return Err("patch proposal may change only existing text files".into());
    }
    let mut files = Vec::new();
    for line in diff.lines().filter(|line| line.starts_with("diff --git ")) {
        let Some(paths) = line.strip_prefix("diff --git ") else {
            continue;
        };
        let mut paths = paths.split_whitespace();
        let left = paths
            .next()
            .and_then(|path| path.strip_prefix("a/"))
            .ok_or_else(|| "patch diff header has an invalid old path".to_string())?;
        let right = paths
            .next()
            .and_then(|path| path.strip_prefix("b/"))
            .ok_or_else(|| "patch diff header has an invalid new path".to_string())?;
        if paths.next().is_some() || left != right {
            return Err("patch diff header must modify one existing path in place".into());
        }
        let path = PathBuf::from(left);
        validate_workspace_relative_path(&path)?;
        if files.contains(&path) {
            return Err("patch diff contains a duplicate file header".into());
        }
        files.push(path);
    }
    if files.is_empty() {
        return Err("patch proposal requires at least one git diff header".into());
    }
    Ok(files)
}

/// F4 "Patch preview/review": splits a validated multi-file unified diff into its per-file
/// blocks, in the same order `diff_header_files` reports them. This is what makes a "seçilebilir
/// dosya scope'u" (selectable file scope) possible — a caller can show each file's diff on its
/// own and let the user approve a subset via `scope_patch_proposal_to_files`, rather than only
/// ever all-or-nothing.
pub(crate) fn split_diff_by_file(diff: &str) -> Result<Vec<(PathBuf, String)>, String> {
    let files = diff_header_files(diff)?;
    let mut blocks = Vec::with_capacity(files.len());
    let mut current_block = String::new();
    let mut current_index: Option<usize> = None;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if let Some(index) = current_index {
                blocks.push((files[index].clone(), std::mem::take(&mut current_block)));
            }
            current_index = Some(blocks.len());
        }
        current_block.push_str(line);
    }
    if let Some(index) = current_index {
        blocks.push((files[index].clone(), current_block));
    }
    Ok(blocks)
}

/// Rebuilds a new, independently-validated `PatchProposal` scoped to only `selected_files` — the
/// user approving a subset of a multi-file patch, not the whole thing. `selected_files` must each
/// already be part of `proposal.affected_files`; this never lets a caller expand scope, only
/// narrow it. The new proposal gets its own diff hash (derived from the narrower diff content),
/// so an old approval for the full proposal can never be replayed against the narrower one.
pub fn scope_patch_proposal_to_files(
    plan: &CodingPlan,
    proposal: &PatchProposal,
    selected_files: &[PathBuf],
) -> Result<PatchProposal, String> {
    if selected_files.is_empty() {
        return Err("at least one file must be selected".into());
    }
    for file in selected_files {
        if !proposal.affected_files.contains(file) {
            return Err(format!(
                "{} is not part of the reviewed patch proposal",
                file.display()
            ));
        }
    }
    let blocks = split_diff_by_file(&proposal.unified_diff)?;
    let mut scoped_diff = String::new();
    let mut scoped_files = Vec::new();
    for (path, block) in &blocks {
        if selected_files.contains(path) {
            scoped_diff.push_str(block);
            scoped_files.push(path.clone());
        }
    }
    create_patch_proposal(plan, scoped_diff, scoped_files)
}

fn snapshot_files(plan: &CodingPlan, files: &[PathBuf]) -> Result<PatchSnapshot, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let snapshot_root = std::env::temp_dir().join(format!(
        "jarvis-patch-snapshot-{}-{nonce}",
        plan.plan_id
            .replace(|character: char| !character.is_ascii_alphanumeric(), "_")
    ));
    fs::create_dir_all(&snapshot_root)
        .map_err(|error| format!("patch snapshot directory cannot be created: {error}"))?;
    for relative_path in files {
        validate_workspace_relative_path(relative_path)?;
        let source = plan.workspace_root.join(relative_path);
        let metadata = fs::metadata(&source)
            .map_err(|error| format!("patch source metadata cannot be read: {error}"))?;
        if !metadata.is_file() || metadata.len() > plan.limits.max_diff_bytes as u64 {
            let _ = fs::remove_dir_all(&snapshot_root);
            return Err("patch source must be a bounded regular file".into());
        }
        let destination = snapshot_root.join(relative_path);
        let parent = destination
            .parent()
            .ok_or_else(|| "patch snapshot destination has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("patch snapshot cannot create parent: {error}"))?;
        fs::copy(&source, &destination)
            .map_err(|error| format!("patch snapshot cannot copy source: {error}"))?;
    }
    Ok(PatchSnapshot {
        snapshot_root,
        workspace_root: plan.workspace_root.clone(),
        files: files.to_vec(),
    })
}

fn run_git_apply(
    workspace_root: &Path,
    diff: &str,
    check_only: bool,
    limits: &WorkerLimits,
) -> Result<(), String> {
    let mut command = isolated_git_apply_command(workspace_root, check_only, limits)?;
    apply_worker_rlimits(&mut command, limits);
    let mut child = command
        .spawn()
        .map_err(|error| format!("isolated patch worker could not start git apply: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "isolated patch worker stdin is unavailable".to_string())?
        .write_all(diff.as_bytes())
        .map_err(|error| format!("isolated patch worker could not receive diff: {error}"))?;
    // stdin is dropped here (its handle was a temporary above) — the child sees EOF and can run
    // to completion. F4 "Resource kontrolü": `WorkerLimits.max_runtime_seconds` used to be a
    // struct field nobody ever read — a hung or pathological `git apply` inside the sandbox could
    // block forever with no watchdog.
    wait_with_timeout(&mut child, limits.max_runtime_seconds)?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("isolated patch worker did not complete: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // `max_output_bytes` bounds the preview surfaced to the caller (a display-only cap, not a
    // live kill-on-overflow while the process is still running). git's own stderr for a rejected
    // `apply` is inherently small and already indirectly bounded by `max_diff_bytes` (a 256 KiB
    // diff cannot itself provoke a multi-megabyte error) — a live streaming cap would only matter
    // once an *arbitrary* command runner exists (F4 "Allowlist command runner", not built yet).
    let preview_chars = (limits.max_output_bytes as usize).clamp(200, 2_000);
    let preview: String = stderr.chars().take(preview_chars).collect();
    Err(format!("isolated patch worker rejected diff: {preview}"))
}

/// Polls `child` for exit instead of blocking on it (`Child::wait`/`wait_with_output` would block
/// indefinitely), killing it once `max_runtime_seconds` elapses. Thin wrapper over
/// `wait_with_deadline_and_cancellation` with no cancel flag, kept for the existing `git apply`
/// call site and its tests.
fn wait_with_timeout(child: &mut Child, max_runtime_seconds: u32) -> Result<(), String> {
    wait_with_deadline_and_cancellation(child, max_runtime_seconds, None).map_err(
        |(reason, message)| {
            debug_assert_eq!(reason, WorkerStopReason::RuntimeQuotaExceeded);
            message
        },
    )
}

/// F4 "Gerçek cancellation": `task cancel -> child process signal -> grace period -> kill ->
/// cleanup`. Polls `child` for exit (never blocks indefinitely), and stops it early for either of
/// two reasons: the caller flipped `cancel` from another thread, or `max_runtime_seconds` elapsed
/// on its own. Either way the child is asked to exit first (`SIGTERM`) and only escalated to a
/// hard `SIGKILL` after a short grace period — a process that exits cleanly on `SIGTERM` never
/// sees the harder signal. Returns `Ok(())` once the child has genuinely exited by itself;
/// `Err((reason, message))` only when it had to be stopped, so a caller can tell a user-requested
/// cancel apart from a quota timeout (they are audited under different event names).
pub(crate) fn wait_with_deadline_and_cancellation(
    child: &mut Child,
    max_runtime_seconds: u32,
    cancel: Option<&CancelFlag>,
) -> Result<(), (WorkerStopReason, String)> {
    const GRACE_PERIOD: Duration = Duration::from_millis(200);
    let deadline = Instant::now() + Duration::from_secs(u64::from(max_runtime_seconds));
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return Ok(()),
            Ok(None) => {
                let cancelled = cancel.is_some_and(|flag| flag.load(Ordering::SeqCst));
                let timed_out = Instant::now() >= deadline;
                if cancelled || timed_out {
                    let reason = if cancelled {
                        WorkerStopReason::UserCancelled
                    } else {
                        WorkerStopReason::RuntimeQuotaExceeded
                    };
                    terminate_then_kill(child, GRACE_PERIOD);
                    let explanation = match reason {
                        WorkerStopReason::UserCancelled => {
                            "isolated worker was cancelled by the user and terminated".to_string()
                        }
                        WorkerStopReason::RuntimeQuotaExceeded => format!(
                            "isolated worker exceeded its {max_runtime_seconds}s runtime quota and was terminated"
                        ),
                    };
                    return Err((reason, explanation));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                return Err((
                    WorkerStopReason::RuntimeQuotaExceeded,
                    format!("isolated worker status check failed: {error}"),
                ));
            }
        }
    }
}

/// Sends `SIGTERM` and gives the child `grace_period` to exit on its own before escalating to a
/// hard `SIGKILL`. A process that ignores or ends up unaffected by `SIGTERM` (or exits fast
/// enough that the signal never even matters) is still guaranteed to be gone by the time this
/// returns — `child.wait()` is always called last so the process is fully reaped, never a
/// zombie.
pub(crate) fn terminate_then_kill(child: &mut Child, grace_period: Duration) {
    // SAFETY: `libc::kill` with a valid pid and a standard signal number is a plain syscall with
    // no aliasing/lifetime requirements Rust needs to uphold here.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let grace_deadline = Instant::now() + grace_period;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return,
            Ok(None) => {
                if Instant::now() >= grace_deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Number of *threads* (not processes — see below) the real UID running this process currently
/// owns, system-wide: for every numeric `/proc` entry whose owner uid matches, the number of
/// entries under its `task/` subdirectory, summed. Used to compute a headroom-above-current
/// `RLIMIT_NPROC` instead of a fixed absolute number — see the bug this fixed, documented on
/// `apply_worker_rlimits`. Must run in the parent, before `fork`; `/proc` traversal is not
/// async-signal-safe, so this cannot itself live inside a `pre_exec` closure.
///
/// **Counting threads, not processes, is not optional**: `RLIMIT_NPROC` is documented (`man
/// setrlimit`) as counting threads ("more precisely, on Linux, threads"), and an ordinary desktop
/// is overwhelmingly threads, not processes — verified live (2026-08-16) on this machine: ~170
/// top-level processes for the session's UID, but ~1837 threads once every process's `task/`
/// entries are summed (a single browser process alone holds dozens). A first version of this
/// function counted only top-level `/proc/PID` entries (one per process) and, even with generous
/// headroom added on top, still landed far below the real in-kernel count — reproduced live with
/// `prlimit --nproc=428:428 --pid=$$` on this same shell, which *still* failed a plain `bwrap`
/// invocation the same way the too-low fixed `64` originally did.
fn current_thread_count_for_real_uid() -> u64 {
    use std::os::unix::fs::MetadataExt;
    let real_uid = unsafe { libc::getuid() };
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter(|entry| {
            entry
                .metadata()
                .map(|metadata| metadata.uid() == real_uid)
                .unwrap_or(false)
        })
        .map(|entry| {
            fs::read_dir(entry.path().join("task"))
                .map(|tasks| tasks.count() as u64)
                .unwrap_or(1)
        })
        .sum()
}

/// Applies `setrlimit` to the child right before `exec` (F4 "Resource kontrolü"). Needs no
/// special privilege — a process may always lower its own limits — and is genuinely enforced by
/// the kernel, unlike the struct fields this module used to leave unread. Four limits, each
/// targeting one line of the F4 threat model:
/// - `RLIMIT_AS` (`limits.max_memory_bytes`): bounds the child's own address space.
/// - `RLIMIT_FSIZE`: bounds any single file the child writes, independent of `max_memory_bytes`
///   — a compiler/test run can legitimately produce build artifacts far larger than a source
///   file, so this is deliberately more generous than `max_diff_bytes`.
/// - `RLIMIT_NPROC`: bounds how many *additional* processes the worker's user may hold at once —
///   a real, if blunt, fork-bomb backstop (blunt because the limit is per-uid, not per-child-tree;
///   the PID namespace from `--unshare-pid` is the sharper containment for "how many processes can
///   this worker itself spawn"). **Bug found and fixed live (2026-08-16):** this used to be a
///   fixed `64`. `RLIMIT_NPROC` is a per-real-UID limit counted *system-wide*, not "how many
///   descendants of this process" — on any ordinary desktop the user's UID already owns far more
///   than 64 processes (168 on this machine at the time this was found: browser, IDE, JARVIS's own
///   services, ...), so setting a fixed `64` made *any* further `fork`/`clone` from the rlimited
///   process fail immediately with `EAGAIN`, including bwrap's own internal
///   `unshare(CLONE_NEWUSER)` call — silently breaking the entire isolated worker path. Verified
///   live: `prlimit --nproc=64:64 --pid=$$` on an ordinary shell (168 pre-existing processes) made
///   even a plain `fork()` fail with the exact "Resource temporarily unavailable" the isolated
///   worker was hitting. This had been misdiagnosed in an earlier session as the *sandbox* denying
///   namespace creation — it was this bug the whole time. The fix keeps the same real backstop
///   (bounded new-process budget) without breaking on a normal, already-busy desktop session.
pub(crate) fn apply_worker_rlimits(command: &mut Command, limits: &WorkerLimits) {
    let max_memory_bytes = limits.max_memory_bytes;
    let max_runtime_seconds = u64::from(limits.max_runtime_seconds.max(1));
    // 1024 headroom, not a smaller number: the ambient thread count on a real desktop session
    // moves on its own between this measurement and the child's actual `fork`/`clone` a few
    // milliseconds later (background browser/IDE activity) — still a real, finite fork-bomb
    // backstop, just nowhere near "unlimited" (`ulimit -u` with no rlimit at all is ~250,000 on
    // this machine).
    const NPROC_HEADROOM: u64 = 1024;
    let nproc = current_thread_count_for_real_uid().saturating_add(NPROC_HEADROOM);
    // SAFETY: `pre_exec` runs in the forked child before `exec`, between `fork` and `exec` — only
    // async-signal-safe calls are allowed there. `setrlimit` is async-signal-safe. Each call sets
    // a limit on the child itself; it cannot affect the parent (this) process's own limits.
    unsafe {
        command.pre_exec(move || {
            let as_limit = libc::rlimit {
                rlim_cur: max_memory_bytes,
                rlim_max: max_memory_bytes,
            };
            libc::setrlimit(libc::RLIMIT_AS, &as_limit);
            let fsize_bytes: u64 = 64 * 1024 * 1024;
            let fsize_limit = libc::rlimit {
                rlim_cur: fsize_bytes,
                rlim_max: fsize_bytes,
            };
            libc::setrlimit(libc::RLIMIT_FSIZE, &fsize_limit);
            let nproc_limit = libc::rlimit {
                rlim_cur: nproc,
                rlim_max: nproc,
            };
            libc::setrlimit(libc::RLIMIT_NPROC, &nproc_limit);
            let cpu_seconds = max_runtime_seconds.saturating_mul(16);
            let cpu_limit = libc::rlimit {
                rlim_cur: cpu_seconds,
                rlim_max: cpu_seconds,
            };
            libc::setrlimit(libc::RLIMIT_CPU, &cpu_limit);
            Ok(())
        });
    }
}

/// F4 "Resource kontrolü": wraps `command` in a real cgroup v2 scope (`systemd-run --user
/// --scope`) enforcing `limits.max_memory_bytes`/`limits.max_cpu_percent` on the *whole* worker
/// process tree, not just the single process `apply_worker_rlimits` bounds. `MemorySwapMax=0` is
/// required, not cosmetic — verified live (2026-08-16): with swap available, exceeding
/// `MemoryMax` alone just pushes pages to swap instead of triggering the kernel OOM-kill; only
/// with swap denied inside the scope does a genuinely over-budget worker actually get killed.
/// `--quiet` is load-bearing too: without it, systemd-run's own "Running as unit: ..." banner
/// lands on the *wrapped* command's stdout, corrupting the real worker's output. Not gated behind
/// `#[cfg(not(test))]` like `isolated_worker_command` is — building these args has no namespace/
/// process side effects of its own, so the real-kernel test below can call it directly.
fn cgroup_scope_prefix(command: &mut Command, limits: &WorkerLimits) {
    command
        .arg("--user")
        .arg("--scope")
        .arg("--quiet")
        .arg("--collect")
        .arg(format!("-pMemoryMax={}", limits.max_memory_bytes))
        .arg("-pMemorySwapMax=0")
        .arg(format!("-pCPUQuota={}%", limits.max_cpu_percent.max(1)))
        .arg("--");
}

/// The release worker sees only the approved workspace, read-only runtime libraries (plus
/// `extra_ro_binds` — e.g. a rustup-style `~/.cargo/bin` that lives outside `/usr`), a private
/// `/tmp`, no inherited environment and a new network namespace, inside a real cgroup v2 scope
/// (`cgroup_scope_prefix`) bounding the whole tree's RAM/CPU. Failure to start systemd-run or
/// bubblewrap is an execution failure, not permission to fall back to the host shell. Shared by
/// `git apply` (F4 patch apply) and the allowlist command runner (F4 "Allowlist command runner" /
/// "Test/verifier runner") — one isolation boundary, not two independently-maintained ones.
#[cfg(not(test))]
pub(crate) fn isolated_worker_command(
    workspace_root: &Path,
    chdir_relative: Option<&Path>,
    extra_ro_binds: &[PathBuf],
    program: &Path,
    args: &[&str],
    limits: &WorkerLimits,
    write_mode: WorkspaceWriteMode,
) -> Result<Command, String> {
    let workspace_root = fs::canonicalize(workspace_root)
        .map_err(|error| format!("isolated worker cannot resolve workspace: {error}"))?;
    if let Some(relative) = chdir_relative {
        validate_workspace_relative_path(relative)?;
    }
    let chdir = match chdir_relative {
        Some(relative) => workspace_root.join(relative),
        None => workspace_root.clone(),
    };
    let systemd_run = Path::new("/usr/bin/systemd-run");
    if !systemd_run.is_file() {
        return Err("isolated worker requires /usr/bin/systemd-run".into());
    }
    let bubblewrap = Path::new("/usr/bin/bwrap");
    if !bubblewrap.is_file() {
        return Err("isolated worker requires /usr/bin/bwrap".into());
    }
    if !program.is_file() {
        return Err(format!(
            "isolated worker program is unavailable: {}",
            program.display()
        ));
    }
    for runtime_path in [Path::new("/usr"), Path::new("/lib"), Path::new("/lib64")] {
        if !runtime_path.exists() {
            return Err(format!(
                "isolated worker runtime path is unavailable: {}",
                runtime_path.display()
            ));
        }
    }
    let mut command = Command::new(systemd_run);
    cgroup_scope_prefix(&mut command, limits);
    command
        .arg(bubblewrap)
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--unshare-net")
        // F4 threat model'in "process tree" maddesi (ADR-0001): worker'ın host'taki başka
        // süreçleri görmesi/sinyallemesi/PID'lerini keşfetmesi mümkün olmamalı. Bu üçü de
        // bwrap'ın standart, blob/derleme gerektirmeyen namespace bayrakları — `--proc /proc`
        // zaten var olduğu için yeni PID namespace'i doğru `/proc` görünümüyle çalışır.
        .arg("--unshare-pid")
        .arg("--unshare-ipc")
        .arg("--unshare-uts")
        .arg("--clearenv")
        // Bug found and fixed live (2026-08-16): `--tmpfs /tmp` must be mounted *before* any bind
        // that might land inside `/tmp` — bwrap applies mounts in argument order, so a later,
        // broader mount at a parent path (here, a fresh private `/tmp`) shadows/hides an earlier,
        // more specific bind mounted underneath it. This used to come *after* the workspace bind;
        // a workspace whose real path happens to live under `/tmp` (routine — e.g.
        // `std::env::temp_dir()`-based scratch roots, exactly what a coding-plan fixture or a
        // real user session's temp workspace can be) would silently vanish inside the sandbox,
        // surfacing as `bwrap: Can't chdir to <path>: No such file or directory` — reproduced live
        // with the exact patch-apply path this module implements.
        .args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"])
        .args(["--ro-bind", "/usr", "/usr"])
        .args(["--ro-bind", "/lib", "/lib"])
        .args(["--ro-bind", "/lib64", "/lib64"]);
    for bind in extra_ro_binds {
        command.arg("--ro-bind").arg(bind).arg(bind);
    }
    match write_mode {
        WorkspaceWriteMode::Direct => {
            command
                .arg("--bind")
                .arg(&workspace_root)
                .arg(&workspace_root);
        }
        WorkspaceWriteMode::Overlay => {
            command
                .arg("--overlay-src")
                .arg(&workspace_root)
                .arg("--tmp-overlay")
                .arg(&workspace_root);
        }
    }
    command
        .args(["--setenv", "HOME", "/nonexistent"])
        .args(["--setenv", "PATH", "/usr/bin"])
        .arg("--chdir")
        .arg(&chdir)
        .arg("--")
        .arg(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

/// Unit tests prove patch/command semantics using a controlled temporary folder rather than
/// spawning real `systemd-run`/`bwrap` processes — `cargo test` runs many tests in parallel, and
/// each real invocation creates its own set of Linux namespaces; doing that hundreds of times
/// concurrently is exactly the kind of load that can transiently exhaust a sandboxed CI runner's
/// namespace budget (`bwrap: Creating new namespace failed: Resource temporarily unavailable`).
/// Production builds never use this path; the real cgroup/namespace mechanism is instead proven
/// once, directly, by `cgroup_memory_limit_is_enforced_by_the_real_kernel_when_available` below.
#[cfg(test)]
pub(crate) fn isolated_worker_command(
    workspace_root: &Path,
    chdir_relative: Option<&Path>,
    _extra_ro_binds: &[PathBuf],
    program: &Path,
    args: &[&str],
    _limits: &WorkerLimits,
    _write_mode: WorkspaceWriteMode,
) -> Result<Command, String> {
    if let Some(relative) = chdir_relative {
        validate_workspace_relative_path(relative)?;
    }
    let workspace_root = match chdir_relative {
        Some(relative) => workspace_root.join(relative),
        None => workspace_root.to_path_buf(),
    };
    let mut command = Command::new(program);
    command
        .current_dir(workspace_root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn isolated_git_apply_command(
    workspace_root: &Path,
    check_only: bool,
    limits: &WorkerLimits,
) -> Result<Command, String> {
    let git = Path::new("/usr/bin/git");
    #[cfg(not(test))]
    if !git.is_file() {
        return Err("isolated patch worker requires /usr/bin/git".into());
    }
    let mut args = vec!["apply"];
    if check_only {
        args.push("--check");
    }
    args.extend(["--whitespace=error", "-"]);
    // `git apply` is the one worker whose whole point is that a successful write reaches the real
    // workspace — `WorkspaceWriteMode::Direct`, not `Overlay` (which the allowlist command runner
    // uses instead, since a test/build command's writes should never persist at all).
    let mut command = isolated_worker_command(
        workspace_root,
        None,
        &[],
        git,
        &args,
        limits,
        WorkspaceWriteMode::Direct,
    )?;
    command.stdout(Stdio::null());
    Ok(command)
}

fn hash_fields(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F4 "Patch generator": two full-content strings must produce a diff whose header exactly
    /// matches what `validate_patch_proposal`/`diff_header_files` expect — a single `a/<path>`,
    /// `b/<path>` pair with the same relative path on both sides, no rename/mode-change noise.
    #[test]
    fn generated_diff_has_exactly_the_header_shape_the_validator_expects() {
        let relative_path = PathBuf::from("src/example.rs");
        let diff = generate_unified_diff_for_file(&relative_path, "old\n", "new\n")
            .expect("git diff must run")
            .expect("contents differ, a diff must be produced");
        assert!(diff.starts_with("diff --git a/src/example.rs b/src/example.rs"));
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
        // The generated diff must itself pass the same validation a model-authored diff would.
        let plan = create_read_only_coding_plan(
            std::env::current_dir().expect("workspace cwd"),
            "Generated diff smoke test.",
            vec![relative_path.clone()],
            vec![],
        )
        .expect("valid plan");
        let proposal = create_patch_proposal(&plan, diff, vec![relative_path])
            .expect("a machine-generated diff must satisfy the same validator a model's would");
        assert_eq!(
            proposal.affected_files,
            vec![PathBuf::from("src/example.rs")]
        );
    }

    /// Identical contents must not be reported as a diff at all — this is what lets a caller skip
    /// a file the model decided needs no change, rather than trying (and failing) to build an
    /// empty diff for it.
    #[test]
    fn identical_contents_produce_no_diff() {
        let relative_path = PathBuf::from("src/unchanged.rs");
        let result = generate_unified_diff_for_file(&relative_path, "same\n", "same\n")
            .expect("git diff must run even with no difference");
        assert!(result.is_none());
    }

    /// F4 "Resource kontrolü": `WorkerLimits.max_runtime_seconds` bir alan olarak vardı ama
    /// hiçbir yerde okunmuyordu — gerçek bir watchdog yoktu. `sleep` gerçekten `git apply`'den
    /// çok daha uzun süren, kasıtlı bir "asılı" süreç — `max_runtime_seconds=0` ile neredeyse
    /// anında öldürüldüğünü kanıtlar (testin kendisi birkaç saniye değil, birkaç on milisaniye
    /// sürer).
    #[test]
    fn wait_with_timeout_kills_a_process_that_outlives_its_quota() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("sleep must be available to run this test");
        let started = Instant::now();
        let result = wait_with_timeout(&mut child, 0);
        assert!(
            result.is_err(),
            "a hung process must be reported as an error, not silently waited out"
        );
        assert!(
            result.unwrap_err().contains("runtime quota"),
            "the error must explain why the process was killed"
        );
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "the watchdog must actually kill the process, not wait for it to finish on its own"
        );
    }

    /// Sıradan durum: süreç kotanın içinde kendiliğinden bitiyorsa hiçbir hata dönmemeli.
    #[test]
    fn wait_with_timeout_succeeds_for_a_process_that_finishes_in_time() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("the `true` coreutil must be available to run this test");
        assert!(wait_with_timeout(&mut child, 5).is_ok());
    }

    /// F4 "Gerçek cancellation": a user-triggered cancel (flag flipped from another thread) must
    /// stop a still-running child well before its runtime quota would — proves the cancel path is
    /// genuinely wired, not just the deadline path already covered above.
    #[test]
    fn a_cancelled_process_is_stopped_long_before_its_runtime_quota() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("sleep must be available to run this test");
        let cancel = new_cancel_flag();
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            cancel_for_thread.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let result = wait_with_deadline_and_cancellation(&mut child, 120, Some(&cancel));
        let (reason, message) = result.expect_err("a cancelled worker must be reported as stopped");
        assert_eq!(reason, WorkerStopReason::UserCancelled);
        assert!(message.contains("cancelled"));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cancellation must not wait anywhere near the 120s quota"
        );
    }

    /// A process that ignores `SIGTERM` (traps it away) must still be forced dead via `SIGKILL`
    /// once the grace period elapses — proves the escalation path, not just the polite one.
    #[test]
    fn a_process_that_ignores_sigterm_is_escalated_to_sigkill() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "trap '' TERM; sleep 5"])
            .spawn()
            .expect("sh must be available to run this test");
        let started = Instant::now();
        let result = wait_with_deadline_and_cancellation(&mut child, 0, None);
        let (reason, _message) = result.expect_err("a SIGTERM-immune child must still be killed");
        assert_eq!(reason, WorkerStopReason::RuntimeQuotaExceeded);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "SIGKILL escalation must happen shortly after the grace period, not hang"
        );
    }

    /// F4 "Resource kontrolü": a child whose `RLIMIT_FSIZE` is exceeded must fail to write past
    /// that size — proves the rlimit is genuinely applied to the child, not merely stored as an
    /// unread struct field (the bug this same module already fixed once for the runtime quota).
    #[test]
    fn a_child_process_is_bound_by_the_configured_memory_rlimit() {
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            // Ask the shell's own `ulimit -v` (RLIMIT_AS, in KiB) to prove the limit that was set
            // via `pre_exec` is visible from inside the child — a direct, deterministic read of
            // the kernel-enforced value rather than trying to provoke an allocation failure.
            "ulimit -v",
        ]);
        let limits = WorkerLimits {
            max_memory_bytes: 256 * 1024 * 1024,
            ..WorkerLimits::default()
        };
        apply_worker_rlimits(&mut command, &limits);
        let output = command.output().expect("sh must be available");
        let reported_kib: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("ulimit -v must print a number of KiB");
        assert_eq!(reported_kib * 1024, limits.max_memory_bytes);
    }

    /// F4 "Resource kontrolü": proves `cgroup_scope_prefix` genuinely constrains a worker's total
    /// memory at the kernel level, complementing (not duplicating) the rlimit test above — a
    /// process that actually *touches* enough pages to exceed `MemoryMax` must be killed, not
    /// merely throttled. `MemorySwapMax=0` is what makes this true: verified live (2026-08-16)
    /// that on a machine with swap, exceeding `MemoryMax` alone just pushes pages to swap instead
    /// of triggering the OOM-killer — and that an *untouched* `bytearray()` can stay backed by the
    /// shared zero page and never grow real RSS at all, which would make a naive version of this
    /// test pass for the wrong reason. Skips (does not fail) when this environment has no
    /// functioning `systemd-run --user` session (e.g. a minimal CI container with no user systemd
    /// instance) — the mechanism itself is real-kernel and environment-dependent, same category as
    /// the rlimit test; production code fails loudly on that same unavailability instead
    /// (`isolated_worker_command`'s explicit `/usr/bin/systemd-run` check) rather than silently
    /// degrading, so a skip here never hides a real production gap.
    #[test]
    fn cgroup_memory_limit_is_enforced_by_the_real_kernel_when_available() {
        let probe = std::process::Command::new("systemd-run")
            .args(["--user", "--scope", "--quiet", "--", "/bin/true"])
            .status();
        if !matches!(probe, Ok(status) if status.success()) {
            eprintln!(
                "skipping cgroup_memory_limit_is_enforced_by_the_real_kernel_when_available: \
                 no functioning `systemd-run --user` session in this environment"
            );
            return;
        }
        let limits = WorkerLimits {
            max_memory_bytes: 64 * 1024 * 1024,
            ..WorkerLimits::default()
        };
        let mut command = std::process::Command::new("systemd-run");
        cgroup_scope_prefix(&mut command, &limits);
        let status = command
            .arg("/usr/bin/python3")
            .args([
                "-c",
                "data = bytearray(400 * 1024 * 1024)\n\
                 for i in range(0, len(data), 4096): data[i] = 1\n\
                 print('should not reach here')",
            ])
            .status()
            .expect("systemd-run must be spawnable when the probe above succeeded");
        assert!(
            !status.success(),
            "a worker that actually touches memory past MemoryMax (swap denied) must be killed"
        );
    }

    /// F4 "Isolated worker bootstrap": proves `WorkspaceWriteMode::Overlay` (used by the
    /// allowlist command runner, `command_runner.rs`) is genuinely a one-way mirror — a worker
    /// can modify an existing file and create brand-new ones, but *none* of it ever reaches the
    /// real host workspace, not even after the worker exits successfully. Builds the exact same
    /// `bwrap --overlay-src <root> --tmp-overlay <root>` invocation `isolated_worker_command`'s
    /// `#[cfg(not(test))]` branch does (this test module is itself compiled under `cfg(test)`, so
    /// calling `isolated_worker_command` directly would silently hit the plain-process bypass
    /// instead — same reason the cgroup test above builds its own `Command`). Skips (does not
    /// fail) without a functioning `systemd-run --user` session, same rationale as that test.
    #[test]
    fn overlay_write_mode_never_lets_a_workers_writes_reach_the_real_workspace() {
        let probe = std::process::Command::new("systemd-run")
            .args(["--user", "--scope", "--quiet", "--", "/bin/true"])
            .status();
        if !matches!(probe, Ok(status) if status.success()) {
            eprintln!(
                "skipping overlay_write_mode_never_lets_a_workers_writes_reach_the_real_workspace: \
                 no functioning `systemd-run --user` session in this environment"
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "jarvis-overlay-write-mode-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("test workspace dir");
        fs::write(dir.join("existing.txt"), "original\n").expect("fixture file");

        let limits = WorkerLimits::default();
        let mut command = std::process::Command::new("systemd-run");
        cgroup_scope_prefix(&mut command, &limits);
        let status = command
            .arg("/usr/bin/bwrap")
            .arg("--die-with-parent")
            .arg("--new-session")
            .arg("--unshare-net")
            .arg("--unshare-pid")
            .arg("--unshare-ipc")
            .arg("--unshare-uts")
            .arg("--clearenv")
            .args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"])
            .args(["--ro-bind", "/usr", "/usr"])
            .args(["--ro-bind", "/lib", "/lib"])
            .args(["--ro-bind", "/lib64", "/lib64"])
            .arg("--overlay-src")
            .arg(&dir)
            .arg("--tmp-overlay")
            .arg(&dir)
            .args(["--setenv", "PATH", "/usr/bin"])
            .arg("--chdir")
            .arg(&dir)
            .arg("--")
            .arg("/usr/bin/sh")
            .arg("-c")
            .arg("echo modified > existing.txt; echo new > brand-new.txt")
            .status()
            .expect("bwrap must be spawnable when the probe above succeeded");
        assert!(status.success(), "the worker command itself must succeed");

        let existing_after = fs::read_to_string(dir.join("existing.txt")).expect("file survives");
        assert_eq!(
            existing_after, "original\n",
            "an overlay-mode worker's write to an existing file must never reach the real host"
        );
        assert!(
            !dir.join("brand-new.txt").exists(),
            "an overlay-mode worker's brand-new file must never appear on the real host"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn coding_plan_is_read_only_bounded_and_network_denied() {
        let plan = create_read_only_coding_plan(
            std::env::current_dir().expect("workspace cwd"),
            "Add a test for the parser.",
            vec![PathBuf::from("src/lib.rs")],
            vec!["cargo test".into()],
        )
        .expect("valid plan");
        assert_eq!(plan.limits.network, WorkerNetwork::Denied);
        assert!(plan.risk_notes.iter().any(|note| note.contains("approval")));
        validate_coding_plan(&plan).expect("plan remains valid");
    }

    #[test]
    fn patch_requires_diff_hash_and_plan_containment() {
        let plan = create_read_only_coding_plan(
            std::env::current_dir().expect("workspace cwd"),
            "Fix a test.",
            vec![PathBuf::from("src/lib.rs")],
            vec!["cargo test".into()],
        )
        .expect("valid plan");
        let patch = create_patch_proposal(
            &plan,
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n",
            vec![PathBuf::from("src/lib.rs")],
        )
        .expect("contained proposal");
        assert_eq!(patch.diff_sha256.len(), 64);
        assert!(create_patch_proposal(
            &plan,
            "diff --git a/src/other.rs b/src/other.rs\n",
            vec![PathBuf::from("src/other.rs")],
        )
        .is_err());
    }

    #[test]
    fn traversal_and_network_relaxation_are_rejected() {
        assert!(create_read_only_coding_plan(
            std::env::current_dir().expect("workspace cwd"),
            "Unsafe path.",
            vec![PathBuf::from("../secret")],
            vec![],
        )
        .is_err());
    }

    #[test]
    fn approved_patch_is_hash_bound_applied_verified_and_rollbackable() {
        let root = std::env::temp_dir().join(format!(
            "jarvis-workbench-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("fixture workspace");
        fs::write(root.join("demo.txt"), "old\n").expect("fixture file");
        let plan = create_read_only_coding_plan(
            &root,
            "Replace the demo text.",
            vec![PathBuf::from("demo.txt")],
            vec!["read demo.txt".into()],
        )
        .expect("valid plan");
        let proposal = create_patch_proposal(
            &plan,
            "diff --git a/demo.txt b/demo.txt\n--- a/demo.txt\n+++ b/demo.txt\n@@ -1 +1 @@\n-old\n+new\n",
            vec![PathBuf::from("demo.txt")],
        )
        .expect("valid proposal");
        assert!(approve_patch(&proposal, false).is_err());
        let approval = approve_patch(&proposal, true).expect("explicit approval");
        let application =
            apply_approved_patch(&plan, &proposal, &approval).expect("approved patch applies");
        assert_eq!(fs::read_to_string(root.join("demo.txt")).unwrap(), "new\n");
        assert!(application
            .verifier_evidence
            .iter()
            .any(|evidence| evidence.starts_with("file.sha256:demo.txt:")));
        restore_patch_snapshot(&application.snapshot).expect("explicit rollback works");
        assert_eq!(fs::read_to_string(root.join("demo.txt")).unwrap(), "old\n");
        discard_patch_snapshot(application.snapshot).expect("snapshot cleanup");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn patch_cannot_change_an_unplanned_or_tampered_diff() {
        let plan = create_read_only_coding_plan(
            std::env::current_dir().expect("workspace cwd"),
            "Keep scope narrow.",
            vec![PathBuf::from("src/lib.rs")],
            vec![],
        )
        .expect("valid plan");
        let mut proposal = create_patch_proposal(
            &plan,
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n",
            vec![PathBuf::from("src/lib.rs")],
        )
        .expect("valid proposal");
        proposal.unified_diff.push_str("# tampered");
        assert!(validate_patch_proposal(&proposal)
            .unwrap_err()
            .contains("hash"));
    }

    fn two_file_plan_and_proposal() -> (CodingPlan, PatchProposal) {
        let plan = create_read_only_coding_plan(
            std::env::current_dir().expect("workspace cwd"),
            "İki dosyayı değiştir.",
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/model.rs")],
            vec![],
        )
        .expect("valid plan");
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n\
diff --git a/src/model.rs b/src/model.rs\n--- a/src/model.rs\n+++ b/src/model.rs\n@@ -1 +1 @@\n-a\n+b\n";
        let proposal = create_patch_proposal(
            &plan,
            diff,
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/model.rs")],
        )
        .expect("valid two-file proposal");
        (plan, proposal)
    }

    /// F4 "Patch preview/review": bir çok-dosyalı diff, her dosyanın kendi bloğuna doğru şekilde
    /// bölünmeli — parçalanan bloklar birleştirildiğinde orijinal diff'i tam olarak yeniden
    /// üretmeli (kayıp/tekrar yok).
    #[test]
    fn a_multi_file_diff_splits_into_exactly_its_files_reassembling_losslessly() {
        let (_plan, proposal) = two_file_plan_and_proposal();
        let blocks = split_diff_by_file(&proposal.unified_diff).expect("splits cleanly");
        assert_eq!(
            blocks
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/model.rs")]
        );
        let reassembled: String = blocks.iter().map(|(_, block)| block.as_str()).collect();
        assert_eq!(reassembled, proposal.unified_diff);
    }

    /// Kullanıcı çok-dosyalı bir patch'in yalnız bir alt kümesini onaylayabilmeli — sonuç kendi
    /// başına geçerli, kendi hash'ine sahip bağımsız bir `PatchProposal` olmalı.
    #[test]
    fn a_user_can_scope_approval_to_a_subset_of_a_multi_file_patch() {
        let (plan, proposal) = two_file_plan_and_proposal();
        let scoped =
            scope_patch_proposal_to_files(&plan, &proposal, &[PathBuf::from("src/lib.rs")])
                .expect("scoping to one file succeeds");
        assert_eq!(scoped.affected_files, vec![PathBuf::from("src/lib.rs")]);
        assert!(scoped.unified_diff.contains("src/lib.rs"));
        assert!(!scoped.unified_diff.contains("src/model.rs"));
        assert_ne!(
            scoped.diff_sha256, proposal.diff_sha256,
            "a narrower proposal must never reuse the full proposal's hash"
        );
        validate_patch_proposal(&scoped).expect("scoped proposal is independently valid");
    }

    /// Bir dosya seçilmeye çalışılırsa ama orijinal proposal'ın parçası değilse (icat edilmiş bir
    /// yol), scope daraltma reddedilmeli — genişletme asla mümkün olmamalı, yalnız daraltma.
    #[test]
    fn scoping_to_a_file_outside_the_original_proposal_is_rejected() {
        let (plan, proposal) = two_file_plan_and_proposal();
        assert!(scope_patch_proposal_to_files(
            &plan,
            &proposal,
            &[PathBuf::from("src/does_not_exist.rs")]
        )
        .is_err());
    }
}
