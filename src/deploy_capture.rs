use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use walkdir::WalkDir;

use crate::snapshot::Snapshot;

const INSTALL_ROOTS_ENV: &str = "ARCH_INSTALL_ROOTS";
const INSTALL_EXCLUDES_ENV: &str = "ARCH_INSTALL_EXCLUDES";
const PROTECTED_PATHS_ENV: &str = "ARCH_PROTECTED_PATHS";
const HIDDEN_PATHS_ENV: &str = "ARCH_HIDDEN_PATHS";

pub struct DeployPackage {
    pub path: PathBuf,
    _workdir: TempDir,
}

pub fn build_from_deploy_script(source_dir: &Path) -> Result<DeployPackage> {
    let source_dir = fs::canonicalize(source_dir)
        .with_context(|| format!("Failed to resolve deploy source: {}", source_dir.display()))?;
    let home = invoking_home();
    let roots = default_install_roots(home.as_deref());
    let excludes = default_excludes(&source_dir, home.as_deref());

    let before = Snapshot::capture(&roots, &excludes);
    run_deploy_script(&source_dir)?;
    let after = Snapshot::capture(&roots, &excludes);
    let changed = after.added_or_changed(&before);

    if changed.is_empty() {
        bail!(
            "deploy.sh wrote no files in scanned install roots; extend deploy capture roots if this project installs elsewhere"
        );
    }

    let workdir = tempfile::tempdir().context("Failed to create deploy package workdir")?;
    let package_path =
        synthesize_package_from_paths(&source_dir, &changed, Path::new("/"), workdir.path())?;

    Ok(DeployPackage {
        path: package_path,
        _workdir: workdir,
    })
}

pub fn default_install_roots(home: Option<&Path>) -> Vec<PathBuf> {
    if let Some(roots) = paths_from_env(INSTALL_ROOTS_ENV) {
        return roots;
    }

    let mut roots = vec![
        PathBuf::from("/usr/local"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/usr/share/bash-completion"),
        PathBuf::from("/usr/share/zsh"),
        PathBuf::from("/usr/share/fish"),
        PathBuf::from("/usr/share/man"),
        PathBuf::from("/usr/share/licenses"),
        PathBuf::from("/etc"),
        PathBuf::from("/opt"),
        PathBuf::from("/usr/lib/systemd/system"),
        PathBuf::from("/etc/systemd/system"),
    ];

    if let Some(home) = home {
        roots.extend([
            home.join(".config"),
            home.join(".cargo/bin"),
            home.join(".local/bin"),
            home.join(".local/share/applications"),
            home.join(".local/share/icons"),
        ]);
    }

    roots
}

pub fn default_excludes(source_dir: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    if let Some(excludes) = paths_from_env(INSTALL_EXCLUDES_ENV) {
        return excludes;
    }

    let mut excludes = vec![
        source_dir.join("target"),
        source_dir.join(".git"),
        source_dir.join("node_modules"),
    ];

    if let Some(home) = home {
        excludes.extend([
            home.join(".cache"),
            home.join(".cargo/registry"),
            home.join(".cargo/git"),
            home.join(".cache/git"),
            home.join(".cache/sccache"),
        ]);
    }

    excludes
}

pub fn protected_paths(home: Option<&Path>) -> Vec<PathBuf> {
    paths_from_env(PROTECTED_PATHS_ENV).unwrap_or_else(|| default_sensitive_paths(home))
}

pub fn hidden_paths(home: Option<&Path>) -> Vec<PathBuf> {
    paths_from_env(HIDDEN_PATHS_ENV).unwrap_or_else(|| default_sensitive_paths(home))
}

pub fn assert_not_protected(path: &Path) -> Result<()> {
    let home = invoking_home();
    let protected_paths = protected_paths(home.as_deref());
    if let Some(protected) = protected_paths
        .iter()
        .find(|protected| path.starts_with(protected))
    {
        bail!(
            "Refusing to install protected path {} under {}",
            path.display(),
            protected.display()
        );
    }
    Ok(())
}

pub fn stage_captured_paths(paths: &[PathBuf], root: &Path, pkgdir: &Path) -> Result<Vec<String>> {
    let mut staged_entries = Vec::new();

    for source in paths {
        let relative = archive_relative_path(source, root)?;
        stage_parent_directories(&relative, root, pkgdir)?;
        let destination = pkgdir.join(&relative);
        copy_entry_to_pkgdir(source, &destination)
            .with_context(|| format!("Failed to stage captured path: {}", source.display()))?;
        staged_entries.push(relative.to_string_lossy().to_string());
    }

    staged_entries.sort();
    Ok(staged_entries)
}

fn stage_parent_directories(relative: &Path, root: &Path, pkgdir: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in relative.parent().into_iter().flat_map(Path::components) {
        current.push(component);
        let source_dir = root.join(&current);
        let destination = pkgdir.join(&current);
        if source_dir.is_dir() {
            copy_entry_to_pkgdir(&source_dir, &destination).with_context(|| {
                format!("Failed to stage parent directory: {}", source_dir.display())
            })?;
        }
    }
    Ok(())
}

pub fn synthesize_package_from_paths(
    source_dir: &Path,
    paths: &[PathBuf],
    root: &Path,
    destdir: &Path,
) -> Result<PathBuf> {
    let pkgdir = destdir.join("pkg");
    fs::create_dir_all(&pkgdir).context("Failed to create deploy package staging directory")?;
    for path in paths {
        assert_not_protected(path)?;
    }
    stage_captured_paths(paths, root, &pkgdir)?;
    write_pkginfo(source_dir, &pkgdir)?;
    let entries = collect_entries(&pkgdir)?;
    let mtree = write_mtree(&pkgdir, &entries)?;
    let output = destdir.join(package_filename(source_dir));
    create_archive(&output, &mtree, entries)?;
    fs::remove_file(mtree).ok();
    Ok(output)
}

fn invoking_home() -> Option<PathBuf> {
    if let Some(user) = invoking_user() {
        return Some(user.home);
    }
    std::env::var_os("HOME").map(PathBuf::from)
}

fn default_sensitive_paths(home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/nix"), PathBuf::from("/syncthing/Shared")];

    if let Some(home) = home {
        paths.extend([home.join("Enpass"), home.join(".ssh"), home.join(".gnupg")]);
    }

    paths
}

fn paths_from_env(name: &str) -> Option<Vec<PathBuf>> {
    let value = std::env::var_os(name)?;
    Some(std::env::split_paths(&value).collect())
}

struct InvokingUser {
    name: String,
    uid: u32,
    gid: u32,
    home: PathBuf,
}

fn invoking_user() -> Option<InvokingUser> {
    let user_name = std::env::var("SUDO_USER")
        .ok()
        .or_else(|| std::env::var("USER").ok())?;
    let user = nix::unistd::User::from_name(&user_name).ok().flatten()?;
    Some(InvokingUser {
        name: user_name,
        uid: user.uid.as_raw(),
        gid: user.gid.as_raw(),
        home: user.dir,
    })
}

fn run_deploy_script(source_dir: &Path) -> Result<()> {
    let mut command = Command::new("sh");
    command.arg("./deploy.sh").current_dir(source_dir);

    if nix::unistd::Uid::effective().as_raw() == 0 {
        if let Some(user) = invoking_user() {
            let hidden_paths = hidden_paths(Some(&user.home));
            let masks = existing_hidden_masks(&hidden_paths)?;
            command
                .env("HOME", &user.home)
                .env("USER", &user.name)
                .env("LOGNAME", &user.name);
            unsafe {
                command.pre_exec(move || {
                    enter_masked_mount_namespace(&masks)?;
                    drop_to_invoking_user(&user)
                });
            }
        }
    }

    let output = command
        .output()
        .with_context(|| format!("Failed to run {}", source_dir.join("deploy.sh").display()))?;

    if output.status.success() {
        return Ok(());
    }

    bail!(
        "deploy.sh failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn existing_hidden_masks(paths: &[PathBuf]) -> Result<Vec<CString>> {
    paths
        .iter()
        .filter(|path| path.exists())
        .map(|path| {
            CString::new(path.as_os_str().as_encoded_bytes())
                .with_context(|| format!("Hidden path contains NUL byte: {}", path.display()))
        })
        .collect()
}

fn enter_masked_mount_namespace(masks: &[CString]) -> io::Result<()> {
    unsafe {
        check_libc_call(nix::libc::unshare(nix::libc::CLONE_NEWNS))?;
        check_libc_call(nix::libc::mount(
            std::ptr::null(),
            cstring_literal("/").as_ptr(),
            std::ptr::null(),
            (nix::libc::MS_REC | nix::libc::MS_PRIVATE) as nix::libc::c_ulong,
            std::ptr::null(),
        ))?;
    }

    for target in masks {
        mask_path_with_tmpfs(target)?;
    }
    Ok(())
}

fn mask_path_with_tmpfs(target: &CString) -> io::Result<()> {
    let source = cstring_literal("tmpfs");
    let fstype = cstring_literal("tmpfs");
    let data = cstring_literal("size=4k,mode=0700");
    unsafe {
        check_libc_call(nix::libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            (nix::libc::MS_NOSUID | nix::libc::MS_NODEV | nix::libc::MS_NOEXEC)
                as nix::libc::c_ulong,
            data.as_ptr().cast(),
        ))
    }
}

fn drop_to_invoking_user(user: &InvokingUser) -> io::Result<()> {
    unsafe {
        check_libc_call(nix::libc::setgroups(0, std::ptr::null()))?;
        check_libc_call(nix::libc::setgid(user.gid))?;
        check_libc_call(nix::libc::setuid(user.uid))
    }
}

fn check_libc_call(result: nix::libc::c_int) -> io::Result<()> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn cstring_literal(value: &'static str) -> CString {
    CString::new(value).expect("static strings do not contain NUL bytes")
}

fn archive_relative_path(source: &Path, root: &Path) -> Result<PathBuf> {
    let relative = source
        .strip_prefix(root)
        .or_else(|_| source.strip_prefix("/"))
        .with_context(|| format!("Captured path is not under root: {}", source.display()))?;

    if relative.as_os_str().is_empty() {
        bail!("Refusing to package root path");
    }

    Ok(relative.to_path_buf())
}

fn copy_entry_to_pkgdir(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    if metadata.is_symlink() {
        let target = fs::read_link(source)?;
        symlink(target, destination)?;
        preserve_owner_if_root(destination, &metadata, true);
        return Ok(());
    }

    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
    } else if metadata.is_file() {
        fs::copy(source, destination)?;
    } else {
        return Ok(());
    }

    fs::set_permissions(
        destination,
        fs::Permissions::from_mode(metadata.permissions().mode()),
    )?;
    preserve_owner_if_root(destination, &metadata, false);
    Ok(())
}

fn preserve_owner_if_root(path: &Path, metadata: &fs::Metadata, symlink_entry: bool) {
    if nix::unistd::Uid::effective().as_raw() != 0 {
        return;
    }

    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok();
    if let Some(path) = path {
        unsafe {
            if symlink_entry {
                nix::libc::lchown(path.as_ptr(), metadata.uid(), metadata.gid());
            } else {
                nix::libc::chown(path.as_ptr(), metadata.uid(), metadata.gid());
            }
        }
    }
}

fn write_pkginfo(source_dir: &Path, pkgdir: &Path) -> Result<()> {
    let pkgname = package_name(source_dir);
    let size = installed_size(pkgdir)?;
    let builddate = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let content = format!(
        "pkgname = {pkgname}\npkgver = 0.0.0-1\npkgdesc = Synthesized package captured from deploy.sh\nurl = {}\nbuilddate = {builddate}\npackager = arch install deploy capture\nsize = {size}\narch = {}\n",
        source_dir.display(),
        std::env::consts::ARCH
    );
    fs::write(pkgdir.join(".PKGINFO"), content).context("Failed to write .PKGINFO")
}

fn package_name(source_dir: &Path) -> String {
    let fallback = "deploy-package";
    let raw = source_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(fallback);
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '+' | '-') {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    sanitized.trim_matches('-').trim_matches('.').to_string()
}

fn package_filename(source_dir: &Path) -> String {
    format!(
        "{}-0.0.0-1-{}.pkg.tar.zst",
        package_name(source_dir),
        std::env::consts::ARCH
    )
}

fn installed_size(pkgdir: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in WalkDir::new(pkgdir).min_depth(1) {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.is_file() {
            total += metadata.len();
        }
    }
    Ok(total)
}

fn collect_entries(pkgdir: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut entries = BTreeMap::new();
    for entry in WalkDir::new(pkgdir).min_depth(1) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(pkgdir)?;
        let name = relative.to_string_lossy().to_string();
        if name != ".MTREE" {
            entries.insert(name, entry.path().to_path_buf());
        }
    }
    Ok(entries)
}

fn write_mtree(pkgdir: &Path, entries: &BTreeMap<String, PathBuf>) -> Result<PathBuf> {
    let mtree_path = pkgdir.join(".MTREE");
    let mtree = generate_mtree(entries)?;
    let file = File::create(&mtree_path).context("Failed to create .MTREE")?;
    let mut encoder = GzEncoder::new(file, flate2::Compression::default());
    encoder.write_all(mtree.as_bytes())?;
    encoder.finish()?;
    Ok(mtree_path)
}

fn generate_mtree(entries: &BTreeMap<String, PathBuf>) -> Result<String> {
    let mut mtree = String::from("#mtree\n/set type=file\n");
    for (name, path) in entries {
        append_mtree_entry(&mut mtree, name, path)?;
    }
    Ok(mtree)
}

fn append_mtree_entry(mtree: &mut String, name: &str, path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        append_mtree_dir(mtree, name, &metadata);
    } else if metadata.is_symlink() {
        append_mtree_symlink(mtree, name, path, &metadata)?;
    } else if metadata.is_file() {
        append_mtree_file(mtree, name, path, &metadata)?;
    }
    Ok(())
}

fn append_mtree_dir(mtree: &mut String, name: &str, metadata: &fs::Metadata) {
    mtree.push_str(&format!(
        "./{} type=dir uid={} gid={} mode={:04o}\n",
        name,
        metadata.uid(),
        metadata.gid(),
        metadata.permissions().mode() & 0o7777
    ));
}

fn append_mtree_symlink(
    mtree: &mut String,
    name: &str,
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<()> {
    let target = fs::read_link(path)?;
    mtree.push_str(&format!(
        "./{} type=link uid={} gid={} link={}\n",
        name,
        metadata.uid(),
        metadata.gid(),
        target.display()
    ));
    Ok(())
}

fn append_mtree_file(
    mtree: &mut String,
    name: &str,
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<()> {
    mtree.push_str(&format!(
        "./{} uid={} gid={} mode={:04o} size={} time={} sha256digest={}\n",
        name,
        metadata.uid(),
        metadata.gid(),
        metadata.permissions().mode() & 0o7777,
        metadata.len(),
        metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs(),
        sha256(path)?
    ));
    Ok(())
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn create_archive(
    output: &Path,
    mtree_path: &Path,
    mut entries: BTreeMap<String, PathBuf>,
) -> Result<()> {
    let mut zstd = Command::new("zstd")
        .args(["-c", "-T0", "--ultra", "-20"])
        .stdin(Stdio::piped())
        .stdout(File::create(output).context("Failed to create package output")?)
        .spawn()
        .context("Failed to spawn zstd")?;
    let zstd_stdin = zstd.stdin.take().context("Failed to open zstd stdin")?;
    let mut tar = tar::Builder::new(BufWriter::new(zstd_stdin));

    if let Some(path) = entries.remove(".PKGINFO") {
        add_archive_entry(&mut tar, ".PKGINFO", &path)?;
    }
    add_archive_entry(&mut tar, ".MTREE", mtree_path)?;
    for (name, path) in entries {
        add_archive_entry(&mut tar, &name, &path)?;
    }

    tar.into_inner()?.into_inner()?.flush()?;
    let status = zstd.wait().context("Failed to wait for zstd")?;
    if !status.success() {
        bail!("zstd failed with status {}", status);
    }
    Ok(())
}

fn add_archive_entry(tar: &mut tar::Builder<impl Write>, name: &str, path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let mut header = tar::Header::new_gnu();
    header.set_path(name)?;
    header.set_uid(metadata.uid() as u64);
    header.set_gid(metadata.gid() as u64);
    header.set_mtime(metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs());

    if metadata.is_dir() {
        let directory_name = format!("{}/", name.trim_end_matches('/'));
        header.set_path(directory_name)?;
        header.set_size(0);
        header.set_mode(metadata.permissions().mode());
        header.set_entry_type(tar::EntryType::Directory);
        header.set_cksum();
        tar.append(&header, io::empty())?;
    } else if metadata.is_symlink() {
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name(fs::read_link(path)?)?;
        header.set_cksum();
        tar.append(&header, io::empty())?;
    } else if metadata.is_file() {
        header.set_size(metadata.len());
        header.set_mode(metadata.permissions().mode());
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar.append(&header, File::open(path)?)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
