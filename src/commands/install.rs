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
    err.to_string().to_lowercase().contains("lock")
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

// Add local package files to the transaction. Same-version packages are
// re-added (reinstall) in-process: the transaction uses TransFlag::NONE for
// local files (see initialize_install_transaction), so alpm performs the
// reinstall itself — no need to shell out to `pacman -U`.
fn add_local_files(handle: &mut Alpm, local_files: &[String]) -> Result<()> {
    for file in local_files {
        let pkg = handle
            .pkg_load(file.as_str(), true, SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to load package: {}", file))?;

        let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(super::describe_error);
        if let Some(err) = add_err {
            let _ = handle.trans_release();
            bail!("Failed to add package {}: {}", file, err);
        }
    }
    Ok(())
}

fn add_repo_packages(handle: &mut Alpm, repo_names: &[String]) -> Result<()> {
    for name in repo_names {
        let pkg = handle
            .syncdbs()
            .iter()
            .find_map(|db| db.pkg(name.as_str()).ok())
            .or_else(|| handle.syncdbs().find_satisfier(name.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Package '{}' not found", name))?;

        let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(super::describe_error);
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

fn install_targets(handle: &mut Alpm, targets: &InstallTargets, reinstall: bool) -> Result<()> {
    verify_repo_packages(handle, &targets.repo_names)?;
    initialize_install_transaction(handle, reinstall, targets.has_local_files())?;

    add_local_files(handle, &targets.local_files)?;
    add_repo_packages(handle, &targets.repo_names)?;

    // Note: deliberately NOT calling sync_sysupgrade here. Installing specific
    // packages must not upgrade the rest of the system; `arch upgrade` is the
    // explicit entry point for that. Dependencies are still pulled in by
    // trans_prepare's resolution below.
    println!(":: Resolving dependencies...");
    commit_transaction(handle, "installation")?;
    Ok(())
}

fn commit_transaction(handle: &mut Alpm, label: &str) -> Result<()> {
    let prepare_err: Option<String> = handle.trans_prepare().err().map(super::describe_error);
    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }

    let to_add = handle.trans_add();
    if to_add.is_empty() {
        println!("Nothing to do - packages are up to date");
        handle.trans_release().ok();
        return Ok(());
    }

    println!("\nPackages ({}):", to_add.len());
    for pkg in to_add.iter() {
        println!("  {} {}", pkg.name(), pkg.version());
    }

    println!("\n:: Proceeding with {}...", label);
    let commit_err: Option<String> = handle.trans_commit().err().map(super::describe_error);
    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }
    Ok(())
}

fn prepare_upgrade_transaction(handle: &mut Alpm) -> Result<()> {
    handle
        .trans_init(TransFlag::NONE)
        .context("Failed to initialize transaction")?;

    handle
        .sync_sysupgrade(false)
        .context("Failed to set up system upgrade")?;

    println!(":: Resolving dependencies...");
    let prepare_err: Option<String> = handle.trans_prepare().err().map(super::describe_error);
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
    let commit_err: Option<String> = handle.trans_commit().err().map(super::describe_error);
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

fn install_packages(packages: &[String], reinstall: bool) -> Result<()> {
    let targets = collect_install_targets(packages)?;
    if targets.is_empty() {
        return Ok(());
    }

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    // Only hit the network when we actually need repo metadata. A pure local
    // install (e.g. `arch install .`) resolves deps against the sync DBs
    // already on disk, so it stays hermetic — no sync, no DB signature check,
    // and therefore unaffected by a flaky repo (e.g. cachyos).
    if !targets.repo_names.is_empty() {
        println!(":: Synchronizing package databases...");
        sync_databases_with_retry(&mut handle)?;
    }

    install_targets(&mut handle, &targets, reinstall)?;
    println!("Done!");
    Ok(())
}

/// Install specific packages. Local packages install hermetically; repo
/// packages trigger a DB sync first. Never performs a full system upgrade
/// (use `arch upgrade` for that).
pub fn run(packages: &[String], reinstall: bool) -> Result<()> {
    super::ensure_root()?;
    install_packages(packages, reinstall)
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
