use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::{alpm_handle, callbacks, pkgbuild};
use alpm::{Alpm, SigLevel, TransFlag};
use anyhow::{bail, Context, Result};

const MAX_SYNC_RETRIES: u32 = 5;
const INITIAL_RETRY_DELAY_MS: u64 = 500;

enum SyncAttemptOutcome {
    Synced,
    RetryLater,
}

fn is_lock_error(err: &alpm::Error) -> bool {
    let err_str = format!("{:?}", err);
    err_str.contains("lock") || err_str.contains("Lock")
}

fn is_last_sync_attempt(attempt: u32) -> bool {
    attempt >= MAX_SYNC_RETRIES - 1
}

fn wait_for_sync_retry(attempt: u32) {
    let delay = INITIAL_RETRY_DELAY_MS * 2u64.pow(attempt);
    eprintln!(
        "Database locked during sync, retrying in {}ms (attempt {}/{})",
        delay,
        attempt + 1,
        MAX_SYNC_RETRIES
    );
    thread::sleep(Duration::from_millis(delay));
}

fn handle_sync_update_error(err: alpm::Error, attempt: u32) -> Result<()> {
    if !is_lock_error(&err) {
        return Err(err).context("Failed to sync databases");
    }

    if is_last_sync_attempt(attempt) {
        return Err(err).context("Failed to sync databases after retries");
    }

    wait_for_sync_retry(attempt);
    Ok(())
}

fn sync_databases_attempt(handle: &mut Alpm, attempt: u32) -> Result<SyncAttemptOutcome> {
    match handle.syncdbs_mut().update(false) {
        Ok(_) => Ok(SyncAttemptOutcome::Synced),
        Err(err) => {
            handle_sync_update_error(err, attempt)?;
            Ok(SyncAttemptOutcome::RetryLater)
        }
    }
}

fn sync_databases_for_attempts(handle: &mut Alpm, attempts: std::ops::Range<u32>) -> Result<()> {
    for attempt in attempts {
        match sync_databases_attempt(handle, attempt)? {
            SyncAttemptOutcome::Synced => return Ok(()),
            SyncAttemptOutcome::RetryLater => {}
        }
    }
    Ok(())
}

/// Sync databases with retry logic for lock contention
fn sync_databases_with_retry(handle: &mut Alpm) -> Result<()> {
    sync_databases_for_attempts(handle, 0..MAX_SYNC_RETRIES)
}

/// Categorize an argument as a directory, package file, or package name
enum PackageSource {
    /// Directory containing PKGBUILD
    Directory(String),
    /// Local .pkg.tar.* file
    File(String),
    /// Package name from repos
    Name(String),
}

struct InstallTargets {
    local_files: Vec<String>,
    repo_names: Vec<String>,
}

impl InstallTargets {
    fn is_empty(&self) -> bool {
        self.local_files.is_empty() && self.repo_names.is_empty()
    }

    fn has_local_files(&self) -> bool {
        !self.local_files.is_empty()
    }
}

fn categorize_package(arg: &str) -> PackageSource {
    let path = Path::new(arg);

    // Check if it's a directory with PKGBUILD
    if path.is_dir() && path.join("PKGBUILD").exists() {
        return PackageSource::Directory(arg.to_string());
    }

    // Check if it's a package file
    if path.is_file() && is_package_file(arg) {
        return PackageSource::File(arg.to_string());
    }

    // Otherwise treat as package name
    PackageSource::Name(arg.to_string())
}

fn is_package_file(name: &str) -> bool {
    name.ends_with(".pkg.tar.zst")
        || name.ends_with(".pkg.tar.xz")
        || name.ends_with(".pkg.tar.gz")
        || name.ends_with(".pkg.tar.bz2")
        || name.ends_with(".pkg.tar")
}

fn build_directories(directories: &[String], local_files: &mut Vec<String>) -> Result<()> {
    for dir in directories {
        let source_dir = std::fs::canonicalize(dir)
            .with_context(|| format!("Failed to resolve path: {}", dir))?;
        let pkg_path = pkgbuild::build_package(source_dir.clone(), &source_dir)?;
        local_files.push(pkg_path.to_string_lossy().to_string());
    }
    Ok(())
}

fn collect_install_targets(packages: &[String]) -> Result<InstallTargets> {
    let mut directories = Vec::new();
    let mut local_files = Vec::new();
    let mut repo_names = Vec::new();

    for arg in packages {
        match categorize_package(arg) {
            PackageSource::Directory(directory) => directories.push(directory),
            PackageSource::File(file) => local_files.push(file),
            PackageSource::Name(name) => repo_names.push(name),
        }
    }

    build_directories(&directories, &mut local_files)?;

    Ok(InstallTargets {
        local_files,
        repo_names,
    })
}

fn verify_repo_packages(handle: &Alpm, repo_names: &[String]) -> Result<()> {
    for name in repo_names {
        let found = handle
            .syncdbs()
            .iter()
            .any(|db| db.pkg(name.as_str()).is_ok());
        if !found && handle.syncdbs().find_satisfier(name.as_str()).is_none() {
            bail!("Package '{}' not found in sync databases", name);
        }
    }
    Ok(())
}

fn add_local_files(handle: &mut Alpm, local_files: &[String]) -> Result<Vec<String>> {
    let mut force_reinstall = Vec::new();
    for file in local_files {
        let pkg = handle
            .pkg_load(file.as_str(), true, SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to load package: {}", file))?;

        let pkg_name = pkg.name().to_string();
        let pkg_version = pkg.version();
        let already_installed = handle
            .localdb()
            .pkg(pkg_name.as_str())
            .map(|p| p.version() == pkg_version)
            .unwrap_or(false);

        if already_installed {
            force_reinstall.push(file.clone());
            continue;
        }

        let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(|e| format!("{:?}", e));
        if let Some(err) = add_err {
            let _ = handle.trans_release();
            bail!("Failed to add package {}: {}", file, err);
        }
    }
    Ok(force_reinstall)
}

fn add_repo_packages(handle: &mut Alpm, repo_names: &[String]) -> Result<()> {
    for name in repo_names {
        let pkg = handle
            .syncdbs()
            .iter()
            .find_map(|db| db.pkg(name.as_str()).ok())
            .or_else(|| handle.syncdbs().find_satisfier(name.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Package '{}' not found", name))?;

        let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(|e| format!("{:?}", e));
        if let Some(err) = add_err {
            let _ = handle.trans_release();
            bail!("Failed to add package {}: {}", name, err);
        }
    }
    Ok(())
}

fn initialize_install_transaction(
    handle: &mut Alpm,
    reinstall: bool,
    has_local_files: bool,
) -> Result<()> {
    let flags = if reinstall || has_local_files {
        TransFlag::NONE
    } else {
        TransFlag::NEEDED
    };
    handle
        .trans_init(flags)
        .context("Failed to initialize transaction")
}

fn install_targets(
    handle: &mut Alpm,
    targets: &InstallTargets,
    reinstall: bool,
) -> Result<Vec<String>> {
    verify_repo_packages(handle, &targets.repo_names)?;
    initialize_install_transaction(handle, reinstall, targets.has_local_files())?;

    let force_reinstall_files = add_local_files(handle, &targets.local_files)?;
    add_repo_packages(handle, &targets.repo_names)?;

    handle
        .sync_sysupgrade(false)
        .context("Failed to set up system upgrade")?;

    println!(":: Resolving dependencies...");
    commit_transaction(handle, &force_reinstall_files, "installation")?;
    Ok(force_reinstall_files)
}

fn commit_transaction(
    handle: &mut Alpm,
    force_reinstall_files: &[String],
    label: &str,
) -> Result<()> {
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));
    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    let to_add = handle.trans_add();
    if to_add.is_empty() && force_reinstall_files.is_empty() {
        println!("Nothing to do - packages are up to date");
        handle.trans_release().ok();
        return Ok(());
    }

    if to_add.is_empty() {
        handle.trans_release().ok();
        return Ok(());
    }

    println!("\nPackages ({}):", to_add.len());
    for pkg in to_add.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }

    println!("\n:: Proceeding with {}...", label);
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));
    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }
    Ok(())
}

fn force_reinstall_with_pacman(files: &[String]) -> Result<()> {
    println!("\n:: Reinstalling {} package(s)...", files.len());
    let mut cmd = std::process::Command::new("pacman");
    cmd.arg("-U").arg("--noconfirm");
    for file in files {
        cmd.arg(file);
    }
    let status = cmd.status().context("Failed to run pacman")?;
    if !status.success() {
        bail!("pacman -U failed");
    }
    Ok(())
}

fn reinstall_local_files_if_needed(force_reinstall_files: &[String]) -> Result<()> {
    if force_reinstall_files.is_empty() {
        return Ok(());
    }

    force_reinstall_with_pacman(force_reinstall_files)
}

fn prepare_upgrade_transaction(handle: &mut Alpm) -> Result<()> {
    handle
        .trans_init(TransFlag::NONE)
        .context("Failed to initialize transaction")?;

    handle
        .sync_sysupgrade(false)
        .context("Failed to set up system upgrade")?;

    println!(":: Resolving dependencies...");
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));
    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    Ok(())
}

fn commit_upgrade_transaction(handle: &mut Alpm) -> Result<()> {
    let to_add = handle.trans_add();
    if to_add.is_empty() {
        println!("System is up to date");
        handle.trans_release().ok();
        return Ok(());
    }

    println!("\nPackages to upgrade ({}):", to_add.len());
    for pkg in to_add.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }

    println!("\n:: Proceeding with upgrade...");
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));
    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }

    Ok(())
}

fn upgrade_system(handle: &mut Alpm) -> Result<()> {
    prepare_upgrade_transaction(handle)?;
    commit_upgrade_transaction(handle)
}

/// Install packages (always syncs and upgrades first for safety)
pub fn run(packages: &[String], reinstall: bool) -> Result<()> {
    super::ensure_root()?;

    let targets = collect_install_targets(packages)?;
    if targets.is_empty() {
        return Ok(());
    }

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    println!(":: Synchronizing package databases...");
    sync_databases_with_retry(&mut handle)?;

    let force_reinstall_files = install_targets(&mut handle, &targets, reinstall)?;
    drop(handle);
    reinstall_local_files_if_needed(&force_reinstall_files)?;
    println!("Done!");
    Ok(())
}

/// Upgrade all packages
pub fn upgrade() -> Result<()> {
    super::ensure_root()?;

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    println!(":: Synchronizing package databases...");
    sync_databases_with_retry(&mut handle)?;

    upgrade_system(&mut handle)?;
    println!("Done!");
    Ok(())
}
