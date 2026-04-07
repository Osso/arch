//! arch-makepkg: Create pacman package archives from a package directory
//!
//! Usage: arch-makepkg <pkgdir> <destdir>
//!
//! Reads package metadata from environment variables (set by sourcing PKGBUILD):
//!   pkgname, pkgver, pkgrel, pkgdesc, url, arch, license, depends, etc.
//!
//! Creates a properly formatted pacman package with:
//! - Correct file ordering (.PKGINFO, .MTREE first)
//! - Paths without ./ prefix
//! - root:root ownership
//! - Preserved permissions including setuid

mod archive;
mod mtree;
mod pkginfo;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: arch-makepkg <pkgdir> <destdir>");
        std::process::exit(1);
    }

    let pkgdir = PathBuf::from(&args[1]);
    let destdir = PathBuf::from(&args[2]);

    let output = create_package(&pkgdir, &destdir)?;
    println!("{}", output.display());
    Ok(())
}

fn create_package(pkgdir: &Path, destdir: &Path) -> Result<PathBuf> {
    // Read package metadata from environment
    let pkgname = env::var("pkgname").context("pkgname not set")?;
    let pkgver = env::var("pkgver").context("pkgver not set")?;
    let pkgrel = env::var("pkgrel").context("pkgrel not set")?;
    let arch = env::var("arch").unwrap_or_else(|_| env::consts::ARCH.to_string());

    // Handle install script if specified
    if let Ok(install_file) = env::var("install") {
        if !install_file.is_empty() {
            // Install script is in /src (the source directory mount point)
            let src_install = Path::new("/src").join(&install_file);
            let dst_install = pkgdir.join(".INSTALL");
            if src_install.exists() {
                fs::copy(&src_install, &dst_install)
                    .with_context(|| format!("Failed to copy install script: {}", install_file))?;
                println!("  -> Adding install file...");
            } else {
                anyhow::bail!("Install script not found: {}", src_install.display());
            }
        }
    }

    // Determine output filename
    let filename = format!("{}-{}-{}-{}.pkg.tar.zst", pkgname, pkgver, pkgrel, arch);
    let output = destdir.join(&filename);

    // Calculate installed size
    let size = pkginfo::calculate_installed_size(pkgdir)?;

    // Generate and write .PKGINFO
    let pkginfo_content = pkginfo::generate_pkginfo(size)?;
    let pkginfo_path = pkgdir.join(".PKGINFO");
    fs::write(&pkginfo_path, pkginfo_content).context("Failed to write .PKGINFO")?;

    // Collect all entries (excluding .MTREE which we'll generate)
    let entries = collect_entries(pkgdir)?;

    // Generate and write .MTREE
    let mtree_path = mtree::write_mtree(pkgdir, &entries)?;

    // Create the archive
    archive::create_archive(&output, &mtree_path, entries)?;

    // Clean up .MTREE
    fs::remove_file(&mtree_path).ok();

    Ok(output)
}

/// Collect all entries in a package directory, excluding .MTREE
fn collect_entries(pkgdir: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut entries = BTreeMap::new();

    for entry in WalkDir::new(pkgdir).min_depth(1) {
        let entry = entry.context("Failed to read directory entry")?;
        let rel_path = entry.path().strip_prefix(pkgdir)?;
        let path_str = rel_path.to_string_lossy().to_string();

        if path_str != ".MTREE" {
            entries.insert(path_str, entry.path().to_path_buf());
        }
    }

    Ok(entries)
}
