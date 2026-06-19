use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::process::Command;

#[test]
fn default_roots_include_system_and_home_install_targets() {
    let home = tempfile::tempdir().unwrap();
    let roots = default_install_roots(Some(home.path()));
    for root in [
        "/usr/local",
        "/usr/bin",
        "/usr/lib",
        "/usr/share/applications",
        "/usr/share/icons",
        "/usr/share/bash-completion",
        "/usr/share/zsh",
        "/usr/share/fish",
        "/usr/share/man",
        "/usr/share/licenses",
        "/etc",
        "/opt",
        "/usr/lib/systemd/system",
        "/etc/systemd/system",
    ] {
        assert!(roots.contains(&PathBuf::from(root)));
    }
    for root in [
        ".config",
        ".cargo/bin",
        ".local/bin",
        ".local/share/applications",
        ".local/share/icons",
    ] {
        assert!(roots.contains(&home.path().join(root)));
    }
}

#[test]
fn default_excludes_cover_source_build_dirs_and_user_caches() {
    let source = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let excludes = default_excludes(source.path(), Some(home.path()));
    for exclude in ["target", ".git", "node_modules"] {
        assert!(excludes.contains(&source.path().join(exclude)));
    }
    for exclude in [
        ".cache",
        ".cargo/registry",
        ".cargo/git",
        ".cache/git",
        ".cache/sccache",
    ] {
        assert!(excludes.contains(&home.path().join(exclude)));
    }
}

#[test]
fn sensitive_paths_include_home_secret_dirs() {
    let home = tempfile::tempdir().unwrap();
    let paths = protected_paths(Some(home.path()));

    assert!(paths.contains(&PathBuf::from("/nix")));
    assert!(paths.contains(&PathBuf::from("/syncthing/Shared")));
    assert!(paths.contains(&home.path().join("Enpass")));
    assert!(paths.contains(&home.path().join(".ssh")));
    assert!(paths.contains(&home.path().join(".gnupg")));
}

#[test]
fn synthesized_package_refuses_protected_paths() {
    let source = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let protected = PathBuf::from("/nix/store/secret");

    let error =
        synthesize_package_from_paths(source.path(), &[protected], Path::new("/"), dest.path())
            .unwrap_err();

    assert!(
        error.to_string().contains("protected path"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn stage_captured_paths_preserves_absolute_layout_symlink_and_mode() {
    let root = tempfile::tempdir().unwrap();
    let pkgdir = tempfile::tempdir().unwrap();
    let cargo_dir = root.path().join("home/osso/.cargo");
    let source_bin = root.path().join("usr/local/bin/tool");
    let cargo_bin = cargo_dir.join("bin/tool");
    let source_link = root.path().join("usr/local/bin/tool-link");

    fs::create_dir_all(source_bin.parent().unwrap()).unwrap();
    fs::create_dir_all(cargo_bin.parent().unwrap()).unwrap();
    fs::set_permissions(&cargo_dir, fs::Permissions::from_mode(0o775)).unwrap();
    fs::write(&source_bin, "#!/bin/sh\n").unwrap();
    fs::write(&cargo_bin, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&source_bin, fs::Permissions::from_mode(0o755)).unwrap();
    symlink("tool", &source_link).unwrap();

    let entries = stage_captured_paths(
        &[source_bin.clone(), cargo_bin, source_link.clone()],
        root.path(),
        pkgdir.path(),
    )
    .unwrap();

    let staged_bin = pkgdir.path().join("usr/local/bin/tool");
    let staged_link = pkgdir.path().join("usr/local/bin/tool-link");

    assert_eq!(entries.len(), 3);
    assert_eq!(
        fs::metadata(&staged_bin).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(pkgdir.path().join("home/osso/.cargo"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o775
    );
    assert_eq!(fs::read_link(&staged_link).unwrap(), PathBuf::from("tool"));
}

#[test]
fn create_synthesized_package_contains_pkginfo_and_captured_entries() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let captured = root.path().join("usr/local/bin/my-tool");

    fs::create_dir_all(captured.parent().unwrap()).unwrap();
    fs::write(&captured, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&captured, fs::Permissions::from_mode(0o755)).unwrap();

    let package =
        synthesize_package_from_paths(source.path(), &[captured], root.path(), dest.path())
            .unwrap();

    let zstd_output = Command::new("zstd")
        .args(["-d", "-c"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(zstd_output.status.success());

    let mut archive = tar::Archive::new(&zstd_output.stdout[..]);
    let paths: Vec<String> = archive
        .entries()
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().unwrap().to_string_lossy().to_string())
        .collect();

    assert_eq!(paths[0], ".PKGINFO");
    assert_eq!(paths[1], ".MTREE");
    assert!(paths.contains(&"usr/local/bin/my-tool".to_string()));
}
