use crate::alpm_handle;
use anyhow::Result;

/// Search for packages in sync databases
pub fn run(pattern: &str, search_desc: bool) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;
    let pat_lower = pattern.to_lowercase();
    let mut found_any = false;

    for db in handle.syncdbs() {
        for pkg in db.pkgs() {
            let name_match = pkg.name().to_lowercase().contains(&pat_lower);
            let desc_match = search_desc
                && pkg
                    .desc()
                    .map(|d| d.to_lowercase().contains(&pat_lower))
                    .unwrap_or(false);

            if name_match || desc_match {
                found_any = true;
                let installed = handle.localdb().pkg(pkg.name()).is_ok();
                let installed_marker = if installed { " [installed]" } else { "" };

                println!(
                    "{}/{} {}{}",
                    db.name(),
                    pkg.name(),
                    pkg.version(),
                    installed_marker
                );

                if let Some(desc) = pkg.desc() {
                    println!("    {}", desc);
                }
            }
        }
    }

    if !found_any {
        eprintln!("No packages found matching '{}'", pattern);
    }

    Ok(())
}
