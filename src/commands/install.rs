use crate::{alpm_handle, callbacks};
use alpm::TransFlag;
use anyhow::{bail, Context, Result};

/// Install packages (always syncs and upgrades first for safety)
pub fn run(packages: &[String], reinstall: bool) -> Result<()> {
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Sync databases first
    println!(":: Synchronizing package databases...");
    handle
        .syncdbs_mut()
        .update(false)
        .context("Failed to sync databases")?;

    // Find packages to install
    let mut pkg_names = Vec::new();
    for name in packages {
        // Verify package exists
        let found = handle.syncdbs().iter().any(|db| db.pkg(name.as_str()).is_ok());
        if !found {
            // Try as provider
            if handle.syncdbs().find_satisfier(name.as_str()).is_none() {
                bail!("Package '{}' not found in sync databases", name);
            }
        }
        pkg_names.push(name.clone());
    }

    // Set up transaction flags
    // NEEDED = skip if already installed and up-to-date
    // For reinstall, we want to NOT set NEEDED
    let flags = if reinstall {
        TransFlag::NONE
    } else {
        TransFlag::NEEDED
    };

    // Initialize transaction
    handle
        .trans_init(flags)
        .context("Failed to initialize transaction")?;

    // Add packages to transaction - need to look them up again after trans_init
    for name in &pkg_names {
        let pkg = handle
            .syncdbs()
            .iter()
            .find_map(|db| db.pkg(name.as_str()).ok())
            .or_else(|| handle.syncdbs().find_satisfier(name.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Package '{}' not found", name))?;

        // Convert error to owned string immediately to release borrow
        let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(|e| format!("{:?}", e));
        if let Some(err) = add_err {
            let _ = handle.trans_release();
            bail!("Failed to add package {}: {}", name, err);
        }
    }

    // Always do a sysupgrade first (this is the key safety feature)
    handle
        .sync_sysupgrade(false)
        .context("Failed to set up system upgrade")?;

    // Prepare transaction (resolve dependencies)
    println!(":: Resolving dependencies...");
    // Convert error to owned string immediately to release borrow
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

