pub mod build;
pub mod depends;
pub mod files;
pub mod info;
pub mod install;
pub mod list;
pub mod remove;
pub mod search;
pub mod verify;

use anyhow::{bail, Result};
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Ensure we're running as root. If not, attempt to re-exec via authsudo.
/// Returns Ok(()) if already root, otherwise attempts escalation or bails.
pub fn ensure_root() -> Result<()> {
    if nix::unistd::Uid::effective().is_root() {
        return Ok(());
    }

    // Check if authsudo is available
    if which("authsudo").is_none() {
        bail!("This operation requires root privileges. Install authsudo or run with sudo.");
    }

    // Re-exec through authsudo
    let args: Vec<_> = std::env::args().collect();
    let err = Command::new("authsudo").args(&args).exec();

    // exec() only returns on error
    bail!("Failed to exec authsudo: {}", err)
}

fn which(binary: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let path = dir.join(binary);
            if path.is_file() {
                Some(path)
            } else {
                None
            }
        })
    })
}
