//! ptrace-based fakeroot implementation
//!
//! Intercepts syscalls to simulate root ownership without LD_PRELOAD.
//! Traces all child processes and modifies:
//! - getuid/geteuid/getgid/getegid → return 0
//! - chown/fchown/lchown/fchownat → return 0 (success)
//! - stat/fstat/lstat/newfstatat → modify uid/gid in result to 0

use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{BufRead, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use nix::libc::{self, c_long};
use nix::sys::ptrace;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{fork, ForkResult, Pid};

// x86_64 syscall numbers
const SYS_STAT: u64 = 4;
const SYS_FSTAT: u64 = 5;
const SYS_LSTAT: u64 = 6;
const SYS_CHMOD: u64 = 90;
const SYS_FCHMOD: u64 = 91;
const SYS_CHOWN: u64 = 92;
const SYS_FCHOWN: u64 = 93;
const SYS_LCHOWN: u64 = 94;
const SYS_GETUID: u64 = 102;
const SYS_GETGID: u64 = 104;
const SYS_GETEUID: u64 = 107;
const SYS_GETEGID: u64 = 108;
const SYS_FCHMODAT: u64 = 268;
const SYS_FCHOWNAT: u64 = 260;
const SYS_NEWFSTATAT: u64 = 262;
const SYS_STATX: u64 = 332;

// Offset of st_uid/st_gid in struct stat (x86_64)
// st_mode is at 24, st_uid at 28, st_gid at 32
const STAT_UID_OFFSET: usize = 28;
const STAT_GID_OFFSET: usize = 32;

// Offset of stx_uid/stx_gid in struct statx (x86_64)
// stx_uid at 20, stx_gid at 24
const STATX_UID_OFFSET: usize = 20;
const STATX_GID_OFFSET: usize = 24;

/// Environment variable that signals we're in fakeroot mode
const FAKEROOT_ENV: &str = "__ARCH_FAKEROOT";

/// Tracks whether we're at syscall entry or exit for each process
#[derive(Default)]
struct TracerState {
    /// PIDs currently at syscall entry (waiting for exit)
    in_syscall: HashSet<i32>,
    /// Pending stat buffer addresses to modify on exit
    pending_stat: HashMap<i32, u64>,
    /// Pending statx buffer addresses to modify on exit
    pending_statx: HashMap<i32, u64>,
}

/// Check if we should run as the fakeroot tracer (called from inside sandbox)
pub fn maybe_run_as_tracer() -> bool {
    if env::var(FAKEROOT_ENV).is_ok() {
        if let Err(e) = run_tracer_mode() {
            eprintln!("fakeroot tracer error: {}", e);
            std::process::exit(1);
        }
        true
    } else {
        false
    }
}

/// Tracer mode: fork, trace child, child runs bash script from stdin
fn run_tracer_mode() -> Result<()> {
    // Read script from stdin
    let stdin = std::io::stdin();
    let mut script = String::new();
    for line in stdin.lock().lines() {
        script.push_str(&line?);
        script.push('\n');
    }

    match unsafe { fork() }.context("fork failed")? {
        ForkResult::Child => {
            ptrace::traceme().context("traceme failed")?;

            // Exec bash with the script
            let err = Command::new("bash")
                .arg("-c")
                .arg(&script)
                .exec();
            eprintln!("exec failed: {}", err);
            std::process::exit(1);
        }
        ForkResult::Parent { child } => {
            trace_child(child)?;
        }
    }
    Ok(())
}

/// Run a command inside the sandbox with ptrace-based fakeroot.
/// This spawns the command, which should invoke this same binary as a tracer.
pub fn run_sandboxed_with_fakeroot(mut bwrap_cmd: Command, script: &str) -> Result<()> {
    // Get path to our own binary
    let self_exe = env::current_exe().context("failed to get current exe")?;

    // Modify bwrap to run our binary as tracer instead of bash
    // We'll pass the script via stdin
    bwrap_cmd.args(["--", self_exe.to_str().unwrap()]);
    bwrap_cmd.env(FAKEROOT_ENV, "1");
    bwrap_cmd.stdin(Stdio::piped());

    let mut child = bwrap_cmd.spawn().context("failed to spawn bwrap")?;

    // Send script to tracer via stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(script.as_bytes())?;
    }

    let status = child.wait().context("failed to wait for bwrap")?;
    if !status.success() {
        anyhow::bail!("sandboxed command failed with exit code {:?}", status.code());
    }

    Ok(())
}

/// Main tracing loop
fn trace_child(initial_pid: Pid) -> Result<()> {
    let mut state = TracerState::default();
    let mut active_pids: HashSet<i32> = HashSet::new();
    active_pids.insert(initial_pid.as_raw());

    // Wait for initial exec stop
    waitpid(initial_pid, None).context("initial waitpid failed")?;

    // Set options to trace children
    let options = ptrace::Options::PTRACE_O_TRACESYSGOOD
        | ptrace::Options::PTRACE_O_TRACEFORK
        | ptrace::Options::PTRACE_O_TRACEVFORK
        | ptrace::Options::PTRACE_O_TRACECLONE
        | ptrace::Options::PTRACE_O_TRACEEXEC;

    ptrace::setoptions(initial_pid, options).context("setoptions failed")?;
    ptrace::syscall(initial_pid, None).context("initial syscall failed")?;

    loop {
        // Wait for any traced process
        let status = match waitpid(None, Some(WaitPidFlag::__WALL)) {
            Ok(s) => s,
            Err(nix::errno::Errno::ECHILD) => break,
            Err(e) => return Err(e).context("waitpid failed"),
        };

        match status {
            WaitStatus::Exited(pid, _code) => {
                active_pids.remove(&pid.as_raw());
                state.in_syscall.remove(&pid.as_raw());
                state.pending_stat.remove(&pid.as_raw());
                state.pending_statx.remove(&pid.as_raw());
                if active_pids.is_empty() {
                    break;
                }
            }
            WaitStatus::Signaled(pid, _signal, _) => {
                active_pids.remove(&pid.as_raw());
                if active_pids.is_empty() {
                    break;
                }
            }
            WaitStatus::PtraceEvent(pid, _signal, _event) => {
                if let Ok(new_pid) = ptrace::getevent(pid) {
                    let new_pid = Pid::from_raw(new_pid as i32);
                    active_pids.insert(new_pid.as_raw());
                }
                let _ = ptrace::syscall(pid, None);
            }
            WaitStatus::PtraceSyscall(pid) => {
                handle_syscall(pid, &mut state)?;
                let _ = ptrace::syscall(pid, None);
            }
            WaitStatus::Stopped(pid, signal) => {
                let sig = if signal == Signal::SIGTRAP {
                    None
                } else {
                    Some(signal)
                };
                let _ = ptrace::setoptions(pid, options);
                active_pids.insert(pid.as_raw());
                let _ = ptrace::syscall(pid, sig);
            }
            _ => {}
        }
    }

    Ok(())
}

/// Handle a syscall entry/exit
fn handle_syscall(pid: Pid, state: &mut TracerState) -> Result<()> {
    let regs = match ptrace::getregs(pid) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    let pid_raw = pid.as_raw();
    let syscall = regs.orig_rax;

    if state.in_syscall.contains(&pid_raw) {
        state.in_syscall.remove(&pid_raw);

        let mut regs = regs;
        let mut modified = false;

        match syscall {
            SYS_GETUID | SYS_GETEUID | SYS_GETGID | SYS_GETEGID => {
                regs.rax = 0;
                modified = true;
            }
            SYS_CHOWN | SYS_FCHOWN | SYS_LCHOWN | SYS_FCHOWNAT
            | SYS_CHMOD | SYS_FCHMOD | SYS_FCHMODAT => {
                // Make ownership/permission changes appear to succeed
                if regs.rax as i64 == -1 || (regs.rax as i64) < 0 {
                    regs.rax = 0;
                    modified = true;
                }
            }
            SYS_STAT | SYS_FSTAT | SYS_LSTAT | SYS_NEWFSTATAT => {
                if regs.rax == 0 {
                    if let Some(buf_addr) = state.pending_stat.remove(&pid_raw) {
                        modify_stat_buffer(pid, buf_addr)?;
                    }
                }
            }
            SYS_STATX => {
                if regs.rax == 0 {
                    if let Some(buf_addr) = state.pending_statx.remove(&pid_raw) {
                        modify_statx_buffer(pid, buf_addr)?;
                    }
                }
            }
            _ => {}
        }

        if modified {
            let _ = ptrace::setregs(pid, regs);
        }
    } else {
        state.in_syscall.insert(pid_raw);

        match syscall {
            SYS_STAT | SYS_LSTAT => {
                state.pending_stat.insert(pid_raw, regs.rsi);
            }
            SYS_FSTAT => {
                state.pending_stat.insert(pid_raw, regs.rsi);
            }
            SYS_NEWFSTATAT => {
                state.pending_stat.insert(pid_raw, regs.rdx);
            }
            SYS_STATX => {
                // statx(dirfd, path, flags, mask, buf) - buf is in r8
                state.pending_statx.insert(pid_raw, regs.r8);
            }
            _ => {}
        }
    }

    Ok(())
}

/// Modify uid/gid in a stat buffer to 0 (root)
fn modify_stat_buffer(pid: Pid, buf_addr: u64) -> Result<()> {
    if buf_addr == 0 {
        return Ok(());
    }

    let uid_addr = buf_addr + STAT_UID_OFFSET as u64;
    let gid_addr = buf_addr + STAT_GID_OFFSET as u64;

    write_u32(pid, uid_addr, 0)?;
    write_u32(pid, gid_addr, 0)?;

    Ok(())
}

/// Modify uid/gid in a statx buffer to 0 (root)
fn modify_statx_buffer(pid: Pid, buf_addr: u64) -> Result<()> {
    if buf_addr == 0 {
        return Ok(());
    }

    let uid_addr = buf_addr + STATX_UID_OFFSET as u64;
    let gid_addr = buf_addr + STATX_GID_OFFSET as u64;

    write_u32(pid, uid_addr, 0)?;
    write_u32(pid, gid_addr, 0)?;

    Ok(())
}

/// Write a u32 value to tracee memory
fn write_u32(pid: Pid, addr: u64, value: u32) -> Result<()> {
    let word_addr = addr & !7;
    let offset = (addr & 7) as usize;

    let current = ptrace::read(pid, word_addr as *mut libc::c_void)
        .context("ptrace read failed")? as u64;

    let mut bytes = current.to_ne_bytes();
    let value_bytes = value.to_ne_bytes();
    bytes[offset..offset + 4].copy_from_slice(&value_bytes);
    let new_word = u64::from_ne_bytes(bytes);

    ptrace::write(
        pid,
        word_addr as *mut libc::c_void,
        new_word as c_long,
    )
    .context("ptrace write failed")?;

    Ok(())
}
