use crate::{alpm_handle, callbacks};
use alpm::{PackageReason, TransFlag};
use anyhow::{bail, Context, Result};

/// Remove packages and their dependencies
pub fn run(packages: &[String]) -> Result<()> {
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    verify_installed_packages(&handle, packages)?;
    initialize_removal_transaction(&mut handle)?;
    add_packages_to_removal_transaction(&mut handle, packages)?;
    prepare_removal_transaction(&mut handle)?;

    if !print_removal_candidates(&mut handle, "Packages to remove") {
        return Ok(());
    }

    commit_removal_transaction(&mut handle)?;

    println!("Done!");
    Ok(())
}

fn verify_installed_packages(handle: &alpm::Alpm, packages: &[String]) -> Result<()> {
    for name in packages {
        if handle.localdb().pkg(name.as_str()).is_err() {
            bail!("Package '{}' is not installed", name);
        }
    }
    Ok(())
}

fn initialize_removal_transaction(handle: &mut alpm::Alpm) -> Result<()> {
    handle
        .trans_init(TransFlag::RECURSE)
        .context("Failed to initialize transaction")
}

fn add_packages_to_removal_transaction(handle: &mut alpm::Alpm, packages: &[String]) -> Result<()> {
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
    Ok(())
}

fn prepare_removal_transaction(handle: &mut alpm::Alpm) -> Result<()> {
    println!(":: Checking dependencies...");
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));
    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }
    Ok(())
}

fn print_removal_candidates(handle: &mut alpm::Alpm, heading: &str) -> bool {
    let to_remove = handle.trans_remove();
    if to_remove.is_empty() {
        println!("Nothing to remove");
        handle.trans_release().ok();
        return false;
    }

    println!("\n{} ({}):", heading, to_remove.len());
    for pkg in to_remove.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }
    true
}

fn commit_removal_transaction(handle: &mut alpm::Alpm) -> Result<()> {
    println!("\n:: Proceeding with removal...");
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));
    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }
    Ok(())
}

fn collect_orphan_names(handle: &alpm::Alpm) -> Vec<String> {
    handle
        .localdb()
        .pkgs()
        .iter()
        .filter(|pkg| {
            pkg.reason() == PackageReason::Depend
                && pkg.required_by().is_empty()
                && pkg.optional_for().is_empty()
        })
        .map(|pkg| pkg.name().to_string())
        .collect()
}

/// Remove orphaned packages (installed as dependencies but no longer needed)
pub fn autoremove() -> Result<()> {
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    let orphan_names = collect_orphan_names(&handle);
    if orphan_names.is_empty() {
        println!("No orphaned packages to remove");
        return Ok(());
    }

    println!("Found {} orphaned packages", orphan_names.len());
    initialize_removal_transaction(&mut handle)?;
    add_packages_to_removal_transaction(&mut handle, &orphan_names)?;
    prepare_removal_transaction(&mut handle)?;

    if !print_removal_candidates(&mut handle, "Orphaned packages to remove") {
        return Ok(());
    }

    commit_removal_transaction(&mut handle)?;

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
