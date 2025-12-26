//! .MTREE generation for pacman packages

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use sha2::{Sha256, Digest};

/// Write compressed .MTREE file for a package
pub fn write_mtree(pkgdir: &Path, entries: &BTreeMap<String, PathBuf>) -> Result<PathBuf> {
    let content = generate_mtree(entries)?;
    let mtree_path = pkgdir.join(".MTREE");

    let file = File::create(&mtree_path).context("Failed to create .MTREE")?;
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    encoder.write_all(content.as_bytes())?;
    encoder.finish()?;

    Ok(mtree_path)
}

/// Generate .MTREE content for a package
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

/// Compute SHA256 hash of a file
pub fn compute_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_mtree_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("hello.txt");
        fs::write(&file_path, "hello").unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("hello.txt".to_string(), file_path);

        let mtree = generate_mtree(&entries).unwrap();

        assert!(mtree.starts_with("#mtree\n"));
        assert!(mtree.contains("/set type=file uid=0 gid=0"));
        assert!(mtree.contains("./hello.txt mode="));
        assert!(mtree.contains("size=5"));
        assert!(mtree.contains("sha256digest="));
    }

    #[test]
    fn test_generate_mtree_directory() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("usr");
        fs::create_dir(&subdir).unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("usr".to_string(), subdir);

        let mtree = generate_mtree(&entries).unwrap();

        assert!(mtree.contains("./usr type=dir mode="));
    }

    #[test]
    fn test_generate_mtree_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        fs::write(&target, "target").unwrap();
        std::os::unix::fs::symlink("target", &link).unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("link".to_string(), link);

        let mtree = generate_mtree(&entries).unwrap();

        assert!(mtree.contains("./link type=link link=target"));
    }

    #[test]
    fn test_compute_sha256() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test");
        fs::write(&file, "hello").unwrap();

        let hash = compute_sha256(&file).unwrap();

        // SHA256 of "hello"
        assert_eq!(hash, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }
}
