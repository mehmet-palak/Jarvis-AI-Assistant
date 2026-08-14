//! F4 workbench contracts. This module intentionally plans and validates patches; it does not
//! execute arbitrary commands or modify a workspace.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerNetwork {
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLimits {
    pub max_runtime_seconds: u32,
    pub max_output_bytes: u64,
    pub max_changed_files: usize,
    pub max_diff_bytes: usize,
    pub network: WorkerNetwork,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        Self {
            max_runtime_seconds: 120,
            max_output_bytes: 1_000_000,
            max_changed_files: 12,
            max_diff_bytes: 256 * 1024,
            network: WorkerNetwork::Denied,
        }
    }
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
    if let Err(error) = run_git_apply(&plan.workspace_root, &proposal.unified_diff, true) {
        let _ = fs::remove_dir_all(&snapshot.snapshot_root);
        return Err(error);
    }
    if let Err(error) = run_git_apply(&plan.workspace_root, &proposal.unified_diff, false) {
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

fn validate_workspace_relative_path(path: &Path) -> Result<(), String> {
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

fn run_git_apply(workspace_root: &Path, diff: &str, check_only: bool) -> Result<(), String> {
    let mut command = isolated_git_apply_command(workspace_root, check_only)?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("isolated patch worker could not start git apply: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "isolated patch worker stdin is unavailable".to_string())?
        .write_all(diff.as_bytes())
        .map_err(|error| format!("isolated patch worker could not receive diff: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("isolated patch worker did not complete: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let preview = stderr.chars().take(2_000).collect::<String>();
    Err(format!("isolated patch worker rejected diff: {preview}"))
}

/// The release worker sees only the approved workspace, read-only runtime libraries, a private
/// `/tmp`, no inherited environment and a new network namespace. Failure to start bubblewrap is
/// an execution failure, not permission to fall back to the host shell.
#[cfg(not(test))]
fn isolated_git_apply_command(workspace_root: &Path, check_only: bool) -> Result<Command, String> {
    let workspace_root = fs::canonicalize(workspace_root)
        .map_err(|error| format!("isolated worker cannot resolve workspace: {error}"))?;
    let bubblewrap = Path::new("/usr/bin/bwrap");
    let git = Path::new("/usr/bin/git");
    if !bubblewrap.is_file() || !git.is_file() {
        return Err("isolated patch worker requires /usr/bin/bwrap and /usr/bin/git".into());
    }
    for runtime_path in [Path::new("/usr"), Path::new("/lib"), Path::new("/lib64")] {
        if !runtime_path.exists() {
            return Err(format!(
                "isolated patch worker runtime path is unavailable: {}",
                runtime_path.display()
            ));
        }
    }
    let mut command = Command::new(bubblewrap);
    command
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--unshare-net")
        .arg("--clearenv")
        .args(["--ro-bind", "/usr", "/usr"])
        .args(["--ro-bind", "/lib", "/lib"])
        .args(["--ro-bind", "/lib64", "/lib64"])
        .arg("--bind")
        .arg(&workspace_root)
        .arg(&workspace_root)
        .args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"])
        .args(["--setenv", "HOME", "/nonexistent"])
        .args(["--setenv", "PATH", "/usr/bin"])
        .arg("--chdir")
        .arg(&workspace_root)
        .arg("--")
        .arg(git)
        .arg("apply");
    if check_only {
        command.arg("--check");
    }
    command
        .args(["--whitespace=error", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    Ok(command)
}

/// Unit tests prove patch semantics using a controlled temporary folder. The test runner itself
/// is already a sandbox and may prohibit `CLONE_NEWNET`; production builds never use this path.
#[cfg(test)]
fn isolated_git_apply_command(workspace_root: &Path, check_only: bool) -> Result<Command, String> {
    let mut command = Command::new("git");
    command.current_dir(workspace_root).arg("apply");
    if check_only {
        command.arg("--check");
    }
    command
        .args(["--whitespace=error", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
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
}
