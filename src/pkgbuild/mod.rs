mod mtree;
mod package;
mod parse;
mod pkginfo;
mod runner;
mod sandbox;
mod types;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

pub use parse::parse_pkgbuild;
pub use runner::BuildContext;
pub use types::Pkgbuild;

pub struct Builder {
    source_dir: PathBuf,
    pkg_dir: PathBuf,
    pkgbuild: Pkgbuild,
}

impl Builder {
    pub fn new(source_dir: PathBuf) -> Result<Self> {
        let pkgbuild_path = source_dir.join("PKGBUILD");
        if !pkgbuild_path.exists() {
            anyhow::bail!("No PKGBUILD found in {}", source_dir.display());
        }

        println!(":: Parsing PKGBUILD...");
        let pkgbuild = parse_pkgbuild(&pkgbuild_path)?;

        // Create pkg directory in /tmp (for package() output)
        let build_id = Uuid::new_v4();
        let pkg_dir = PathBuf::from(format!("/tmp/arch-pkg-{}", build_id));

        Ok(Self {
            source_dir,
            pkg_dir,
            pkgbuild,
        })
    }

    pub fn prepare_pkg_dir(&self) -> Result<()> {
        // Create pkg directory for package() output
        // Source directory is mounted with overlay (copy-on-write)
        fs::create_dir_all(&self.pkg_dir).context("Failed to create pkg directory")?;
        Ok(())
    }

    pub fn build(&self) -> Result<()> {
        let ctx = BuildContext::new(
            self.source_dir.clone(),
            self.pkg_dir.clone(),
            self.pkgbuild.clone(),
        );
        ctx.run_all()
    }

    pub fn create_package(&self, destdir: &Path) -> Result<PathBuf> {
        println!(":: Creating package...");

        let arch = get_architecture(&self.pkgbuild);

        package::create_package(&self.pkgbuild, &self.pkg_dir, destdir, &arch)
    }

    pub fn cleanup(&self) -> Result<()> {
        if self.pkg_dir.exists() {
            fs::remove_dir_all(&self.pkg_dir).context("Failed to cleanup pkg directory")?;
        }
        Ok(())
    }

}

impl Drop for Builder {
    fn drop(&mut self) {
        // Cleanup on drop (best effort)
        let _ = self.cleanup();
    }
}

fn get_architecture(pkgbuild: &Pkgbuild) -> String {
    // Check if package is architecture-independent
    if pkgbuild.arch.contains(&"any".to_string()) {
        return "any".to_string();
    }

    // Use current machine architecture
    std::env::consts::ARCH.to_string()
}

pub fn build_package(source_dir: PathBuf, destdir: &Path) -> Result<PathBuf> {
    let builder = Builder::new(source_dir)?;

    builder.prepare_pkg_dir()?;
    builder.build()?;
    let pkg_path = builder.create_package(destdir)?;

    // Cleanup is handled by Drop

    Ok(pkg_path)
}
