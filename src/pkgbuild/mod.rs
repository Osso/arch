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

    let staging_dir = tempfile::tempdir().context("Failed to create package staging directory")?;
    runner::build_in_sandbox(&source_dir, staging_dir.path())?;

    let staged_package = find_package(staging_dir.path())?;
    let package_name = staged_package
        .file_name()
        .context("Built package has no filename")?;
    let output_path = destdir.join(package_name);
    std::fs::copy(&staged_package, &output_path)
        .with_context(|| format!("Failed to copy package to {}", output_path.display()))?;

    Ok(output_path)
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
