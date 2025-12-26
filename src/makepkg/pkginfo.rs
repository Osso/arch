//! .PKGINFO handling for pacman packages

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use walkdir::WalkDir;

/// Calculate the total installed size of all files in a directory (excluding metadata files)
pub fn calculate_installed_size(pkgdir: &Path) -> Result<u64> {
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
pub fn finalize_pkginfo(pkginfo_path: &Path, size: u64) -> Result<()> {
    let mut content = String::new();
    File::open(pkginfo_path)
        .context("Failed to open .PKGINFO")?
        .read_to_string(&mut content)?;

    let updated = content.replace("__SIZE__", &size.to_string());

    fs::write(pkginfo_path, updated).context("Failed to write .PKGINFO")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
