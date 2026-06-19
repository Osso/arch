use crate::alpm_handle;
use alpm::PackageReason;
use anyhow::Result;

/// List packages with available upgrades
pub fn upgradable() -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();
    let mut found = false;

    for pkg in local_db.pkgs() {
        let Some(newer_version) = find_newer_sync_version(&handle, &pkg) else {
            continue;
        };
        println!("{} {} -> {}", pkg.name(), pkg.version(), newer_version);
        found = true;
    }

    if !found {
        println!("All packages are up to date");
    }

    Ok(())
}

/// List orphaned packages (installed as deps but no longer needed)
pub fn orphans() -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();
    let mut found = false;

    for pkg in local_db.pkgs() {
        // Orphan = installed as dependency + nothing requires it
        if pkg.reason() == PackageReason::Depend
            && pkg.required_by().is_empty()
            && pkg.optional_for().is_empty()
        {
            println!("{} {}", pkg.name(), pkg.version());
            found = true;
        }
    }

    if !found {
        println!("No orphaned packages");
    }

    Ok(())
}

/// List external packages (not in any sync database)
pub fn external() -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();
    let sync_dbs = handle.syncdbs();
    let mut found = false;

    for pkg in local_db.pkgs() {
        // Check if package exists in any sync db
        let in_sync = sync_dbs.iter().any(|db| db.pkg(pkg.name()).is_ok());
        if !in_sync {
            println!("{} {}", pkg.name(), pkg.version());
            found = true;
        }
    }

    if !found {
        println!("No external packages");
    }

    Ok(())
}

/// List explicitly installed packages
pub fn manual(pattern: Option<&str>) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();
    let mut found = false;

    let pat_lower = pattern.map(|p| p.to_lowercase());

    for pkg in local_db.pkgs() {
        if pkg.reason() == PackageReason::Explicit {
            // Apply pattern filter if provided
            if let Some(ref pat) = pat_lower {
                if !pkg.name().to_lowercase().contains(pat) {
                    continue;
                }
            }
            println!("{} {}", pkg.name(), pkg.version());
            found = true;
        }
    }

    if !found {
        if let Some(pat) = pattern {
            eprintln!("No explicitly installed packages matching '{}'", pat);
        } else {
            eprintln!("No explicitly installed packages");
        }
    }

    Ok(())
}

fn print_installed_package(name: &str, version: &str) {
    println!("{} {}", name, version);
}

fn find_newer_sync_version(handle: &alpm::Alpm, pkg: &alpm::Package) -> Option<String> {
    for sync_db in handle.syncdbs() {
        let Ok(sync_pkg) = sync_db.pkg(pkg.name()) else {
            continue;
        };
        if alpm::vercmp(sync_pkg.version().as_str(), pkg.version().as_str())
            == std::cmp::Ordering::Greater
        {
            return Some(sync_pkg.version().to_string());
        }
    }
    None
}

fn search_exact_package(local_db: &alpm::Db, pattern: &str) -> bool {
    if let Ok(pkg) = local_db.pkg(pattern) {
        print_installed_package(pkg.name(), pkg.version().as_str());
        return true;
    }
    false
}

fn search_packages_by_name(local_db: &alpm::Db, pattern: &str) -> bool {
    print_packages(local_db, Some(pattern))
}

fn list_all_installed_packages(local_db: &alpm::Db) -> bool {
    print_packages(local_db, None)
}

fn print_packages(local_db: &alpm::Db, pattern: Option<&str>) -> bool {
    let mut found = false;
    let pattern_lower = pattern.map(|p| p.to_lowercase());

    for pkg in local_db.pkgs() {
        if let Some(ref pat) = pattern_lower {
            if !pkg.name().to_lowercase().contains(pat) {
                continue;
            }
        }
        print_installed_package(pkg.name(), pkg.version().as_str());
        found = true;
    }

    found
}

fn report_no_installed_packages(pattern: Option<&str>) {
    if let Some(pat) = pattern {
        eprintln!("No installed packages matching '{}'", pat);
    } else {
        eprintln!("No packages installed");
    }
}

/// List installed packages, optionally filtered by pattern
pub fn run(pattern: Option<&str>, exact: bool) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();

    let found = if let Some(pat) = pattern {
        if exact {
            search_exact_package(local_db, pat)
        } else {
            search_packages_by_name(local_db, pat)
        }
    } else {
        list_all_installed_packages(local_db)
    };

    if !found {
        report_no_installed_packages(pattern);
    }

    Ok(())
}
