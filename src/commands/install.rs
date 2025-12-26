use std::path::Path;

use crate::{alpm_handle, callbacks, pkgbuild};
use alpm::{SigLevel, TransFlag};
use anyhow::{bail, Context, Result};

/// Categorize an argument as a directory, package file, or package name
enum PackageSource {
    /// Directory containing PKGBUILD
    Directory(String),
    /// Local .pkg.tar.* file
    File(String),
    /// Package name from repos
    Name(String),
}

fn categorize_package(arg: &str) -> PackageSource {
    let path = Path::new(arg);

    // Check if it's a directory with PKGBUILD
    if path.is_dir() && path.join("PKGBUILD").exists() {
        return PackageSource::Directory(arg.to_string());
    }

    // Check if it's a package file
    if path.is_file() && is_package_file(arg) {
        return PackageSource::File(arg.to_string());
    }

    // Otherwise treat as package name
    PackageSource::Name(arg.to_string())
}

fn is_package_file(name: &str) -> bool {
    name.ends_with(".pkg.tar.zst")
        || name.ends_with(".pkg.tar.xz")
        || name.ends_with(".pkg.tar.gz")
        || name.ends_with(".pkg.tar.bz2")
        || name.ends_with(".pkg.tar")
}

/// Install packages (always syncs and upgrades first for safety)
pub fn run(packages: &[String], reinstall: bool) -> Result<()> {
    // Categorize all arguments
    let mut directories = Vec::new();
    let mut local_files = Vec::new();
    let mut repo_names = Vec::new();

    for arg in packages {
        match categorize_package(arg) {
            PackageSource::Directory(d) => directories.push(d),
            PackageSource::File(f) => local_files.push(f),
            PackageSource::Name(n) => repo_names.push(n),
        }
    }

    // Build any directories first (doesn't require root yet)
    for dir in &directories {
        let source_dir = std::fs::canonicalize(dir)
            .with_context(|| format!("Failed to resolve path: {}", dir))?;
        let pkg_path = pkgbuild::build_package(source_dir, Path::new("."))?;
        local_files.push(pkg_path.to_string_lossy().to_string());
    }

    // If we only had directories and no install needed, we're done
    if local_files.is_empty() && repo_names.is_empty() {
        return Ok(());
    }

    // Now we need root for installation
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Sync databases first
    println!(":: Synchronizing package databases...");
    handle
        .syncdbs_mut()
        .update(false)
        .context("Failed to sync databases")?;

    // Verify repo packages exist
    for name in &repo_names {
        let found = handle.syncdbs().iter().any(|db| db.pkg(name.as_str()).is_ok());
        if !found {
            if handle.syncdbs().find_satisfier(name.as_str()).is_none() {
                bail!("Package '{}' not found in sync databases", name);
            }
        }
    }

    // Set up transaction flags
    let flags = if reinstall {
        TransFlag::NONE
    } else {
        TransFlag::NEEDED
    };

    // Initialize transaction
    handle
        .trans_init(flags)
        .context("Failed to initialize transaction")?;

    // Add local package files
    for file in &local_files {
        let pkg = handle
            .pkg_load(file.as_str(), true, SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to load package: {}", file))?;

        let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(|e| format!("{:?}", e));
        if let Some(err) = add_err {
            let _ = handle.trans_release();
            bail!("Failed to add package {}: {}", file, err);
        }
    }

    // Add repo packages
    for name in &repo_names {
        let pkg = handle
            .syncdbs()
            .iter()
            .find_map(|db| db.pkg(name.as_str()).ok())
            .or_else(|| handle.syncdbs().find_satisfier(name.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Package '{}' not found", name))?;

        let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(|e| format!("{:?}", e));
        if let Some(err) = add_err {
            let _ = handle.trans_release();
            bail!("Failed to add package {}: {}", name, err);
        }
    }

    // Always do a sysupgrade first (safety feature)
    handle
        .sync_sysupgrade(false)
        .context("Failed to set up system upgrade")?;

    // Prepare transaction (resolve dependencies)
    println!(":: Resolving dependencies...");
    let prepare_err: Option<String> = handle
        .trans_prepare()
        .err()
        .map(|e| format!("{:?}", e));

    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    // Show what will be installed
    let to_add = handle.trans_add();
    if to_add.is_empty() {
        println!("Nothing to do - packages are up to date");
        handle.trans_release().ok();
        return Ok(());
    }

    println!("\nPackages ({}):", to_add.len());
    for pkg in to_add.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }

    // Commit transaction
    println!("\n:: Proceeding with installation...");
    let commit_err: Option<String> = handle
        .trans_commit()
        .err()
        .map(|e| format!("{:?}", e));

    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }

    println!("Done!");
    Ok(())
}

/// Upgrade all packages
pub fn upgrade() -> Result<()> {
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Sync databases
    println!(":: Synchronizing package databases...");
    handle
        .syncdbs_mut()
        .update(false)
        .context("Failed to sync databases")?;

    // Initialize transaction
    handle
        .trans_init(TransFlag::NONE)
        .context("Failed to initialize transaction")?;

    // Set up system upgrade
    handle
        .sync_sysupgrade(false)
        .context("Failed to set up system upgrade")?;

    // Prepare transaction
    println!(":: Resolving dependencies...");
    let prepare_err: Option<String> = handle
        .trans_prepare()
        .err()
        .map(|e| format!("{:?}", e));

    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    // Show what will be upgraded
    let to_add = handle.trans_add();
    if to_add.is_empty() {
        println!("System is up to date");
        handle.trans_release().ok();
        return Ok(());
    }

    println!("\nPackages to upgrade ({}):", to_add.len());
    for pkg in to_add.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }

    // Commit transaction
    println!("\n:: Proceeding with upgrade...");
    let commit_err: Option<String> = handle
        .trans_commit()
        .err()
        .map(|e| format!("{:?}", e));

    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }

    println!("Done!");
    Ok(())
}
