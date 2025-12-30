use crate::alpm_handle;
use alpm::Package;
use anyhow::{bail, Result};

/// Show package information
pub fn run(package: &str) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;

    // First check if it's installed locally
    if let Ok(pkg) = handle.localdb().pkg(package) {
        print_package_info(&pkg, true);
        return Ok(());
    }

    // Otherwise search sync databases
    for db in handle.syncdbs() {
        if let Ok(pkg) = db.pkg(package) {
            print_package_info(&pkg, false);
            return Ok(());
        }
    }

    bail!("Package '{}' not found", package);
}

fn print_package_info(pkg: &Package, installed: bool) {
    println!("Name            : {}", pkg.name());
    println!("Version         : {}", pkg.version());
    if let Some(desc) = pkg.desc() {
        println!("Description     : {}", desc);
    }
    if let Some(arch) = pkg.arch() {
        println!("Architecture    : {}", arch);
    }
    if let Some(url) = pkg.url() {
        println!("URL             : {}", url);
    }
    println!(
        "Licenses        : {}",
        pkg.licenses().iter().collect::<Vec<_>>().join(" ")
    );

    let groups: Vec<_> = pkg.groups().iter().collect();
    if !groups.is_empty() {
        println!("Groups          : {}", groups.join(" "));
    }

    let provides: Vec<_> = pkg.provides().iter().map(|d| d.to_string()).collect();
    if !provides.is_empty() {
        println!("Provides        : {}", provides.join("  "));
    }

    let depends: Vec<_> = pkg.depends().iter().map(|d| d.to_string()).collect();
    if !depends.is_empty() {
        println!("Depends On      : {}", depends.join("  "));
    }

    let optdepends: Vec<_> = pkg.optdepends().iter().map(|d| d.to_string()).collect();
    if !optdepends.is_empty() {
        println!("Optional Deps   : {}", optdepends.join("  "));
    }

    if let Some(packager) = pkg.packager() {
        println!("Packager        : {}", packager);
    }

    println!("Installed Size  : {}", format_size(pkg.isize()));

    if installed {
        println!(
            "Install Date    : {}",
            format_date(pkg.install_date().unwrap_or(0))
        );
        println!("Install Reason  : {:?}", pkg.reason());
    } else {
        println!("Download Size   : {}", format_size(pkg.size()));
    }
}

fn format_size(bytes: i64) -> String {
    const KIB: i64 = 1024;
    const MIB: i64 = KIB * 1024;
    const GIB: i64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_date(timestamp: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp as u64);
    // Simple formatting without external crate
    format!("{:?}", datetime)
}
