//! Fakeroot sandbox integration
//!
//! Spawns the arch-fakeroot binary inside a bwrap sandbox to trace
//! syscalls and simulate root ownership.

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Find a sibling binary (same directory as current exe)
fn find_sibling_binary(name: &str) -> Result<PathBuf> {
    let self_exe = env::current_exe().context("failed to get current exe")?;
    let dir = self_exe.parent().context("exe has no parent dir")?;
    let binary = dir.join(name);
    if binary.exists() {
        Ok(binary)
    } else {
        anyhow::bail!("{} binary not found at {}", name, binary.display())
    }
}

/// Run a command inside the sandbox with ptrace-based fakeroot.
/// Spawns bwrap which runs arch-fakeroot as the tracer.
pub fn run_sandboxed_with_fakeroot(mut bwrap_cmd: Command, script: &str) -> Result<()> {
    let fakeroot_exe = find_sibling_binary("arch-fakeroot")?;
    let fakeroot_str = fakeroot_exe.to_string_lossy();

    let makepkg_exe = find_sibling_binary("arch-makepkg")?;
    let makepkg_str = makepkg_exe.to_string_lossy();

    // Bind the helper binaries into the sandbox
    bwrap_cmd.args(["--ro-bind", &fakeroot_str, "/opt/arch/arch-fakeroot"]);
    bwrap_cmd.args(["--ro-bind", &makepkg_str, "/opt/arch/arch-makepkg"]);

    // Run arch-fakeroot inside the sandbox
    bwrap_cmd.args(["--", "/opt/arch/arch-fakeroot"]);
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
