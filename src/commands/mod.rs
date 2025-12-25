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

pub fn check_root() -> Result<()> {
    if !nix::unistd::Uid::effective().is_root() {
        bail!("This operation requires root privileges. Run with sudo.");
    }
    Ok(())
}
