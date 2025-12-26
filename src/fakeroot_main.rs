//! Fakeroot tracer binary - runs inside sandbox, traces child processes
//!
//! Reads build script from stdin, forks, traces the child with ptrace
//! to intercept ownership/permission syscalls.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::os::unix::process::CommandExt;
use std::process::Command;

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
const STAT_UID_OFFSET: usize = 28;
const STAT_GID_OFFSET: usize = 32;

// Offset of stx_uid/stx_gid in struct statx (x86_64)
const STATX_UID_OFFSET: usize = 20;
const STATX_GID_OFFSET: usize = 24;

/// Tracks whether we're at syscall entry or exit for each process
#[derive(Default)]
struct TracerState {
    in_syscall: HashSet<i32>,
    pending_stat: HashMap<i32, u64>,
    pending_statx: HashMap<i32, u64>,
}

fn main() -> Result<()> {
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
            let err = Command::new("bash").arg("-c").arg(&script).exec();
            eprintln!("exec failed: {}", err);
            std::process::exit(1);
        }
        ForkResult::Parent { child } => {
            trace_child(child)?;
        }
    }
    Ok(())
}

fn trace_child(initial_pid: Pid) -> Result<()> {
    let mut state = TracerState::default();
    let mut active_pids: HashSet<i32> = HashSet::new();
    active_pids.insert(initial_pid.as_raw());

    waitpid(initial_pid, None).context("initial waitpid failed")?;

    let options = ptrace::Options::PTRACE_O_TRACESYSGOOD
        | ptrace::Options::PTRACE_O_TRACEFORK
        | ptrace::Options::PTRACE_O_TRACEVFORK
        | ptrace::Options::PTRACE_O_TRACECLONE
        | ptrace::Options::PTRACE_O_TRACEEXEC;

    ptrace::setoptions(initial_pid, options).context("setoptions failed")?;
    ptrace::syscall(initial_pid, None).context("initial syscall failed")?;

    loop {
        let status = match waitpid(None, Some(WaitPidFlag::__WALL)) {
            Ok(s) => s,
            Err(nix::errno::Errno::ECHILD) => break,
            Err(e) => return Err(e).context("waitpid failed"),
        };

        match status {
            WaitStatus::Exited(pid, _) | WaitStatus::Signaled(pid, _, _) => {
                active_pids.remove(&pid.as_raw());
                state.in_syscall.remove(&pid.as_raw());
                state.pending_stat.remove(&pid.as_raw());
                state.pending_statx.remove(&pid.as_raw());
                if active_pids.is_empty() {
                    break;
                }
            }
            WaitStatus::PtraceEvent(pid, _, _) => {
                if let Ok(new_pid) = ptrace::getevent(pid) {
                    active_pids.insert(new_pid as i32);
                }
                let _ = ptrace::syscall(pid, None);
            }
            WaitStatus::PtraceSyscall(pid) => {
                handle_syscall(pid, &mut state)?;
                let _ = ptrace::syscall(pid, None);
            }
            WaitStatus::Stopped(pid, signal) => {
                let sig = if signal == Signal::SIGTRAP { None } else { Some(signal) };
                let _ = ptrace::setoptions(pid, options);
                active_pids.insert(pid.as_raw());
                let _ = ptrace::syscall(pid, sig);
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_syscall(pid: Pid, state: &mut TracerState) -> Result<()> {
    let regs = match ptrace::getregs(pid) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    let pid_raw = pid.as_raw();
    let syscall = regs.orig_rax;

    if state.in_syscall.remove(&pid_raw) {
        // Syscall exit
        let mut regs = regs;
        let mut modified = false;

        match syscall {
            SYS_GETUID | SYS_GETEUID | SYS_GETGID | SYS_GETEGID => {
                regs.rax = 0;
                modified = true;
            }
            SYS_CHOWN | SYS_FCHOWN | SYS_LCHOWN | SYS_FCHOWNAT
            | SYS_CHMOD | SYS_FCHMOD | SYS_FCHMODAT => {
                if (regs.rax as i64) < 0 {
                    regs.rax = 0;
                    modified = true;
                }
            }
            SYS_STAT | SYS_FSTAT | SYS_LSTAT | SYS_NEWFSTATAT => {
                if regs.rax == 0 {
                    if let Some(buf) = state.pending_stat.remove(&pid_raw) {
                        modify_stat_buffer(pid, buf, STAT_UID_OFFSET, STAT_GID_OFFSET)?;
                    }
                }
            }
            SYS_STATX => {
                if regs.rax == 0 {
                    if let Some(buf) = state.pending_statx.remove(&pid_raw) {
                        modify_stat_buffer(pid, buf, STATX_UID_OFFSET, STATX_GID_OFFSET)?;
                    }
                }
            }
            _ => {}
        }

        if modified {
            let _ = ptrace::setregs(pid, regs);
        }
    } else {
        // Syscall entry
        state.in_syscall.insert(pid_raw);

        match syscall {
            SYS_STAT | SYS_LSTAT | SYS_FSTAT => {
                state.pending_stat.insert(pid_raw, regs.rsi);
            }
            SYS_NEWFSTATAT => {
                state.pending_stat.insert(pid_raw, regs.rdx);
            }
            SYS_STATX => {
                state.pending_statx.insert(pid_raw, regs.r8);
            }
            _ => {}
        }
    }
    Ok(())
}

fn modify_stat_buffer(pid: Pid, buf: u64, uid_off: usize, gid_off: usize) -> Result<()> {
    if buf == 0 {
        return Ok(());
    }
    write_u32(pid, buf + uid_off as u64, 0)?;
    write_u32(pid, buf + gid_off as u64, 0)?;
    Ok(())
}

fn write_u32(pid: Pid, addr: u64, value: u32) -> Result<()> {
    let word_addr = addr & !7;
    let offset = (addr & 7) as usize;

    let current = ptrace::read(pid, word_addr as *mut libc::c_void)
        .context("ptrace read failed")? as u64;

    let mut bytes = current.to_ne_bytes();
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());

    ptrace::write(pid, word_addr as *mut libc::c_void, u64::from_ne_bytes(bytes) as c_long)
        .context("ptrace write failed")?;
    Ok(())
}
