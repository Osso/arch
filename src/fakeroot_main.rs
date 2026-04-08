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

// Offsets in struct stat (x86_64)
const STAT_MODE_OFFSET: usize = 24;
const STAT_UID_OFFSET: usize = 28;
const STAT_GID_OFFSET: usize = 32;
const STAT_INO_OFFSET: usize = 8;

// Offsets in struct statx (x86_64)
const STATX_MODE_OFFSET: usize = 18;
const STATX_UID_OFFSET: usize = 20;
const STATX_GID_OFFSET: usize = 24;
const STATX_INO_OFFSET: usize = 80;

/// Tracks whether we're at syscall entry or exit for each process
#[derive(Default)]
struct TracerState {
    in_syscall: HashSet<i32>,
    pending_stat: HashMap<i32, u64>,
    pending_statx: HashMap<i32, u64>,
    /// Pending chmod: pid -> (path_addr, mode)
    pending_chmod: HashMap<i32, (u64, u32)>,
    /// Pending fchmod: pid -> (fd, mode)
    pending_fchmod: HashMap<i32, (i32, u32)>,
    /// Faked modes by inode
    fake_modes: HashMap<u64, u32>,
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

    let result = trace_loop(&mut state, &mut active_pids, options);

    // Clean up any remaining traced processes on error or exit
    for &pid in &active_pids {
        // Detach from traced processes to allow them to continue/exit
        let _ = ptrace::detach(Pid::from_raw(pid), None);
    }

    result
}

fn trace_loop(
    state: &mut TracerState,
    active_pids: &mut HashSet<i32>,
    options: ptrace::Options,
) -> Result<()> {
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
                state.pending_chmod.remove(&pid.as_raw());
                state.pending_fchmod.remove(&pid.as_raw());
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
                // Don't propagate errors - just log and continue tracing
                if let Err(e) = handle_syscall(pid, state) {
                    eprintln!("syscall handling error for {}: {}", pid, e);
                }
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

fn handle_syscall_exit(
    pid: Pid,
    pid_raw: i32,
    syscall: u64,
    regs: &mut nix::libc::user_regs_struct,
    state: &mut TracerState,
) -> Result<bool> {
    let mut modified = false;
    match syscall {
        SYS_GETUID | SYS_GETEUID | SYS_GETGID | SYS_GETEGID => {
            regs.rax = 0;
            modified = true;
        }
        SYS_CHOWN | SYS_FCHOWN | SYS_LCHOWN | SYS_FCHOWNAT => {
            if (regs.rax as i64) < 0 {
                regs.rax = 0;
                modified = true;
            }
        }
        SYS_CHMOD | SYS_FCHMODAT => {
            if let Some((path_addr, mode)) = state.pending_chmod.remove(&pid_raw) {
                if let Some(ino) = get_inode_from_path(pid, path_addr) {
                    state.fake_modes.insert(ino, mode);
                }
            }
            if (regs.rax as i64) < 0 {
                regs.rax = 0;
                modified = true;
            }
        }
        SYS_FCHMOD => {
            if let Some((fd, mode)) = state.pending_fchmod.remove(&pid_raw) {
                if let Some(ino) = get_inode_from_fd(pid, fd) {
                    state.fake_modes.insert(ino, mode);
                }
            }
            if (regs.rax as i64) < 0 {
                regs.rax = 0;
                modified = true;
            }
        }
        SYS_STAT | SYS_FSTAT | SYS_LSTAT | SYS_NEWFSTATAT => {
            if regs.rax == 0 {
                if let Some(buf) = state.pending_stat.remove(&pid_raw) {
                    modify_stat_result(pid, buf, &state.fake_modes, false)?;
                }
            }
        }
        SYS_STATX => {
            if regs.rax == 0 {
                if let Some(buf) = state.pending_statx.remove(&pid_raw) {
                    modify_stat_result(pid, buf, &state.fake_modes, true)?;
                }
            }
        }
        _ => {}
    }
    Ok(modified)
}

fn handle_syscall_entry(
    pid_raw: i32,
    syscall: u64,
    regs: &nix::libc::user_regs_struct,
    state: &mut TracerState,
) {
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
        SYS_CHMOD => {
            state
                .pending_chmod
                .insert(pid_raw, (regs.rdi, regs.rsi as u32));
        }
        SYS_FCHMODAT => {
            state
                .pending_chmod
                .insert(pid_raw, (regs.rsi, regs.rdx as u32));
        }
        SYS_FCHMOD => {
            state
                .pending_fchmod
                .insert(pid_raw, (regs.rdi as i32, regs.rsi as u32));
        }
        _ => {}
    }
}

fn handle_syscall(pid: Pid, state: &mut TracerState) -> Result<()> {
    let regs = match ptrace::getregs(pid) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    let pid_raw = pid.as_raw();
    let syscall = regs.orig_rax;

    if state.in_syscall.remove(&pid_raw) {
        let mut regs = regs;
        let modified = handle_syscall_exit(pid, pid_raw, syscall, &mut regs, state)?;
        if modified {
            let _ = ptrace::setregs(pid, regs);
        }
    } else {
        state.in_syscall.insert(pid_raw);
        handle_syscall_entry(pid_raw, syscall, &regs, state);
    }
    Ok(())
}

/// Modify stat result: fake uid/gid to 0, apply faked mode if tracked
fn modify_stat_result(
    pid: Pid,
    buf: u64,
    fake_modes: &HashMap<u64, u32>,
    is_statx: bool,
) -> Result<()> {
    if buf == 0 {
        return Ok(());
    }

    let (uid_off, gid_off, ino_off, mode_off) = if is_statx {
        (
            STATX_UID_OFFSET,
            STATX_GID_OFFSET,
            STATX_INO_OFFSET,
            STATX_MODE_OFFSET,
        )
    } else {
        (
            STAT_UID_OFFSET,
            STAT_GID_OFFSET,
            STAT_INO_OFFSET,
            STAT_MODE_OFFSET,
        )
    };

    // Fake uid/gid to 0
    write_u32(pid, buf + uid_off as u64, 0)?;
    write_u32(pid, buf + gid_off as u64, 0)?;

    // Check if we have a faked mode for this inode
    if let Some(ino) = read_u64(pid, buf + ino_off as u64) {
        if let Some(&mode) = fake_modes.get(&ino) {
            if is_statx {
                // statx uses u16 for mode
                write_u16(pid, buf + mode_off as u64, mode as u16)?;
            } else {
                // stat uses u32 for mode
                write_u32(pid, buf + mode_off as u64, mode)?;
            }
        }
    }

    Ok(())
}

trait ToNeBytes {
    type Bytes: AsRef<[u8]>;
    fn to_ne_bytes(self) -> Self::Bytes;
}

impl ToNeBytes for u32 {
    type Bytes = [u8; 4];
    fn to_ne_bytes(self) -> [u8; 4] {
        u32::to_ne_bytes(self)
    }
}

impl ToNeBytes for u16 {
    type Bytes = [u8; 2];
    fn to_ne_bytes(self) -> [u8; 2] {
        u16::to_ne_bytes(self)
    }
}

fn write_int<T: ToNeBytes>(pid: Pid, addr: u64, value: T) -> Result<()> {
    let word_addr = addr & !7;
    let offset = (addr & 7) as usize;

    let current =
        ptrace::read(pid, word_addr as *mut libc::c_void).context("ptrace read failed")? as u64;

    let mut bytes = current.to_ne_bytes();
    let src = value.to_ne_bytes();
    let src = src.as_ref();
    bytes[offset..offset + src.len()].copy_from_slice(src);

    ptrace::write(
        pid,
        word_addr as *mut libc::c_void,
        u64::from_ne_bytes(bytes) as c_long,
    )
    .context("ptrace write failed")?;
    Ok(())
}

fn write_u32(pid: Pid, addr: u64, value: u32) -> Result<()> {
    write_int(pid, addr, value)
}

fn write_u16(pid: Pid, addr: u64, value: u16) -> Result<()> {
    write_int(pid, addr, value)
}

fn read_u64(pid: Pid, addr: u64) -> Option<u64> {
    let word_addr = addr & !7;
    let offset = (addr & 7) as usize;

    let word = ptrace::read(pid, word_addr as *mut libc::c_void).ok()? as u64;
    let bytes = word.to_ne_bytes();

    // If aligned, just return. If not, we need to read next word too.
    if offset == 0 {
        Some(word)
    } else {
        let next = ptrace::read(pid, (word_addr + 8) as *mut libc::c_void).ok()? as u64;
        let next_bytes = next.to_ne_bytes();
        let mut result = [0u8; 8];
        result[..8 - offset].copy_from_slice(&bytes[offset..]);
        result[8 - offset..].copy_from_slice(&next_bytes[..offset]);
        Some(u64::from_ne_bytes(result))
    }
}

/// Read a null-terminated string from tracee memory
fn read_string(pid: Pid, mut addr: u64) -> Option<String> {
    let mut bytes = Vec::new();
    loop {
        let word = ptrace::read(pid, addr as *mut libc::c_void).ok()? as u64;
        let word_bytes = word.to_ne_bytes();
        for &b in &word_bytes {
            if b == 0 {
                return String::from_utf8(bytes).ok();
            }
            bytes.push(b);
            if bytes.len() > 4096 {
                return None; // Path too long
            }
        }
        addr += 8;
    }
}

/// Get inode of a file by reading path from tracee and stat'ing from tracer
fn get_inode_from_path(pid: Pid, path_addr: u64) -> Option<u64> {
    let path = read_string(pid, path_addr)?;
    // Read the cwd of the tracee via /proc
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", pid.as_raw())).ok()?;
    let full_path = if path.starts_with('/') {
        std::path::PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let meta = std::fs::metadata(&full_path).ok()?;
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

/// Get inode of a file by fd from tracee
fn get_inode_from_fd(pid: Pid, fd: i32) -> Option<u64> {
    let fd_path = format!("/proc/{}/fd/{}", pid.as_raw(), fd);
    let meta = std::fs::metadata(&fd_path).ok()?;
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}
