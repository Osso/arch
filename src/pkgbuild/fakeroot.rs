//! Fakeroot sandbox integration
//!
//! Spawns the arch-fakeroot binary inside a bwrap sandbox to trace
//! syscalls and simulate root ownership.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Run a command inside the sandbox with ptrace-based fakeroot.
/// Spawns bwrap which runs arch-fakeroot as the tracer.
/// Requires arch-fakeroot and arch-makepkg to be installed in /usr/bin.
pub fn run_sandboxed_with_fakeroot(mut bwrap_cmd: Command, script: &str) -> Result<()> {
    // Run arch-fakeroot inside the sandbox (available via /usr bind)
    bwrap_cmd.args(["--", "/usr/bin/arch-fakeroot"]);
    bwrap_cmd.stdin(Stdio::piped());

    let mut child = bwrap_cmd.spawn().context("failed to spawn bwrap")?;

    // Send script to tracer via stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(script.as_bytes())?;
    }

    let status = child.wait().context("failed to wait for bwrap")?;
    if !status.success() {
        anyhow::bail!(
            "sandboxed command failed with exit code {:?}",
            status.code()
        );
    }

    Ok(())
}
