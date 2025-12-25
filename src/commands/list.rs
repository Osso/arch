use crate::alpm_handle;
use alpm::PackageReason;
use anyhow::Result;

/// List packages with available upgrades
pub fn upgradable() -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();
    let mut found = false;

    for pkg in local_db.pkgs() {
        // Check if there's a newer version in sync dbs
        for sync_db in handle.syncdbs() {
            if let Ok(sync_pkg) = sync_db.pkg(pkg.name()) {
                if alpm::vercmp(sync_pkg.version().as_str(), pkg.version().as_str())
                    == std::cmp::Ordering::Greater
                {
                    println!(
                        "{} {} -> {}",
                        pkg.name(),
                        pkg.version(),
                        sync_pkg.version()
                    );
                    found = true;
                    break;
                }
            }
        }
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

/// List installed packages, optionally filtered by pattern
pub fn run(pattern: Option<&str>, exact: bool) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();

    let mut found = false;

    if let Some(pat) = pattern {
        if exact {
            // Exact match - try to find the specific package
            if let Ok(pkg) = local_db.pkg(pat) {
                println!("{} {}", pkg.name(), pkg.version());
                found = true;
            }
        } else {
            // Search by name only
            let pat_lower = pat.to_lowercase();
            for pkg in local_db.pkgs() {
                if pkg.name().to_lowercase().contains(&pat_lower) {
                    println!("{} {}", pkg.name(), pkg.version());
                    found = true;
                }
            }
        }
    } else {
        // No pattern - list all installed packages
        for pkg in local_db.pkgs() {
            println!("{} {}", pkg.name(), pkg.version());
            found = true;
        }
    }

    if !found {
        if let Some(pat) = pattern {
            eprintln!("No installed packages matching '{}'", pat);
        } else {
            eprintln!("No packages installed");
        }
    }

    Ok(())
}

