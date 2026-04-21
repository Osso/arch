use crate::alpm_handle;
use anyhow::Result;

fn package_matches_pattern(pkg: &alpm::Package, pattern_lower: &str, search_desc: bool) -> bool {
    if pkg.name().to_lowercase().contains(pattern_lower) {
        return true;
    }

    if !search_desc {
        return false;
    }

    pkg.desc()
        .map(|description| description.to_lowercase().contains(pattern_lower))
        .unwrap_or(false)
}

fn print_search_result(pkg: &alpm::Package, db_name: &str, installed: bool) {
    let installed_marker = if installed { " [installed]" } else { "" };
    println!(
        "{}/{} {}{}",
        db_name,
        pkg.name(),
        pkg.version(),
        installed_marker
    );

    if let Some(desc) = pkg.desc() {
        println!("    {}", desc);
    }
}

/// Search for packages in sync databases
pub fn run(pattern: &str, search_desc: bool) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let pat_lower = pattern.to_lowercase();
    let local_db = handle.localdb();
    let mut found_any = false;

    for db in handle.syncdbs() {
        for pkg in db.pkgs() {
            if !package_matches_pattern(pkg, &pat_lower, search_desc) {
                continue;
            }

            found_any = true;
            let installed = local_db.pkg(pkg.name()).is_ok();
            print_search_result(pkg, db.name(), installed);
        }
    }

    if !found_any {
        eprintln!("No packages found matching '{}'", pattern);
    }

    Ok(())
}
