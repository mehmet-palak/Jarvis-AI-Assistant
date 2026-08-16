//! F4 "OS izolasyonu": a real seccomp-bpf allowlist filter for the isolated worker's *final*
//! exec'd program (git, cargo, npm, pytest, python3, go — whatever `isolated_worker_command`
//! ends up running), loaded by `bwrap --seccomp <fd>` right before bwrap execs it. This never
//! touches bwrap's own setup phase: bwrap loads the filter on itself immediately before its own
//! final `execve`, so its prior namespace/mount work (which genuinely needs syscalls this filter
//! denies, like `unshare`/`mount`) is already finished by the time the filter takes effect.
//!
//! **Built empirically, not guessed** (2026-08-16): built from `strace -f -c` traces of real
//! invocations of every allowlisted program that is actually installed on the machine this was
//! developed on — `git apply --check`, `cargo check/fmt --check/clippy`, `npm test` (a real
//! throwaway `package.json`), `pytest` (a real throwaway test file), `python3 -m platform`, and
//! `go build/test/vet` (a real throwaway module) — unioned into one allowlist. `mvn`/`gradle`
//! were not installed anywhere reachable and are **not empirically verified**; the allowlist is
//! generous enough (a fairly complete "ordinary Linux CLI tool" syscall set) that they will very
//! likely work, but this is an honest, documented gap rather than a silent assumption.
//!
//! A deliberate, documented margin was added on top of the raw trace union for syscalls that are
//! virtually universal for any POSIX program but didn't show up in `strace -c`'s summary output
//! for a structural reason, not because they're unneeded: `exit`/`exit_group` never return, so
//! `strace -c` cannot compute a duration for them and drops them from the table even though they
//! were called; `clock_gettime` is usually served by the vDSO (no real syscall trap at all) on
//! this machine, but isn't guaranteed to be everywhere. A handful of other near-universal
//! filesystem/signal syscalls (`symlink`, `readv`/`writev`, `fsync`, `sync`, xattr reads) were
//! added the same way — plausible for fuller real-world runs (a large `cargo test`, a bigger
//! `npm`/`pytest` suite) than the quick smoke invocations actually traced.

use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::Command;

/// Empirically observed (see module docs) union, plus a documented margin of near-universal
/// syscalls. Sorted for readability; order has no effect on the compiled filter.
const ALLOWED_SYSCALLS: &[&str] = &[
    "access",
    "arch_prctl",
    "bind",
    "brk",
    "capget",
    "chdir",
    "chmod",
    "clock_gettime",
    "clock_nanosleep",
    "clone",
    "clone3",
    "close",
    "close_range",
    "copy_file_range",
    "dup2",
    "dup3",
    "epoll_create1",
    "epoll_ctl",
    "epoll_pwait",
    "eventfd2",
    "execve",
    "exit",
    "exit_group",
    "faccessat2",
    "fallocate",
    "fchmodat",
    "fcntl",
    "fdatasync",
    "flock",
    "fstat",
    "fsync",
    "ftruncate",
    "futex",
    "getcwd",
    "getdents64",
    "getegid",
    "geteuid",
    "getgid",
    "getpgrp",
    "getpid",
    "getppid",
    "getrandom",
    "getresgid",
    "getresuid",
    "getsockname",
    "gettid",
    "getuid",
    "getxattr",
    "io_uring_enter",
    "io_uring_setup",
    "ioctl",
    "kill",
    "lgetxattr",
    "linkat",
    "listxattr",
    "lseek",
    "madvise",
    "membarrier",
    "mkdir",
    "mkdirat",
    "mmap",
    "mprotect",
    "mremap",
    "munmap",
    "nanosleep",
    "newfstatat",
    "open",
    "openat",
    "pidfd_open",
    "pidfd_send_signal",
    "pipe2",
    "poll",
    "prctl",
    "pread64",
    "prlimit64",
    "pwrite64",
    "pwritev",
    "read",
    "readlink",
    "readlinkat",
    "readv",
    "recvfrom",
    "recvmsg",
    "rename",
    "renameat",
    "restart_syscall",
    "rmdir",
    "rseq",
    "rt_sigaction",
    "rt_sigprocmask",
    "rt_sigreturn",
    "rt_sigsuspend",
    "sched_getaffinity",
    "sched_getparam",
    "sched_getscheduler",
    "sched_yield",
    "sendto",
    "set_robust_list",
    "set_tid_address",
    "setfsgid",
    "setfsuid",
    "setsid",
    "shutdown",
    "sigaltstack",
    "socket",
    "socketpair",
    "statfs",
    "statx",
    "symlink",
    "symlinkat",
    "sync",
    "tgkill",
    "umask",
    "uname",
    "unlink",
    "unlinkat",
    "utimensat",
    "vfork",
    "wait4",
    "waitid",
    "write",
    "writev",
];

/// Builds the compiled cBPF program bwrap's `--seccomp FD` expects — `ScmpFilterContext::
/// export_bpf_mem` calls the exact same `seccomp_export_bpf` libseccomp function bwrap's own man
/// page names as the required format, so this cannot drift from what bwrap actually parses.
/// Default action is `Errno(EPERM)`: an unlisted syscall fails cleanly (the calling program sees
/// a normal "permission denied"-shaped error) rather than being killed outright — friendlier to
/// diagnose if the allowlist ever needs widening, at no real cost to the security property (the
/// syscall still never executes).
fn build_seccomp_bpf() -> Result<Vec<u8>, String> {
    let mut ctx = ScmpFilterContext::new(ScmpAction::Errno(libc::EPERM))
        .map_err(|error| format!("seccomp filter init failed: {error}"))?;
    for name in ALLOWED_SYSCALLS {
        let syscall = ScmpSyscall::from_name(name)
            .map_err(|error| format!("unknown syscall '{name}': {error}"))?;
        ctx.add_rule(ScmpAction::Allow, syscall)
            .map_err(|error| format!("could not allow '{name}': {error}"))?;
    }
    ctx.export_bpf_mem()
        .map_err(|error| format!("could not export compiled seccomp program: {error}"))
}

/// Writes `bytes` into a fresh `memfd` *without* `MFD_CLOEXEC` — the fd must survive `execve`,
/// since it needs to still be open by the time bwrap (several `fork`+`exec` generations down the
/// `systemd-run` → `bwrap` chain) reads it. Verified live (2026-08-16) that a non-`CLOEXEC` fd
/// really does survive that whole chain unmodified, at the same fd number, all the way through.
fn seccomp_memfd(bytes: &[u8]) -> Result<OwnedFd, String> {
    let name = CString::new("jarvis-seccomp-filter").expect("no NUL bytes in a literal");
    // SAFETY: `memfd_create` is a plain syscall wrapper; `name` is a valid, live `CString`.
    let raw_fd: RawFd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
    if raw_fd < 0 {
        return Err(format!(
            "memfd_create failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: `raw_fd` was just returned by a successful `memfd_create` call above and is not
    // owned anywhere else yet.
    let owned = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let mut file = std::fs::File::from(
        std::os::fd::OwnedFd::try_clone(&owned)
            .map_err(|error| format!("could not duplicate seccomp memfd for writing: {error}"))?,
    );
    use std::io::{Seek, SeekFrom, Write};
    file.write_all(bytes)
        .map_err(|error| format!("could not write seccomp program into memfd: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("could not rewind seccomp memfd: {error}"))?;
    // `file`'s clone is dropped here, closing only that duplicate — `owned`'s underlying fd stays
    // open (the kernel file description is shared, reference-counted; the seek above already
    // repositioned the shared offset back to the start for whichever fd bwrap ends up reading).
    Ok(owned)
}

/// Attaches `--seccomp <fd>` to `command` (a bwrap invocation, as built by `isolated_worker_
/// command`) and returns the `OwnedFd` the caller must keep alive until the command has been
/// spawned — dropping it earlier closes the underlying memfd before bwrap gets a chance to read
/// it. The simplest safe way to guarantee that ordering: move the returned `OwnedFd` into the
/// same `Command`'s `pre_exec` closure (see `isolated_worker_command`) — that closure runs in the
/// forked child right up until `execve` replaces its memory wholesale (never unwinding, so the
/// fd's `Drop` impl never runs there), while in *this* process it naturally gets dropped, and the
/// fd genuinely closed, only once the `Command` itself is dropped after `spawn()`.
pub(crate) fn attach_seccomp_filter(command: &mut Command) -> Result<OwnedFd, String> {
    let bpf = build_seccomp_bpf()?;
    let memfd = seccomp_memfd(&bpf)?;
    command.arg("--seccomp").arg(memfd.as_raw_fd().to_string());
    Ok(memfd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_allowed_syscall_name_is_real_on_this_architecture() {
        // A typo'd or renamed syscall would otherwise only surface as a runtime error deep
        // inside a real worker invocation — catch it here instead, for every entry, every time.
        for name in ALLOWED_SYSCALLS {
            assert!(
                ScmpSyscall::from_name(name).is_ok(),
                "'{name}' is not a recognized syscall name on this architecture"
            );
        }
    }

    #[test]
    fn the_allowlist_has_no_accidental_duplicates() {
        let mut sorted = ALLOWED_SYSCALLS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ALLOWED_SYSCALLS.len(),
            "a duplicate entry wastes nothing functionally, but signals a copy-paste mistake"
        );
    }

    #[test]
    fn build_seccomp_bpf_produces_a_non_empty_compiled_program() {
        let bpf = build_seccomp_bpf().expect("a filter built from real syscall names must compile");
        assert!(!bpf.is_empty());
        // A cBPF program is a sequence of 8-byte `sock_filter` structs.
        assert_eq!(
            bpf.len() % 8,
            0,
            "cBPF programs are always a multiple of 8 bytes"
        );
    }

    #[test]
    fn attach_seccomp_filter_adds_a_seccomp_flag_pointing_at_a_readable_fd() {
        let mut command = Command::new("/bin/true");
        let fd = attach_seccomp_filter(&mut command).expect("filter must build and attach");
        // The fd itself must be readable and contain a well-formed (non-empty, 8-byte-aligned)
        // cBPF program — the same shape `build_seccomp_bpf_produces_...` already checks.
        let mut file = std::fs::File::from(fd);
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes).expect("read back the attached memfd");
        assert!(!bytes.is_empty());
        assert_eq!(bytes.len() % 8, 0);
    }

    #[test]
    fn seccomp_memfd_round_trips_the_exact_bytes_from_the_start() {
        let payload = b"jarvis-seccomp-test-payload";
        let fd = seccomp_memfd(payload).expect("memfd_create must succeed in a normal test run");
        let mut file = std::fs::File::from(fd);
        let mut read_back = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut read_back).expect("read back the memfd");
        assert_eq!(read_back, payload);
    }
}
