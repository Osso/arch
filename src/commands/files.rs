use crate::alpm_handle;
use anyhow::{bail, Context, Result};
use std::collections::HashSet;

fn collect_owned_file_names(files: &alpm::FileList) -> Vec<String> {
    files
        .files()
        .iter()
        .map(|file| String::from_utf8_lossy(file.name()).into_owned())
        .collect()
}

fn print_owned_files(package: &str, file_names: &[String], empty_hint: Option<&str>) {
    if file_names.is_empty() {
        match empty_hint {
            Some(hint) => println!("No files recorded for {} ({})", package, hint),
            None => println!("No files recorded for {}", package),
        }
        return;
    }

    for name in file_names {
        println!("/{}", name);
    }
}

/// List files owned by a package (dpkg -L / pacman -Ql)
pub fn files(package: &str) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;

    // Check local db first
    if let Ok(pkg) = handle.localdb().pkg(package) {
        let file_names = collect_owned_file_names(pkg.files());
        print_owned_files(package, &file_names, None);
        return Ok(());
    }

    // Check sync dbs (files db must be synced with pacman -Fy)
    for db in handle.syncdbs() {
        if let Ok(pkg) = db.pkg(package) {
            let file_names = collect_owned_file_names(pkg.files());
            print_owned_files(
                package,
                &file_names,
                Some("run 'pacman -Fy' to sync file databases"),
            );
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

fn print_provider_match(pkg_name: &str, pkg_version: &str, source: &str, file_name: &str) {
    println!("{} {} {}", pkg_name, pkg_version, source);
    println!("    /{}", file_name);
}

fn collect_installed_package_names(handle: &alpm::Alpm) -> HashSet<String> {
    handle
        .localdb()
        .pkgs()
        .iter()
        .map(|pkg| pkg.name().to_string())
        .collect()
}

fn search_installed_providers(local_handle: &alpm::Alpm, search_pattern: &str) -> bool {
    let mut found = false;

    for pkg in local_handle.localdb().pkgs() {
        for file in pkg.files().files() {
            let file_name = String::from_utf8_lossy(file.name());
            if file_name.contains(search_pattern) {
                print_provider_match(
                    pkg.name(),
                    pkg.version().as_str(),
                    "[installed]",
                    &file_name,
                );
                found = true;
            }
        }
    }

    found
}

fn search_sync_providers(
    files_handle: &alpm::Alpm,
    installed_packages: &HashSet<String>,
    search_pattern: &str,
) -> bool {
    let mut found = false;

    for db in files_handle.syncdbs() {
        for pkg in db.pkgs() {
            if installed_packages.contains(pkg.name()) {
                continue;
            }
            found |= search_sync_package_files(&pkg, db.name(), search_pattern);
        }
    }

    found
}

fn search_sync_package_files(pkg: &alpm::Package, db_name: &str, search_pattern: &str) -> bool {
    let mut found = false;
    let source = format!("({})", db_name);
    for file in pkg.files().files() {
        let file_name = String::from_utf8_lossy(file.name());
        if !file_name.contains(search_pattern) {
            continue;
        }
        print_provider_match(
            pkg.name(),
            pkg.version().as_str(),
            source.as_str(),
            &file_name,
        );
        found = true;
    }
    found
}

/// Search for packages that provide a file (apt-file search / pacman -F)
pub fn provides(pattern: &str, sync: bool) -> Result<()> {
    // Sync file databases if requested
    if sync {
        sync_file_databases()?;
    }

    // Normalize pattern - remove leading slash for comparison
    let search_pattern = pattern.strip_prefix('/').unwrap_or(pattern);

    // First search installed packages (use regular handle for local db)
    let local_handle = alpm_handle::init_readonly()?;
    let installed_packages = collect_installed_package_names(&local_handle);
    let mut found = search_installed_providers(&local_handle, search_pattern);

    // Then search sync dbs using files handle
    let files_handle = alpm_handle::init_files_readonly()?;
    found |= search_sync_providers(&files_handle, &installed_packages, search_pattern);

    if !found {
        println!("No packages found providing '{}'", pattern);
    }

    Ok(())
}
