//! arch-makepkg: Create pacman package archives from a package directory
//!
//! Usage: arch-makepkg <pkgdir> <output.pkg.tar.zst>
//!
//! Creates a properly formatted pacman package with:
//! - Correct file ordering (.PKGINFO, .MTREE first)
//! - Paths without ./ prefix
//! - root:root ownership
//! - Preserved permissions including setuid
//! - Automatic size calculation for .PKGINFO

mod archive;
mod mtree;
mod pkginfo;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: arch-makepkg <pkgdir> <output.pkg.tar.zst>");
        std::process::exit(1);
    }

    let pkgdir = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);

    create_package(&pkgdir, &output)?;
    Ok(())
}

fn create_package(pkgdir: &Path, output: &Path) -> Result<()> {
    // Calculate installed size and finalize .PKGINFO
    let pkginfo_path = pkgdir.join(".PKGINFO");
    if pkginfo_path.exists() {
        let size = pkginfo::calculate_installed_size(pkgdir)?;
        pkginfo::finalize_pkginfo(&pkginfo_path, size)?;
    }

    // Collect all entries (excluding .MTREE which we'll generate)
    let entries = collect_entries(pkgdir)?;

    // Generate and write .MTREE
    let mtree_path = mtree::write_mtree(pkgdir, &entries)?;

    // Create the archive
    archive::create_archive(output, &mtree_path, entries)?;

    // Clean up .MTREE
    fs::remove_file(&mtree_path).ok();

    Ok(())
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
