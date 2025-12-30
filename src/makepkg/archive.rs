//! Tar archive creation for pacman packages

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};

/// Create a zstd-compressed tar archive from package entries
///
/// Adds files in correct order: .PKGINFO, .MTREE, optional metadata, then all other entries
pub fn create_archive(
    output: &Path,
    mtree_path: &Path,
    mut entries: BTreeMap<String, PathBuf>,
) -> Result<()> {
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
    add_file(&mut tar, ".MTREE", mtree_path)?;

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

    Ok(())
}

/// Add a regular file to the tar archive with root ownership
pub fn add_file(tar: &mut tar::Builder<impl Write>, name: &str, path: &Path) -> Result<()> {
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

/// Add any entry (file, directory, or symlink) to the tar archive with root ownership
pub fn add_entry(tar: &mut tar::Builder<impl Write>, name: &str, path: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;
    use tempfile::TempDir;

    #[test]
    fn test_add_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello").unwrap();

        let mut buf = Vec::new();
        {
            let mut tar = tar::Builder::new(BufWriter::new(&mut buf));
            add_file(&mut tar, "test.txt", &file_path).unwrap();
            tar.finish().unwrap();
        }

        // Verify tar contains the file
        let mut archive = tar::Archive::new(&buf[..]);
        let entry = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap().to_str().unwrap(), "test.txt");
        assert_eq!(entry.header().uid().unwrap(), 0);
        assert_eq!(entry.header().gid().unwrap(), 0);
    }

    #[test]
    fn test_add_entry_directory() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("usr");
        fs::create_dir(&subdir).unwrap();

        let mut buf = Vec::new();
        {
            let mut tar = tar::Builder::new(BufWriter::new(&mut buf));
            add_entry(&mut tar, "usr", &subdir).unwrap();
            tar.finish().unwrap();
        }

        let mut archive = tar::Archive::new(&buf[..]);
        let entry = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap().to_str().unwrap(), "usr/");
        assert_eq!(entry.header().entry_type(), tar::EntryType::Directory);
    }

    #[test]
    fn test_add_entry_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        fs::write(&target, "content").unwrap();
        std::os::unix::fs::symlink("target", &link).unwrap();

        let mut buf = Vec::new();
        {
            let mut tar = tar::Builder::new(BufWriter::new(&mut buf));
            add_entry(&mut tar, "link", &link).unwrap();
            tar.finish().unwrap();
        }

        let mut archive = tar::Archive::new(&buf[..]);
        let entry = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!(entry.header().entry_type(), tar::EntryType::Symlink);
        assert_eq!(
            entry.link_name().unwrap().unwrap().to_str().unwrap(),
            "target"
        );
    }

    #[test]
    fn test_create_archive() {
        use std::process::Command;

        let dir = TempDir::new().unwrap();
        let pkgdir = dir.path().join("pkg");
        fs::create_dir(&pkgdir).unwrap();

        // Create test files
        fs::write(pkgdir.join(".PKGINFO"), "pkgname = test\n").unwrap();
        fs::create_dir_all(pkgdir.join("usr/bin")).unwrap();
        fs::write(pkgdir.join("usr/bin/hello"), "#!/bin/sh\necho hello").unwrap();

        // Create .MTREE (normally done by mtree module)
        let mtree_content = "#mtree\n/set type=file uid=0 gid=0\n";
        let mtree_file = pkgdir.join(".MTREE");
        let mut encoder = flate2::write::GzEncoder::new(
            File::create(&mtree_file).unwrap(),
            flate2::Compression::default(),
        );
        encoder.write_all(mtree_content.as_bytes()).unwrap();
        encoder.finish().unwrap();

        // Build entries map
        let mut entries = BTreeMap::new();
        entries.insert(".PKGINFO".to_string(), pkgdir.join(".PKGINFO"));
        entries.insert("usr".to_string(), pkgdir.join("usr"));
        entries.insert("usr/bin".to_string(), pkgdir.join("usr/bin"));
        entries.insert("usr/bin/hello".to_string(), pkgdir.join("usr/bin/hello"));

        // Create archive
        let output = dir.path().join("test.pkg.tar.zst");
        create_archive(&output, &mtree_file, entries).unwrap();

        // Verify archive exists and has content
        assert!(output.exists());
        assert!(fs::metadata(&output).unwrap().len() > 0);

        // Decompress and verify contents using zstd and tar
        let zstd_output = Command::new("zstd")
            .args(["-d", "-c"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(zstd_output.status.success());

        let mut archive = tar::Archive::new(&zstd_output.stdout[..]);
        let paths: Vec<String> = archive
            .entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path().unwrap().to_string_lossy().to_string())
            .collect();

        // Verify correct ordering (.PKGINFO first, .MTREE second)
        assert_eq!(paths[0], ".PKGINFO");
        assert_eq!(paths[1], ".MTREE");
        assert!(paths.contains(&"usr/".to_string()));
        assert!(paths.contains(&"usr/bin/".to_string()));
        assert!(paths.contains(&"usr/bin/hello".to_string()));
    }
}
