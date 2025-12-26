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

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

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

/// Calculate the total installed size of all files in a directory (excluding metadata files)
fn calculate_installed_size(pkgdir: &Path) -> Result<u64> {
    let mut total: u64 = 0;

    for entry in WalkDir::new(pkgdir).min_depth(1) {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        // Skip metadata files
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        let meta = fs::symlink_metadata(path)?;
        if meta.is_file() {
            total += meta.len();
        }
    }

    Ok(total)
}

/// Finalize .PKGINFO by replacing __SIZE__ placeholder with actual size
fn finalize_pkginfo(pkginfo_path: &Path, size: u64) -> Result<()> {
    let mut content = String::new();
    File::open(pkginfo_path)
        .context("Failed to open .PKGINFO")?
        .read_to_string(&mut content)?;

    let updated = content.replace("__SIZE__", &size.to_string());

    fs::write(pkginfo_path, updated).context("Failed to write .PKGINFO")?;
    Ok(())
}

fn create_package(pkgdir: &Path, output: &Path) -> Result<()> {
    // Calculate installed size and finalize .PKGINFO
    let pkginfo_path = pkgdir.join(".PKGINFO");
    if pkginfo_path.exists() {
        let size = calculate_installed_size(pkgdir)?;
        finalize_pkginfo(&pkginfo_path, size)?;
    }

    // Collect all entries
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();

    for entry in WalkDir::new(pkgdir).min_depth(1) {
        let entry = entry.context("Failed to read directory entry")?;
        let rel_path = entry.path().strip_prefix(pkgdir)?;
        let path_str = rel_path.to_string_lossy().to_string();

        // Skip .MTREE - we'll generate it
        if path_str == ".MTREE" {
            continue;
        }

        entries.insert(path_str, entry.path().to_path_buf());
    }

    // Generate .MTREE
    let mtree_content = generate_mtree(&entries)?;
    let mtree_path = pkgdir.join(".MTREE");

    // Write compressed .MTREE
    let mtree_file = File::create(&mtree_path).context("Failed to create .MTREE")?;
    let mut encoder = flate2::write::GzEncoder::new(mtree_file, flate2::Compression::default());
    encoder.write_all(mtree_content.as_bytes())?;
    encoder.finish()?;

    // Create tar archive piped to zstd
    let mut zstd = Command::new("zstd")
        .args(["-c", "-T0", "--ultra", "-20"])
        .stdin(Stdio::piped())
        .stdout(File::create(output).context("Failed to create output file")?)
        .spawn()
        .context("Failed to spawn zstd")?;

    let zstd_stdin = zstd.stdin.take().unwrap();
    let mut tar = tar::Builder::new(BufWriter::new(zstd_stdin));

    // Add files in correct order
    // 1. .PKGINFO first
    if let Some(path) = entries.remove(".PKGINFO") {
        add_file(&mut tar, ".PKGINFO", &path)?;
    }

    // 2. .MTREE second
    add_file(&mut tar, ".MTREE", &mtree_path)?;

    // 3. Optional metadata files
    for name in [".BUILDINFO", ".INSTALL", ".CHANGELOG"] {
        if let Some(path) = entries.remove(name) {
            add_file(&mut tar, name, &path)?;
        }
    }

    // 4. All other entries (sorted by BTreeMap)
    for (name, path) in entries {
        add_entry(&mut tar, &name, &path)?;
    }

    // Finish tar and close stdin to signal zstd
    tar.into_inner()?.into_inner()?.flush()?;

    // Wait for zstd to finish
    let status = zstd.wait().context("Failed to wait for zstd")?;
    if !status.success() {
        anyhow::bail!("zstd failed with exit code {:?}", status.code());
    }

    // Clean up .MTREE
    fs::remove_file(&mtree_path).ok();

    Ok(())
}

fn add_file(tar: &mut tar::Builder<impl Write>, name: &str, path: &Path) -> Result<()> {
    let meta = fs::metadata(path).context("Failed to stat file")?;
    let mut header = tar::Header::new_gnu();

    header.set_path(name)?;
    header.set_size(meta.len());
    header.set_mode(meta.permissions().mode());
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(meta.modified()?.duration_since(UNIX_EPOCH)?.as_secs());
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();

    let file = File::open(path)?;
    tar.append(&header, file)?;
    Ok(())
}

fn add_entry(tar: &mut tar::Builder<impl Write>, name: &str, path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("Failed to stat entry")?;
    let mut header = tar::Header::new_gnu();

    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(meta.modified()?.duration_since(UNIX_EPOCH)?.as_secs());

    if meta.is_dir() {
        // Directory - ensure trailing slash
        let dir_name = if name.ends_with('/') {
            name.to_string()
        } else {
            format!("{}/", name)
        };
        header.set_path(&dir_name)?;
        header.set_size(0);
        header.set_mode(meta.permissions().mode());
        header.set_entry_type(tar::EntryType::Directory);
        header.set_cksum();
        tar.append(&header, io::empty())?;
    } else if meta.is_symlink() {
        let target = fs::read_link(path)?;
        header.set_path(name)?;
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name(target)?;
        header.set_cksum();
        tar.append(&header, io::empty())?;
    } else if meta.is_file() {
        header.set_path(name)?;
        header.set_size(meta.len());
        header.set_mode(meta.permissions().mode());
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        let file = File::open(path)?;
        tar.append(&header, file)?;
    }

    Ok(())
}

fn generate_mtree(entries: &BTreeMap<String, PathBuf>) -> Result<String> {
    let mut mtree = String::new();
    mtree.push_str("#mtree\n");
    mtree.push_str("/set type=file uid=0 gid=0\n");

    for (name, path) in entries {
        let meta = fs::symlink_metadata(path)?;

        if meta.is_dir() {
            mtree.push_str(&format!(
                "./{} type=dir mode={:04o}\n",
                name,
                meta.permissions().mode() & 0o7777
            ));
        } else if meta.is_symlink() {
            let target = fs::read_link(path)?;
            mtree.push_str(&format!(
                "./{} type=link link={}\n",
                name,
                target.display()
            ));
        } else if meta.is_file() {
            let size = meta.len();
            let mode = meta.permissions().mode() & 0o7777;
            let mtime = meta.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
            let sha256 = compute_sha256(path)?;

            mtree.push_str(&format!(
                "./{} mode={:04o} size={} time={} sha256digest={}\n",
                name, mode, size, mtime, sha256
            ));
        }
    }

    Ok(mtree)
}

fn compute_sha256(path: &Path) -> Result<String> {
    use sha2::{Sha256, Digest};

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_calculate_installed_size() {
        let dir = TempDir::new().unwrap();
        let pkgdir = dir.path();

        // Create test files
        fs::create_dir_all(pkgdir.join("usr/bin")).unwrap();
        fs::write(pkgdir.join("usr/bin/hello"), "hello world").unwrap(); // 11 bytes
        fs::write(pkgdir.join("usr/bin/test"), "test").unwrap(); // 4 bytes

        // Metadata files should be excluded
        fs::write(pkgdir.join(".PKGINFO"), "pkgname = test").unwrap();

        let size = calculate_installed_size(pkgdir).unwrap();
        assert_eq!(size, 15); // 11 + 4
    }

    #[test]
    fn test_calculate_installed_size_empty() {
        let dir = TempDir::new().unwrap();
        let size = calculate_installed_size(dir.path()).unwrap();
        assert_eq!(size, 0);
    }

    #[test]
    fn test_finalize_pkginfo() {
        let dir = TempDir::new().unwrap();
        let pkginfo = dir.path().join(".PKGINFO");

        fs::write(&pkginfo, "pkgname = test\nsize = __SIZE__\narch = x86_64").unwrap();

        finalize_pkginfo(&pkginfo, 12345).unwrap();

        let content = fs::read_to_string(&pkginfo).unwrap();
        assert!(content.contains("size = 12345"));
        assert!(!content.contains("__SIZE__"));
    }

    #[test]
    fn test_finalize_pkginfo_no_placeholder() {
        let dir = TempDir::new().unwrap();
        let pkginfo = dir.path().join(".PKGINFO");

        fs::write(&pkginfo, "pkgname = test\nsize = 100").unwrap();

        finalize_pkginfo(&pkginfo, 12345).unwrap();

        let content = fs::read_to_string(&pkginfo).unwrap();
        assert!(content.contains("size = 100")); // unchanged
    }
}
