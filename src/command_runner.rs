//! F4 "Allowlist command runner" + "Test/verifier runner". Runs exactly one command from a small
//! fixed program/subcommand allowlist inside the same isolation `git apply` already uses (bwrap +
//! rlimits + wall-clock watchdog + cooperative cancel), and never through a shell: the command
//! line is split into a plain argv and executed directly, so there is no shell-metacharacter
//! injection surface to reason about at all — the same "reject, don't interpret" posture the
//! router already uses for model output.
//!
//! This module intentionally does not decide *which* command to run — that is `CodingPlan`'s
//! `test_plan` (from `RepoOverview.suggested_test_commands` or a user's own request). It only
//! decides whether a given command line is safe to execute and, if so, executes it under the same
//! guarantees as every other F4 worker: no host shell fallback, no network, no ambient `PATH`
//! lookup inside the sandbox itself (the program is resolved to an absolute path on the host
//! first, then bound in explicitly).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::workbench::{
    apply_worker_rlimits, isolated_worker_command, validate_workspace_relative_path, CancelFlag,
    WorkerLimits, WorkerStopReason,
};

/// (program, allowed first subcommand set). An empty subcommand set means "any single token
/// following the program name is accepted" — still bounded by `validate_command_line`'s
/// metacharacter/length/traversal checks, just not further restricted per-subcommand (used for
/// `pytest`, which is normally invoked with no subcommand at all).
const ALLOWED_PROGRAMS: &[(&str, &[&str])] = &[
    ("cargo", &["test", "build", "check", "clippy", "fmt"]),
    ("npm", &["test", "run", "ci"]),
    ("pnpm", &["test", "run"]),
    ("yarn", &["test", "run"]),
    ("pytest", &[]),
    ("python3", &["-m"]),
    ("python", &["-m"]),
    ("go", &["test", "build", "vet"]),
    ("mvn", &["test", "verify"]),
    ("gradle", &["test", "check"]),
];

const MAX_COMMAND_TOKENS: usize = 8;
const FORBIDDEN_CHARACTERS: &[char] = &[
    ';', '|', '&', '$', '`', '\n', '\r', '<', '>', '(', ')', '{', '}', '*', '?', '~', '#', '\\',
    '"', '\'', '=',
];

/// One completed (or dry-run/validated) command execution — the evidence F4's "Test/verifier
/// runner" item asks for: exit code, bounded output previews, and full-output hashes so a caller
/// can prove what actually ran without storing unbounded logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRun {
    pub program: String,
    pub args: Vec<String>,
    pub dry_run: bool,
    pub exit_code: Option<i32>,
    pub stdout_preview: String,
    pub stderr_preview: String,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub stopped: Option<WorkerStopReason>,
}

impl CommandRun {
    /// `false` for a dry run (nothing executed, so nothing can have "succeeded" yet), for a
    /// worker that was cancelled/timed out, or for a nonzero/unknown exit code.
    pub fn succeeded(&self) -> bool {
        !self.dry_run && self.stopped.is_none() && self.exit_code == Some(0)
    }

    pub fn command_line(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }
}

/// Parses a plain-text command line into a validated argv. Never invokes a shell: any character
/// a shell would treat specially is rejected outright rather than interpreted or escaped — there
/// is nothing here that could ever be "smart" about quoting because nothing here ever parses
/// quoting at all.
pub fn validate_command_line(command_line: &str) -> Result<Vec<String>, String> {
    let trimmed = command_line.trim();
    if trimmed.is_empty() {
        return Err("command is empty".into());
    }
    if trimmed
        .chars()
        .any(|character| FORBIDDEN_CHARACTERS.contains(&character))
    {
        return Err(
            "command must not contain shell metacharacters; only a plain program name and arguments are allowed"
                .into(),
        );
    }
    let tokens: Vec<String> = trimmed.split_whitespace().map(str::to_owned).collect();
    if tokens.len() > MAX_COMMAND_TOKENS {
        return Err(format!(
            "command exceeds the {MAX_COMMAND_TOKENS}-token limit"
        ));
    }
    let program = tokens[0].as_str();
    let Some((_, allowed_subcommands)) = ALLOWED_PROGRAMS.iter().find(|(name, _)| *name == program)
    else {
        return Err(format!(
            "'{program}' is not on the allowlisted test/build program list"
        ));
    };
    if let Some(first_arg) = tokens.get(1) {
        if !allowed_subcommands.is_empty() && !allowed_subcommands.contains(&first_arg.as_str()) {
            return Err(format!(
                "'{program} {first_arg}' is not an allowlisted subcommand"
            ));
        }
    }
    for token in &tokens[1..] {
        // Flags themselves (`--release`, `-v`, `-n`) are plain argv entries here, never shell-
        // expanded, so they are not a distinct danger — what actually matters is that no token
        // can point outside the workspace. `=` is already in `FORBIDDEN_CHARACTERS`, so a flag
        // cannot smuggle a second path inside itself (`--manifest-path=/etc/passwd` is rejected
        // above at the whole-line check, before tokens are even split).
        if token.starts_with('/') || token.contains("..") {
            return Err("command arguments must not be absolute paths or contain '..'".into());
        }
    }
    Ok(tokens)
}

/// Resolves an allowlisted program name to a real, executable, absolute path via the *host's*
/// `PATH` — the isolated worker never gets a working `PATH`-based lookup itself (F4 threat model:
/// no ambient shell inside the sandbox), so this lookup happens once, here, before the sandbox is
/// even built, and its result is bound in explicitly.
fn resolve_program(program: &str) -> Result<PathBuf, String> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for directory in path_var.split(':') {
        if directory.is_empty() {
            continue;
        }
        let candidate = Path::new(directory).join(program);
        if let Ok(metadata) = fs::metadata(&candidate) {
            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "allowlisted program '{program}' was not found on PATH"
    ))
}

/// A resolved program outside `/usr`, `/lib`, `/lib64` (e.g. a rustup-style `~/.cargo/bin/cargo`)
/// needs its own read-only bind, or the sandbox simply cannot see it. Production-only: the test
/// build's `isolated_worker_command` runs the program directly with no sandbox at all.
#[cfg(not(test))]
fn extra_read_only_binds(resolved_program: &Path) -> Vec<PathBuf> {
    let known_roots = [Path::new("/usr"), Path::new("/lib"), Path::new("/lib64")];
    if known_roots
        .iter()
        .any(|root| resolved_program.starts_with(root))
    {
        return Vec::new();
    }
    resolved_program
        .parent()
        .map(|parent| vec![parent.to_path_buf()])
        .unwrap_or_default()
}
#[cfg(test)]
fn extra_read_only_binds(_resolved_program: &Path) -> Vec<PathBuf> {
    Vec::new()
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn bounded_preview(bytes: &[u8], max_output_bytes: u64) -> String {
    let max_chars = (max_output_bytes as usize).clamp(200, 4_000);
    String::from_utf8_lossy(bytes)
        .chars()
        .take(max_chars)
        .collect()
}

/// Runs one allowlisted command scoped to `workspace_root` (optionally `subdir` beneath it — must
/// itself be workspace-relative, no traversal). `dry_run: true` validates everything (allowlist
/// membership, program resolution, cwd scope) and returns without ever spawning a process — this
/// is what lets a caller show the user exactly what would run before committing to running it.
pub fn run_allowlisted_command(
    workspace_root: &Path,
    subdir: Option<&Path>,
    command_line: &str,
    limits: &WorkerLimits,
    dry_run: bool,
    cancel: Option<&CancelFlag>,
) -> Result<CommandRun, String> {
    let tokens = validate_command_line(command_line)?;
    let program_name = tokens[0].clone();
    let args: Vec<String> = tokens[1..].to_vec();
    let resolved_program = resolve_program(&program_name)?;
    if let Some(relative) = subdir {
        validate_workspace_relative_path(relative)?;
    }
    if dry_run {
        return Ok(CommandRun {
            program: program_name,
            args,
            dry_run: true,
            exit_code: None,
            stdout_preview: "(dry-run: not executed)".into(),
            stderr_preview: String::new(),
            stdout_sha256: hash_bytes(&[]),
            stderr_sha256: hash_bytes(&[]),
            stopped: None,
        });
    }
    let extra_binds = extra_read_only_binds(&resolved_program);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = isolated_worker_command(
        workspace_root,
        subdir,
        &extra_binds,
        &resolved_program,
        &arg_refs,
        limits,
    )?;
    apply_worker_rlimits(&mut command, limits);
    let mut child = command
        .spawn()
        .map_err(|error| format!("allowlisted worker could not start: {error}"))?;
    drop(child.stdin.take());
    let wait_result = crate::workbench::wait_with_deadline_and_cancellation(
        &mut child,
        limits.max_runtime_seconds,
        cancel,
    );
    let stopped = wait_result.as_ref().err().map(|(reason, _)| *reason);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("allowlisted worker did not complete: {error}"))?;
    Ok(CommandRun {
        program: program_name,
        args,
        dry_run: false,
        exit_code: output.status.code(),
        stdout_preview: bounded_preview(&output.stdout, limits.max_output_bytes),
        stderr_preview: bounded_preview(&output.stderr, limits.max_output_bytes),
        stdout_sha256: hash_bytes(&output.stdout),
        stderr_sha256: hash_bytes(&output.stderr),
        stopped,
    })
}

/// F4 "Test/verifier runner": runs every command in a `CodingPlan.test_plan`, stopping at the
/// first one that is not itself allowlisted-runnable (a plan can legitimately contain free-text
/// notes like project_analyst's "no known manifest" fallback, not just executable commands — this
/// only ever executes lines that pass `validate_command_line`, everything else is reported back
/// as skipped rather than silently ignored or treated as a failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRunReport {
    pub ran: Vec<CommandRun>,
    pub skipped: Vec<String>,
}

impl TestRunReport {
    pub fn all_ran_passed(&self) -> bool {
        !self.ran.is_empty() && self.ran.iter().all(CommandRun::succeeded)
    }
}

pub fn run_test_plan(
    workspace_root: &Path,
    test_plan: &[String],
    limits: &WorkerLimits,
    cancel: Option<&CancelFlag>,
) -> TestRunReport {
    let mut report = TestRunReport {
        ran: Vec::new(),
        skipped: Vec::new(),
    };
    for command_line in test_plan {
        if validate_command_line(command_line).is_err() {
            report.skipped.push(command_line.clone());
            continue;
        }
        match run_allowlisted_command(workspace_root, None, command_line, limits, false, cancel) {
            Ok(run) => report.ran.push(run),
            Err(error) => {
                report.skipped.push(format!("{command_line} ({error})"));
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "jarvis-command-runner-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("fixture workspace");
        root
    }

    #[test]
    fn only_allowlisted_programs_and_subcommands_pass_validation() {
        assert!(validate_command_line("cargo test").is_ok());
        assert!(validate_command_line("cargo build").is_ok());
        assert!(validate_command_line("pytest").is_ok());
        assert!(validate_command_line("rm -rf /").is_err());
        assert!(validate_command_line("cargo publish").is_err());
        assert!(validate_command_line("bash -c 'echo hi'").is_err());
    }

    #[test]
    fn shell_metacharacters_are_rejected_outright_not_interpreted() {
        for dangerous in [
            "cargo test; rm -rf /",
            "cargo test && curl evil.example",
            "cargo test | tee /etc/passwd",
            "cargo test `whoami`",
            "cargo test $(whoami)",
        ] {
            assert!(
                validate_command_line(dangerous).is_err(),
                "must reject: {dangerous}"
            );
        }
    }

    #[test]
    fn path_traversal_and_flag_smuggling_in_arguments_are_rejected() {
        assert!(validate_command_line("cargo test ../../etc/passwd").is_err());
        assert!(validate_command_line("cargo test --manifest-path=/etc/passwd").is_err());
    }

    #[test]
    fn a_dry_run_validates_without_ever_spawning_a_process() {
        let root = fixture_workspace("dry-run");
        let limits = WorkerLimits::default();
        let run = run_allowlisted_command(&root, None, "cargo test", &limits, true, None)
            .expect("dry run of an allowlisted command must succeed");
        assert!(run.dry_run);
        assert!(!run.succeeded());
        assert_eq!(run.exit_code, None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_real_allowlisted_command_actually_runs_and_captures_evidence() {
        // `python3 -m` alone is a deliberately-minimal real allowlisted invocation: it exits
        // nonzero (no module named), which is exactly what proves this executed for real rather
        // than being a stub that always reports success.
        let root = fixture_workspace("real-run");
        let limits = WorkerLimits::default();
        let run = run_allowlisted_command(&root, None, "python3 -m", &limits, false, None)
            .expect("an allowlisted command must be executable in the test harness");
        assert!(!run.dry_run);
        assert!(run.exit_code.is_some());
        assert!(!run.succeeded(), "python3 -m alone must exit nonzero");
        assert_ne!(run.stderr_sha256, hash_bytes(&[]));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_non_allowlisted_command_never_reaches_process_spawn() {
        let root = fixture_workspace("blocked");
        let limits = WorkerLimits::default();
        assert!(run_allowlisted_command(
            &root,
            None,
            "curl http://evil.example",
            &limits,
            false,
            None
        )
        .is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_plan_execution_skips_non_command_free_text_lines_instead_of_failing() {
        let root = fixture_workspace("test-plan");
        let limits = WorkerLimits::default();
        let plan = vec![
            "python3 -m".to_string(),
            "no known manifest detected; inspect the repository manually".to_string(),
        ];
        let report = run_test_plan(&root, &plan, &limits, None);
        assert_eq!(report.ran.len(), 1);
        assert_eq!(report.skipped.len(), 1);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_cancelled_test_run_is_reported_as_stopped_not_as_a_plain_failure() {
        let root = fixture_workspace("cancel");
        let limits = WorkerLimits {
            max_runtime_seconds: 30,
            ..WorkerLimits::default()
        };
        let cancel = crate::workbench::new_cancel_flag();
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            cancel_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        // A real, allowlisted, intentionally-slow invocation (`python3 -m timeit`, run a huge
        // loop count) — long enough that "cancelled before it would ever finish on its own" is
        // unambiguous, unlike a command that might race to a natural exit first.
        let run = run_allowlisted_command(
            &root,
            None,
            "python3 -m timeit -n 999999999 pass",
            &limits,
            false,
            Some(&cancel),
        )
        .expect("command must be spawned");
        assert_eq!(run.stopped, Some(WorkerStopReason::UserCancelled));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "cancellation must not wait anywhere near the 30s quota or the loop's natural end"
        );
        fs::remove_dir_all(&root).ok();
    }
}
