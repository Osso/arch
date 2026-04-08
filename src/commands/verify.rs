use crate::alpm_handle;
use alpm::Package;
use anyhow::Result;
use std::path::Path;

fn check_package_files(pkg: &Package, quiet: bool) -> bool {
    let files = pkg.files();
    let mut missing_count = 0;
    let mut missing_files = Vec::new();

    for file in files.files().iter() {
        let name = String::from_utf8_lossy(file.name());
        if name.ends_with('/') {
            continue;
        }
        let path = Path::new("/").join(name.as_ref());
        if path.symlink_metadata().is_err() {
            missing_count += 1;
            if !quiet {
                missing_files.push(format!("/{}", name));
            }
        }
    }

    if missing_count == 0 {
        return false;
    }

    if quiet {
        println!("{}", pkg.name());
    } else {
        println!(
            "{}: {} missing file{}",
            pkg.name(),
            missing_count,
            if missing_count == 1 { "" } else { "s" }
        );
        for f in missing_files {
            println!("  {}", f);
        }
    }
    true
}

/// Verify installed packages have all their files present
pub fn run(quiet: bool, package: Option<&str>) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();

    let packages: Vec<_> = if let Some(name) = package {
        match local_db.pkg(name) {
            Ok(pkg) => vec![pkg],
            Err(_) => {
                eprintln!("Package '{}' not found", name);
                std::process::exit(1);
            }
        }
    } else {
        local_db.pkgs().into_iter().collect()
    };

    let any_issues = packages.iter().any(|pkg| check_package_files(pkg, quiet));

    if !any_issues {
        if package.is_some() {
            println!("No missing files");
        } else {
            println!("All packages verified");
        }
    }

    Ok(())
}
