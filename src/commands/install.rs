use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::{alpm_handle, callbacks, pkgbuild};
use alpm::{Alpm, SigLevel, TransFlag};
use anyhow::{bail, Context, Result};

const MAX_SYNC_RETRIES: u32 = 5;
const INITIAL_RETRY_DELAY_MS: u64 = 500;

/// Sync databases with retry logic for lock contention
fn sync_databases_with_retry(handle: &mut Alpm) -> Result<()> {
    for attempt in 0..MAX_SYNC_RETRIES {
        match handle.syncdbs_mut().update(false) {
            Ok(_) => return Ok(()),
            Err(e) => {
                let err_str = format!("{:?}", e);
                if err_str.contains("lock") || err_str.contains("Lock") {
                    if attempt < MAX_SYNC_RETRIES - 1 {
                        let delay = INITIAL_RETRY_DELAY_MS * 2u64.pow(attempt);
                        eprintln!(
                            "Database locked during sync, retrying in {}ms (attempt {}/{})",
                            delay,
                            attempt + 1,
                            MAX_SYNC_RETRIES
                        );
                        thread::sleep(Duration::from_millis(delay));
                    } else {
                        return Err(e).context("Failed to sync databases after retries");
                    }
                } else {
                    return Err(e).context("Failed to sync databases");
                }
            }
        }
    }
    Ok(())
}

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
    // Check for root first, before doing any work
    // This prevents re-exec loop (ensure_root re-execs the whole command)
    super::ensure_root()?;

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

    // Build any directories first
    for dir in &directories {
        let source_dir = std::fs::canonicalize(dir)
            .with_context(|| format!("Failed to resolve path: {}", dir))?;
        let pkg_path = pkgbuild::build_package(source_dir.clone(), &source_dir)?;
        local_files.push(pkg_path.to_string_lossy().to_string());
    }

    // If we only had directories and no install needed, we're done
    if local_files.is_empty() && repo_names.is_empty() {
        return Ok(());
    }

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Sync databases first
    println!(":: Synchronizing package databases...");
    sync_databases_with_retry(&mut handle)?;

    // Verify repo packages exist
    for name in &repo_names {
        let found = handle
            .syncdbs()
            .iter()
            .any(|db| db.pkg(name.as_str()).is_ok());
        if !found {
            if handle.syncdbs().find_satisfier(name.as_str()).is_none() {
                bail!("Package '{}' not found in sync databases", name);
            }
        }
    }

    // Set up transaction flags
    // Force reinstall for local packages (user explicitly built/specified them)
    let flags = if reinstall || !local_files.is_empty() {
        TransFlag::NONE
    } else {
        TransFlag::NEEDED
    };

    // Initialize transaction
    handle
        .trans_init(flags)
        .context("Failed to initialize transaction")?;

    // Track local packages that need forced reinstall (same version already installed)
    let mut force_reinstall_files = Vec::new();

    // Add local package files
    for file in &local_files {
        let pkg = handle
            .pkg_load(file.as_str(), true, SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to load package: {}", file))?;

        // Check if same version is already installed - libalpm silently skips these
        let pkg_name = pkg.name().to_string();
        let pkg_version = pkg.version().to_string();
        let already_installed = handle
            .localdb()
            .pkg(pkg_name.as_str())
            .map(|p| p.version().to_string() == pkg_version)
            .unwrap_or(false);

        if already_installed {
            // libalpm won't reinstall same version, use pacman directly later
            // Always reinstall local files since user explicitly built/specified them
            force_reinstall_files.push(file.clone());
            continue;
        }

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
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));

    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    // Show what will be installed
    let to_add = handle.trans_add();
    if to_add.is_empty() && force_reinstall_files.is_empty() {
        println!("Nothing to do - packages are up to date");
        handle.trans_release().ok();
        return Ok(());
    }

    if !to_add.is_empty() {
        println!("\nPackages ({}):", to_add.len());
        for pkg in to_add.iter() {
            println!("  {} {}", pkg.name(), pkg.version());
        }

        // Commit transaction
        println!("\n:: Proceeding with installation...");
        let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));

        if let Some(err) = commit_err {
            let _ = handle.trans_release();
            bail!("Failed to commit transaction: {}", err);
        }
    } else {
        handle.trans_release().ok();
    }

    // Drop the handle to release the database lock before calling pacman
    drop(handle);

    // Handle force reinstall of same-version local packages using pacman directly
    if !force_reinstall_files.is_empty() {
        println!(
            "\n:: Reinstalling {} package(s)...",
            force_reinstall_files.len()
        );
        let mut cmd = std::process::Command::new("pacman");
        cmd.arg("-U").arg("--noconfirm");
        for file in &force_reinstall_files {
            cmd.arg(file);
        }
        let status = cmd.status().context("Failed to run pacman")?;
        if !status.success() {
            bail!("pacman -U failed");
        }
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
    sync_databases_with_retry(&mut handle)?;

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
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));

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
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));

    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }

    println!("Done!");
    Ok(())
}
