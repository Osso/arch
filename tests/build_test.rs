//! Integration test for package building

use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Create a minimal PKGBUILD for testing
fn create_test_pkgbuild(dir: &std::path::Path) {
    let pkgbuild = r##"
pkgname=test-package
pkgver=1.0.0
pkgrel=1
pkgdesc="A test package"
arch=('x86_64')
license=('MIT')

build() {
    echo "#!/bin/sh" > hello
    echo 'echo "Hello, World!"' >> hello
    chmod +x hello
}

package() {
    install -Dm755 hello "$pkgdir/usr/bin/hello"
}
"##;
    fs::write(dir.join("PKGBUILD"), pkgbuild).unwrap();
}

#[test]
fn test_build_creates_package() {
    let dir = TempDir::new().unwrap();
    create_test_pkgbuild(dir.path());

    // Run arch build
    let output = Command::new(env!("CARGO_BIN_EXE_arch"))
        .arg("build")
        .arg(dir.path())
        .output()
        .expect("Failed to run arch build");

    // Check build succeeded
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check package file was created
    let pkg_file = dir.path().join("test-package-1.0.0-1-x86_64.pkg.tar.zst");
    assert!(pkg_file.exists(), "Package file not created");

    // Verify package contents using zstd and tar
    let zstd_output = Command::new("zstd")
        .args(["-d", "-c"])
        .arg(&pkg_file)
        .output()
        .expect("Failed to decompress package");
    assert!(zstd_output.status.success());

    // Parse tar to get file list
    let mut archive = tar::Archive::new(&zstd_output.stdout[..]);
    let paths: Vec<String> = archive
        .entries()
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path().unwrap().to_string_lossy().to_string())
        .collect();

    // Verify correct ordering
    assert_eq!(paths[0], ".PKGINFO", "First entry should be .PKGINFO");
    assert_eq!(paths[1], ".MTREE", "Second entry should be .MTREE");

    // Verify our file is present
    assert!(
        paths.contains(&"usr/bin/hello".to_string()),
        "Package should contain usr/bin/hello"
    );
}

#[test]
fn test_build_package_metadata() {
    let dir = TempDir::new().unwrap();
    create_test_pkgbuild(dir.path());

    // Run arch build
    let output = Command::new(env!("CARGO_BIN_EXE_arch"))
        .arg("build")
        .arg(dir.path())
        .output()
        .expect("Failed to run arch build");

    assert!(output.status.success());

    let pkg_file = dir.path().join("test-package-1.0.0-1-x86_64.pkg.tar.zst");

    // Extract .PKGINFO and verify contents
    let zstd_output = Command::new("zstd")
        .args(["-d", "-c"])
        .arg(&pkg_file)
        .output()
        .unwrap();

    let mut archive = tar::Archive::new(&zstd_output.stdout[..]);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.path().unwrap().to_str() == Some(".PKGINFO") {
            let mut content = String::new();
            std::io::Read::read_to_string(&mut entry, &mut content).unwrap();

            assert!(content.contains("pkgname = test-package"));
            assert!(content.contains("pkgver = 1.0.0-1"));
            assert!(content.contains("pkgdesc = A test package"));
            assert!(content.contains("arch = x86_64"));
            assert!(content.contains("size = "), "Should have size field");
            break;
        }
    }
}

#[test]
fn test_build_preserves_permissions() {
    let dir = TempDir::new().unwrap();

    // PKGBUILD with setuid binary
    let pkgbuild = r##"
pkgname=test-setuid
pkgver=1.0.0
pkgrel=1
pkgdesc="Test setuid package"
arch=('x86_64')
license=('MIT')

build() {
    echo "#!/bin/sh" > suid-bin
}

package() {
    install -Dm4755 suid-bin "$pkgdir/usr/bin/suid-bin"
}
"##;
    fs::write(dir.path().join("PKGBUILD"), pkgbuild).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_arch"))
        .arg("build")
        .arg(dir.path())
        .output()
        .expect("Failed to run arch build");

    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let pkg_file = dir.path().join("test-setuid-1.0.0-1-x86_64.pkg.tar.zst");

    // Verify setuid bit is preserved
    let zstd_output = Command::new("zstd")
        .args(["-d", "-c"])
        .arg(&pkg_file)
        .output()
        .unwrap();

    let mut archive = tar::Archive::new(&zstd_output.stdout[..]);
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        if entry.path().unwrap().to_str() == Some("usr/bin/suid-bin") {
            let mode = entry.header().mode().unwrap();
            assert_eq!(
                mode & 0o7777,
                0o4755,
                "Setuid bit should be preserved (expected 04755, got {:04o})",
                mode & 0o7777
            );
            return;
        }
    }
    panic!("usr/bin/suid-bin not found in package");
}

#[test]
fn test_build_root_ownership() {
    let dir = TempDir::new().unwrap();
    create_test_pkgbuild(dir.path());

    let output = Command::new(env!("CARGO_BIN_EXE_arch"))
        .arg("build")
        .arg(dir.path())
        .output()
        .expect("Failed to run arch build");

    assert!(output.status.success());

    let pkg_file = dir.path().join("test-package-1.0.0-1-x86_64.pkg.tar.zst");

    let zstd_output = Command::new("zstd")
        .args(["-d", "-c"])
        .arg(&pkg_file)
        .output()
        .unwrap();

    let mut archive = tar::Archive::new(&zstd_output.stdout[..]);
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();

        assert_eq!(
            entry.header().uid().unwrap(),
            0,
            "{} should have uid 0",
            path
        );
        assert_eq!(
            entry.header().gid().unwrap(),
            0,
            "{} should have gid 0",
            path
        );
    }
}
