use crate::{alpm_handle, callbacks};
use alpm::{PackageReason, TransFlag};
use anyhow::{bail, Context, Result};

/// Remove packages and their dependencies
pub fn run(packages: &[String]) -> Result<()> {
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Verify all packages are installed
    for name in packages {
        if handle.localdb().pkg(name.as_str()).is_err() {
            bail!("Package '{}' is not installed", name);
        }
    }

    // Initialize transaction with RECURSE flag to remove unneeded deps
    handle
        .trans_init(TransFlag::RECURSE)
        .context("Failed to initialize transaction")?;

    // Add packages to remove
    for name in packages {
        let pkg = handle
            .localdb()
            .pkg(name.as_str())
            .expect("Already verified package exists");
        if let Err(e) = handle.trans_remove_pkg(pkg) {
            let _ = handle.trans_release();
            bail!("Failed to mark package for removal: {}: {:?}", name, e);
        }
    }

    // Prepare transaction
    println!(":: Checking dependencies...");
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));

    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    // Show what will be removed
    let to_remove = handle.trans_remove();
    if to_remove.is_empty() {
        println!("Nothing to remove");
        handle.trans_release().ok();
        return Ok(());
    }

    println!("\nPackages to remove ({}):", to_remove.len());
    for pkg in to_remove.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }

    // Commit transaction
    println!("\n:: Proceeding with removal...");
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));

    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }

    println!("Done!");
    Ok(())
}

/// Remove orphaned packages (installed as dependencies but no longer needed)
pub fn autoremove() -> Result<()> {
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Find orphans (packages installed as deps with no dependents)
    let orphan_names: Vec<String> = handle
        .localdb()
        .pkgs()
        .iter()
        .filter(|pkg| {
            pkg.reason() == PackageReason::Depend
                && pkg.required_by().is_empty()
                && pkg.optional_for().is_empty()
        })
        .map(|pkg| pkg.name().to_string())
        .collect();

    if orphan_names.is_empty() {
        println!("No orphaned packages to remove");
        return Ok(());
    }

    println!("Found {} orphaned packages", orphan_names.len());

    // Initialize transaction
    handle
        .trans_init(TransFlag::RECURSE)
        .context("Failed to initialize transaction")?;

    // Add orphans to remove
    for name in &orphan_names {
        let pkg = handle
            .localdb()
            .pkg(name.as_str())
            .expect("Package should exist");
        if let Err(e) = handle.trans_remove_pkg(pkg) {
            let _ = handle.trans_release();
            bail!("Failed to mark package for removal: {}: {:?}", name, e);
        }
    }

    // Prepare transaction
    println!(":: Checking dependencies...");
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));

    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    // Show what will be removed
    let to_remove = handle.trans_remove();
    if to_remove.is_empty() {
        println!("Nothing to remove");
        handle.trans_release().ok();
        return Ok(());
    }

    println!("\nOrphaned packages to remove ({}):", to_remove.len());
    for pkg in to_remove.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }

    // Commit transaction
    println!("\n:: Proceeding with removal...");
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));

    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }

    println!("Done!");
    Ok(())
}

/// Mark packages as explicitly installed (manual)
pub fn mark_manual(packages: &[String]) -> Result<()> {
    super::ensure_root()?;

    let handle = alpm_handle::init()?;

    for name in packages {
        let pkg = handle
            .localdb()
            .pkg(name.as_str())
            .map_err(|_| anyhow::anyhow!("Package '{}' is not installed", name))?;

        if pkg.reason() == PackageReason::Explicit {
            println!("{} is already marked as explicitly installed", name);
        } else {
            pkg.set_reason(PackageReason::Explicit)
                .map_err(|e| anyhow::anyhow!("Failed to set reason for {}: {:?}", name, e))?;
            println!("{} marked as explicitly installed", name);
        }
    }

    Ok(())
}

/// Mark packages as dependencies (auto)
pub fn mark_auto(packages: &[String]) -> Result<()> {
    super::ensure_root()?;

    let handle = alpm_handle::init()?;

    for name in packages {
        let pkg = handle
            .localdb()
            .pkg(name.as_str())
            .map_err(|_| anyhow::anyhow!("Package '{}' is not installed", name))?;

        if pkg.reason() == PackageReason::Depend {
            println!("{} is already marked as dependency", name);
        } else {
            pkg.set_reason(PackageReason::Depend)
                .map_err(|e| anyhow::anyhow!("Failed to set reason for {}: {:?}", name, e))?;
            println!("{} marked as dependency", name);
        }
    }

    Ok(())
}
