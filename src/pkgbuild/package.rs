use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tar::Builder;
use walkdir::WalkDir;

use super::mtree::create_mtree;
use super::pkginfo::write_pkginfo;
use super::types::Pkgbuild;

pub fn create_package(
    pkgbuild: &Pkgbuild,
    pkgdir: &Path,
    destdir: &Path,
    arch: &str,
) -> Result<PathBuf> {
    // Generate metadata files
    println!("  Generating .PKGINFO...");
    write_pkginfo(pkgbuild, pkgdir, arch)?;

    println!("  Generating .MTREE...");
    create_mtree(pkgdir)?;

    // Build package filename
    let filename = format!(
        "{}-{}-{}.pkg.tar.zst",
        pkgbuild.package_name(),
        pkgbuild.full_version(),
        arch
    );
    let pkg_path = destdir.join(&filename);

    println!("  Creating {}...", filename);

    // Create zstd-compressed tar archive
    let file = File::create(&pkg_path).context("Failed to create package file")?;
    let encoder = zstd::Encoder::new(file, 3)
        .context("Failed to create zstd encoder")?
        .auto_finish();
    let mut builder = Builder::new(encoder);

    // Add metadata files first (order matters for pacman)
    for meta_file in &[".PKGINFO", ".BUILDINFO", ".MTREE", ".INSTALL", ".CHANGELOG"] {
        let meta_path = pkgdir.join(meta_file);
        if meta_path.exists() {
            builder
                .append_path_with_name(&meta_path, meta_file)
                .with_context(|| format!("Failed to add {} to package", meta_file))?;
        }
    }

    // Add all other files
    for entry in WalkDir::new(pkgdir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let relative = path
            .strip_prefix(pkgdir)
            .context("Failed to get relative path")?;

        // Skip metadata files (already added)
        let name = relative.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }

        let archive_path = format!("./{}", relative.display());

        if entry.file_type().is_dir() {
            builder
                .append_dir(&archive_path, path)
                .with_context(|| format!("Failed to add directory {} to package", archive_path))?;
        } else if entry.file_type().is_file() {
            builder
                .append_path_with_name(path, &archive_path)
                .with_context(|| format!("Failed to add file {} to package", archive_path))?;
        } else if entry.file_type().is_symlink() {
            // Handle symlinks
            let target = std::fs::read_link(path)?;
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            builder
                .append_link(&mut header, &archive_path, target)
                .with_context(|| format!("Failed to add symlink {} to package", archive_path))?;
        }
    }

    builder.finish().context("Failed to finish package archive")?;

    Ok(pkg_path)
}
