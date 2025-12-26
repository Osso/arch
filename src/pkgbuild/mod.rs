mod parse;
mod pkginfo;
mod runner;
mod sandbox;
mod types;

use std::path::{Path, PathBuf};

use anyhow::Result;

pub use parse::parse_pkgbuild;
pub use runner::BuildContext;
pub use types::Pkgbuild;

fn get_architecture(pkgbuild: &Pkgbuild) -> String {
    // Check if package is architecture-independent
    if pkgbuild.arch.contains(&"any".to_string()) {
        return "any".to_string();
    }

    // Use current machine architecture
    std::env::consts::ARCH.to_string()
}

pub fn build_package(source_dir: PathBuf, destdir: &Path) -> Result<PathBuf> {
    let pkgbuild_path = source_dir.join("PKGBUILD");
    if !pkgbuild_path.exists() {
        anyhow::bail!("No PKGBUILD found in {}", source_dir.display());
    }

    println!(":: Parsing PKGBUILD...");
    let pkgbuild = parse_pkgbuild(&pkgbuild_path)?;

    let arch = get_architecture(&pkgbuild);
    let pkginfo = pkginfo::generate_pkginfo(&pkgbuild, &arch)?;

    let ctx = BuildContext::new(source_dir, pkgbuild.clone());
    ctx.build_and_package(destdir, &arch, &pkginfo)?;

    let filename = format!(
        "{}-{}-{}.pkg.tar.zst",
        pkgbuild.package_name(),
        pkgbuild.full_version(),
        arch
    );
    Ok(destdir.join(filename))
}
