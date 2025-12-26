use std::path::PathBuf;

use anyhow::{Context, Result};

use super::sandbox::Sandbox;
use super::types::Pkgbuild;

pub struct BuildContext {
    pub source_dir: PathBuf,
    pub pkg_dir: PathBuf,
    pub pkgbuild: Pkgbuild,
}

impl BuildContext {
    pub fn new(source_dir: PathBuf, pkg_dir: PathBuf, pkgbuild: Pkgbuild) -> Self {
        Self {
            source_dir,
            pkg_dir,
            pkgbuild,
        }
    }

    fn run_function(&self, func: &str) -> Result<()> {
        let script = format!(
            r#"
set -e
export srcdir="{source_dir}"
export pkgdir="{pkg_dir}"
export startdir="{source_dir}"
export pkgbase="{pkgbase}"
export pkgname="{pkgname}"
export pkgver="{pkgver}"
export pkgrel="{pkgrel}"

# Rustup: point to sandboxed location
export RUSTUP_HOME="/opt/rustup"

# Cargo: use temp dir for writable state, symlink to cached registry/git
export CARGO_HOME="/tmp/.cargo"
mkdir -p "$CARGO_HOME"
[[ -d /opt/cargo/bin ]] && export PATH="/opt/cargo/bin:$PATH"
[[ -d /opt/cargo/registry && ! -e "$CARGO_HOME/registry" ]] && ln -s /opt/cargo/registry "$CARGO_HOME/registry"
[[ -d /opt/cargo/git && ! -e "$CARGO_HOME/git" ]] && ln -s /opt/cargo/git "$CARGO_HOME/git"

# Create pkg directory if needed
mkdir -p "$pkgdir"

# Source PKGBUILD and run function
cd "{source_dir}"
source PKGBUILD
{func}
"#,
            source_dir = self.source_dir.display(),
            pkg_dir = self.pkg_dir.display(),
            pkgbase = self.pkgbuild.pkgbase,
            pkgname = self.pkgbuild.package_name(),
            pkgver = self.pkgbuild.pkgver,
            pkgrel = self.pkgbuild.pkgrel,
            func = func,
        );

        let sandbox = Sandbox::new(&self.source_dir, &self.pkg_dir);
        sandbox
            .run(&script)
            .with_context(|| format!("Failed to run {}()", func))
    }

    pub fn prepare(&self) -> Result<()> {
        if self.pkgbuild.has_prepare {
            println!(":: Running prepare()...");
            self.run_function("prepare")?;
        }
        Ok(())
    }

    pub fn build(&self) -> Result<()> {
        if self.pkgbuild.has_build {
            println!(":: Running build()...");
            self.run_function("build")?;
        }
        Ok(())
    }

    pub fn check(&self) -> Result<()> {
        if self.pkgbuild.has_check {
            println!(":: Running check()...");
            self.run_function("check")?;
        }
        Ok(())
    }

    pub fn package(&self) -> Result<()> {
        println!(":: Running package()...");
        self.run_function("package")
    }

    pub fn run_all(&self) -> Result<()> {
        self.prepare()?;
        self.build()?;
        self.check()?;
        self.package()?;
        Ok(())
    }
}
