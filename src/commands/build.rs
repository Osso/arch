use crate::pkgbuild;
use crate::{alpm_handle, callbacks};
use alpm::{SigLevel, TransFlag};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub fn run(directory: Option<PathBuf>, install: bool) -> Result<()> {
    let dir = directory
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .context("Failed to resolve directory path")?;

    // Build the package (runs in sandbox, no root needed)
    println!(":: Building package in sandbox...");
    let pkg_file = pkgbuild::build_package(dir.clone(), &dir)?;

    println!(":: Built {}", pkg_file.display());

    if !install {
        return Ok(());
    }

    // Installation requires root
    super::ensure_root()?;

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
