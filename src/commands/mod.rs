pub mod build;
pub mod depends;
pub mod files;
pub mod info;
pub mod install;
pub mod list;
pub mod remove;
pub mod search;
pub mod verify;

use anyhow::Result;

/// Ensure we're running as root. If not, attempt to re-exec via authsudo.
pub fn ensure_root() -> Result<()> {
    authd_escalate::ensure_root()?;
    Ok(())
}
