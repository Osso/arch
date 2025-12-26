pub(crate) mod fakeroot;
mod runner;
mod sandbox;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub fn build_package(source_dir: PathBuf, destdir: &Path) -> Result<PathBuf> {
    let pkgbuild_path = source_dir.join("PKGBUILD");
    if !pkgbuild_path.exists() {
        bail!("No PKGBUILD found in {}", source_dir.display());
    }

    runner::build_in_sandbox(&source_dir, destdir)?;

    // Find the created package
    find_package(destdir)
}

/// Find the .pkg.tar.zst file in destdir
fn find_package(destdir: &Path) -> Result<PathBuf> {
    for entry in std::fs::read_dir(destdir).context("Failed to read destdir")? {
        let entry = entry?;
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".pkg.tar.zst") {
                return Ok(path);
            }
        }
    }
    bail!("No .pkg.tar.zst found in {}", destdir.display())
}
