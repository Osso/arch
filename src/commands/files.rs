use crate::alpm_handle;
use anyhow::{bail, Context, Result};

/// List files owned by a package (dpkg -L / pacman -Ql)
pub fn files(package: &str) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;

    // Check local db first
    if let Ok(pkg) = handle.localdb().pkg(package) {
        let files = pkg.files();
        let file_list: Vec<_> = files.files().iter().collect();

        if file_list.is_empty() {
            println!("No files recorded for {}", package);
        } else {
            for file in file_list {
                let name = String::from_utf8_lossy(file.name());
                println!("/{}", name);
            }
        }
        return Ok(());
    }

    // Check sync dbs (files db must be synced with pacman -Fy)
    for db in handle.syncdbs() {
        if let Ok(pkg) = db.pkg(package) {
            let files = pkg.files();
            let file_list: Vec<_> = files.files().iter().collect();

            if file_list.is_empty() {
                println!("No files recorded for {} (run 'pacman -Fy' to sync file databases)", package);
            } else {
                for file in file_list {
                    let name = String::from_utf8_lossy(file.name());
                    println!("/{}", name);
                }
            }
            return Ok(());
        }
    }

    bail!("Package '{}' not found", package);
}

/// Find which installed package owns a file (dpkg -S / pacman -Qo)
pub fn belongs(path: &str) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;

    // Normalize path - remove leading slash for comparison
    let search_path = path.strip_prefix('/').unwrap_or(path);

    // Search through all installed packages
    for pkg in handle.localdb().pkgs() {
        let files = pkg.files();

        // Check if this package owns the file
        if files.contains(search_path).is_some() {
            println!("{} {} owns /{}", pkg.name(), pkg.version(), search_path);
            return Ok(());
        }

        // Also try with trailing components for directory matching
        for file in files.files() {
            let file_name = String::from_utf8_lossy(file.name());
            if file_name.trim_end_matches('/') == search_path.trim_end_matches('/') {
                println!("{} {} owns /{}", pkg.name(), pkg.version(), search_path);
                return Ok(());
            }
        }
    }

    bail!("No package owns '{}'", path);
}

/// Sync file databases (equivalent to pacman -Fy)
fn sync_file_databases() -> Result<()> {
    if !nix::unistd::Uid::effective().is_root() {
        bail!("Syncing file databases requires root privileges. Run with sudo or use --no-sync.");
    }

    println!(":: Syncing file databases...");

    let mut handle = alpm_handle::init_files()?;

    handle
        .syncdbs_mut()
        .update(false)
        .context("Failed to sync file databases")?;

    Ok(())
}

/// Search for packages that provide a file (apt-file search / pacman -F)
pub fn provides(pattern: &str, sync: bool) -> Result<()> {
    // Sync file databases if requested
    if sync {
        sync_file_databases()?;
    }

    // Normalize pattern - remove leading slash for comparison
    let search_pattern = pattern.strip_prefix('/').unwrap_or(pattern);

    let mut found = false;

    // First search installed packages (use regular handle for local db)
    let local_handle = alpm_handle::init_readonly()?;
    for pkg in local_handle.localdb().pkgs() {
        for file in pkg.files().files() {
            let file_name = String::from_utf8_lossy(file.name());
            if file_name.contains(search_pattern) {
                println!("{} {} [installed]", pkg.name(), pkg.version());
                println!("    /{}", file_name);
                found = true;
            }
        }
    }

    // Then search sync dbs using files handle
    let files_handle = alpm_handle::init_files_readonly()?;
    for db in files_handle.syncdbs() {
        for pkg in db.pkgs() {
            // Skip if already shown as installed
            if local_handle.localdb().pkg(pkg.name()).is_ok() {
                continue;
            }

            for file in pkg.files().files() {
                let file_name = String::from_utf8_lossy(file.name());
                if file_name.contains(search_pattern) {
                    println!("{} {} ({})", pkg.name(), pkg.version(), db.name());
                    println!("    /{}", file_name);
                    found = true;
                }
            }
        }
    }

    if !found {
        println!("No packages found providing '{}'", pattern);
    }

    Ok(())
}
