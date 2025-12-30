use crate::alpm_handle;
use anyhow::{bail, Result};

/// Show what packages a package depends on (needs)
pub fn needs(package: &str) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;

    // Check local db first
    if let Ok(pkg) = handle.localdb().pkg(package) {
        println!("Dependencies for {} {}:", pkg.name(), pkg.version());

        let deps: Vec<_> = pkg.depends().iter().collect();
        if deps.is_empty() {
            println!("  (none)");
        } else {
            for dep in deps {
                let installed = handle.localdb().pkg(dep.name()).is_ok();
                let marker = if installed { "" } else { " [not installed]" };
                println!("  {}{}", dep, marker);
            }
        }

        let optdeps: Vec<_> = pkg.optdepends().iter().collect();
        if !optdeps.is_empty() {
            println!("\nOptional dependencies:");
            for dep in optdeps {
                let installed = handle.localdb().pkg(dep.name()).is_ok();
                let marker = if installed { " [installed]" } else { "" };
                println!("  {}{}", dep, marker);
            }
        }

        return Ok(());
    }

    // Check sync dbs
    for db in handle.syncdbs() {
        if let Ok(pkg) = db.pkg(package) {
            println!(
                "Dependencies for {} {} ({}):",
                pkg.name(),
                pkg.version(),
                db.name()
            );

            let deps: Vec<_> = pkg.depends().iter().collect();
            if deps.is_empty() {
                println!("  (none)");
            } else {
                for dep in deps {
                    println!("  {}", dep);
                }
            }

            let optdeps: Vec<_> = pkg.optdepends().iter().collect();
            if !optdeps.is_empty() {
                println!("\nOptional dependencies:");
                for dep in optdeps {
                    println!("  {}", dep);
                }
            }

            return Ok(());
        }
    }

    bail!("Package '{}' not found", package);
}

/// Show what installed packages depend on a package (needed-by)
pub fn needed_by(package: &str) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;

    // Package must be installed to have dependents
    let pkg = handle
        .localdb()
        .pkg(package)
        .map_err(|_| anyhow::anyhow!("Package '{}' is not installed", package))?;

    println!("Packages that depend on {} {}:", pkg.name(), pkg.version());

    let required = pkg.required_by();
    if required.is_empty() {
        println!("  (none)");
    } else {
        for name in required.iter() {
            // Convert to String explicitly
            println!("  {}", String::from_utf8_lossy(name.as_bytes()));
        }
    }

    let optional = pkg.optional_for();
    if !optional.is_empty() {
        println!("\nOptional for:");
        for name in optional.iter() {
            println!("  {}", String::from_utf8_lossy(name.as_bytes()));
        }
    }

    Ok(())
}
