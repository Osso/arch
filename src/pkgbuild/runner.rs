use std::path::PathBuf;

use anyhow::{Context, Result};

use super::sandbox::Sandbox;
use super::types::Pkgbuild;

pub struct BuildContext {
    pub build_dir: PathBuf,
    pub srcdir: PathBuf,
    pub pkgdir: PathBuf,
    pub pkgbuild: Pkgbuild,
}

impl BuildContext {
    pub fn new(build_dir: PathBuf, pkgbuild: Pkgbuild) -> Self {
        let srcdir = build_dir.join("src");
        let pkgdir = build_dir.join("pkg");

        Self {
            build_dir,
            srcdir,
            pkgdir,
            pkgbuild,
        }
    }

    fn run_function(&self, func: &str) -> Result<()> {
        let script = format!(
            r#"
set -e
export srcdir="{srcdir}"
export pkgdir="{pkgdir}"
export startdir="{build_dir}"
export pkgbase="{pkgbase}"
export pkgname="{pkgname}"
export pkgver="{pkgver}"
export pkgrel="{pkgrel}"

# Cargo: use build dir for writable state, symlink to host cache
export CARGO_HOME="{build_dir}/.cargo"
mkdir -p "$CARGO_HOME"
# Symlink read-only caches if they exist (use -n to not follow existing symlinks)
[[ -d "$HOME/.cargo/registry" && ! -e "$CARGO_HOME/registry" ]] && ln -s "$HOME/.cargo/registry" "$CARGO_HOME/registry"
[[ -d "$HOME/.cargo/git" && ! -e "$CARGO_HOME/git" ]] && ln -s "$HOME/.cargo/git" "$CARGO_HOME/git"

# Create directories if needed
mkdir -p "$srcdir" "$pkgdir"

# Source PKGBUILD
cd "{build_dir}"
source PKGBUILD

# Run the function
cd "$srcdir"
{func}
"#,
            srcdir = self.srcdir.display(),
            pkgdir = self.pkgdir.display(),
            build_dir = self.build_dir.display(),
            pkgbase = self.pkgbuild.pkgbase,
            pkgname = self.pkgbuild.package_name(),
            pkgver = self.pkgbuild.pkgver,
            pkgrel = self.pkgbuild.pkgrel,
            func = func,
        );

        let sandbox = Sandbox::new(&self.build_dir);
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
