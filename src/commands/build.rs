use crate::pkgbuild;
use crate::{alpm_handle, callbacks};
use alpm::{SigLevel, TransFlag};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

fn resolve_build_dir(directory: Option<PathBuf>) -> Result<PathBuf> {
    let dir = directory
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .context("Failed to resolve directory path")?;
    Ok(dir)
}

fn load_and_add_package(handle: &mut alpm::Alpm, pkg_file: &Path) -> Result<()> {
    let pkg_path = pkg_file.to_string_lossy();
    let pkg = handle
        .pkg_load(pkg_path.as_ref(), true, SigLevel::USE_DEFAULT)
        .context("Failed to load package file")?;
    println!("  {} {}", pkg.name(), pkg.version());

    let add_err: Option<String> = handle.trans_add_pkg(pkg).err().map(|e| format!("{:?}", e));
    if let Some(err) = add_err {
        let _ = handle.trans_release();
        bail!("Failed to add package: {}", err);
    }
    Ok(())
}

fn prepare_build_transaction(handle: &mut alpm::Alpm) -> Result<()> {
    let prepare_err: Option<String> = handle.trans_prepare().err().map(|e| format!("{:?}", e));
    if let Some(err) = prepare_err {
        let _ = handle.trans_release();
        bail!("Failed to prepare transaction: {}", err);
    }
    Ok(())
}

fn commit_build_transaction(handle: &mut alpm::Alpm) -> Result<()> {
    let commit_err: Option<String> = handle.trans_commit().err().map(|e| format!("{:?}", e));
    if let Some(err) = commit_err {
        let _ = handle.trans_release();
        bail!("Failed to commit transaction: {}", err);
    }
    Ok(())
}

fn install_built_package(pkg_file: &Path) -> Result<()> {
    super::ensure_root()?;
    println!(":: Installing {}...", pkg_file.display());

    let mut handle = alpm_handle::init()?;
    callbacks::register(&handle);

    handle
        .trans_init(TransFlag::NONE)
        .context("Failed to initialize transaction")?;

    load_and_add_package(&mut handle, pkg_file)?;
    prepare_build_transaction(&mut handle)?;
    commit_build_transaction(&mut handle)?;

    handle.trans_release().ok();
    println!("Done!");
    Ok(())
}

pub fn run(directory: Option<PathBuf>, install: bool) -> Result<()> {
    let dir = resolve_build_dir(directory)?;

    println!(":: Building package in sandbox...");
    let pkg_file = pkgbuild::build_package(dir.clone(), &dir)?;
    println!(":: Built {}", pkg_file.display());

    if install {
        install_built_package(&pkg_file)?;
    }

    Ok(())
}
