use crate::alpm_handle;
use anyhow::Result;
use std::path::Path;

/// Verify installed packages have all their files present
pub fn run(quiet: bool, package: Option<&str>) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let local_db = handle.localdb();

    let mut any_issues = false;

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

    for pkg in packages {
        let files = pkg.files();
        let file_list: Vec<_> = files.files().iter().collect();
        let mut missing_count = 0;
        let mut missing_files = Vec::new();

        for file in &file_list {
            let name = String::from_utf8_lossy(file.name());
            // Skip directories (end with /)
            if name.ends_with('/') {
                continue;
            }

            let path = Path::new("/").join(name.as_ref());
            // Use symlink_metadata to check if path exists (including broken symlinks)
            if path.symlink_metadata().is_err() {
                missing_count += 1;
                if !quiet {
                    missing_files.push(format!("/{}", name));
                }
            }
        }

        if missing_count > 0 {
            any_issues = true;
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
        }
    }

    if !any_issues {
        if package.is_some() {
            println!("No missing files");
        } else {
            println!("All packages verified");
        }
    }

    Ok(())
}
