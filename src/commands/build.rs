use crate::{alpm_handle, callbacks};
use alpm::{SigLevel, TransFlag};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

pub fn run(directory: Option<PathBuf>) -> Result<()> {
    super::check_root()?;

    let dir = directory.unwrap_or_else(|| PathBuf::from("."));

    // Verify PKGBUILD exists
    let pkgbuild = dir.join("PKGBUILD");
    if !pkgbuild.exists() {
        bail!("No PKGBUILD found in {}", dir.display());
    }

    // Run makepkg -f
    println!(":: Building package...");
    let status = Command::new("makepkg")
        .arg("-f")
        .current_dir(&dir)
        .status()
        .context("Failed to run makepkg")?;

    if !status.success() {
        bail!("makepkg failed with exit code {}", status.code().unwrap_or(-1));
    }

    // Find the built package
    let pkg_file = find_package_file(&dir)?;
    println!(":: Installing {}...", pkg_file.display());

    // Initialize alpm handle
    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Load package from file
    let pkg_path = pkg_file.to_string_lossy();
    let pkg = handle
        .pkg_load(pkg_path.as_ref(), true, SigLevel::USE_DEFAULT)
        .context("Failed to load package file")?;

    println!("  {} {}", pkg.name(), pkg.version());

    // Initialize transaction
    handle
        .trans_init(TransFlag::NONE)
        .context("Failed to initialize transaction")?;

    // Add package to transaction
    let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(|e| format!("{:?}", e));
    if let Some(err) = add_err {
        let _ = handle.trans_release();
        bail!("Failed to add package: {}", err);
    }

    // Prepare transaction
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));
    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    // Commit transaction
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));
    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }

    handle.trans_release().ok();
    println!("Done!");
    Ok(())
}

fn find_package_file(dir: &PathBuf) -> Result<PathBuf> {
    let entries = std::fs::read_dir(dir).context("Failed to read directory")?;

    let mut pkg_files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".pkg.tar.zst") || name.ends_with(".pkg.tar.xz")
        })
        .collect();

    if pkg_files.is_empty() {
        bail!("No package file found in {}", dir.display());
    }

    // Sort by modification time, newest first
    pkg_files.sort_by(|a, b| {
        let time_a = a.metadata().and_then(|m| m.modified()).ok();
        let time_b = b.metadata().and_then(|m| m.modified()).ok();
        time_b.cmp(&time_a)
    });

    Ok(pkg_files[0].path())
}

