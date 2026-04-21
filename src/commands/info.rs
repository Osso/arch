use crate::alpm_handle;
use alpm::Package;
use anyhow::{bail, Result};

/// Show package information
pub fn run(package: &str) -> Result<()> {
    let handle = alpm_handle::init_readonly()?;

    // First check if it's installed locally
    if let Ok(pkg) = handle.localdb().pkg(package) {
        print_package_info(pkg, true);
        return Ok(());
    }

    // Otherwise search sync databases
    for db in handle.syncdbs() {
        if let Ok(pkg) = db.pkg(package) {
            print_package_info(pkg, false);
            return Ok(());
        }
    }

    bail!("Package '{}' not found", package);
}

fn print_package_info(pkg: &Package, installed: bool) {
    print_base_package_info(pkg);
    print_collection_fields(pkg);
    print_size_fields(pkg);
    print_installation_state(pkg, installed);
}

fn print_optional_field<T: std::fmt::Display>(label: &str, value: Option<T>) {
    if let Some(value) = value {
        println!("{:<15} : {}", label, value);
    }
}

fn print_joined_field(label: &str, values: &[String], separator: &str) {
    if !values.is_empty() {
        println!("{:<15} : {}", label, values.join(separator));
    }
}

fn print_base_package_info(pkg: &Package) {
    println!("Name            : {}", pkg.name());
    println!("Version         : {}", pkg.version());
    print_optional_field("Description", pkg.desc());
    print_optional_field("Architecture", pkg.arch());
    print_optional_field("URL", pkg.url());
    println!(
        "Licenses        : {}",
        pkg.licenses().iter().collect::<Vec<_>>().join(" ")
    );
}

fn print_collection_fields(pkg: &Package) {
    let groups: Vec<_> = pkg.groups().iter().collect();
    let groups_as_strings = groups
        .iter()
        .map(|group| group.to_string())
        .collect::<Vec<_>>();
    print_joined_field("Groups", &groups_as_strings, " ");

    let provides: Vec<_> = pkg.provides().iter().map(|d| d.to_string()).collect();
    print_joined_field("Provides", &provides, "  ");

    let depends: Vec<_> = pkg.depends().iter().map(|d| d.to_string()).collect();
    print_joined_field("Depends On", &depends, "  ");

    let optdepends: Vec<_> = pkg.optdepends().iter().map(|d| d.to_string()).collect();
    print_joined_field("Optional Deps", &optdepends, "  ");
}

fn print_size_fields(pkg: &Package) {
    print_optional_field("Packager", pkg.packager());
    println!("Installed Size  : {}", format_size(pkg.isize()));
}

fn print_installation_state(pkg: &Package, installed: bool) {
    if installed {
        println!(
            "Install Date    : {}",
            format_date(pkg.install_date().unwrap_or(0))
        );
        println!("Install Reason  : {:?}", pkg.reason());
        return;
    }

    println!("Download Size   : {}", format_size(pkg.size()));
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
