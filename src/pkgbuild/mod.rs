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
    build_dir: PathBuf,
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

        // Create isolated build directory in /tmp
        let build_id = Uuid::new_v4();
        let build_dir = PathBuf::from(format!("/tmp/arch-build-{}", build_id));

        Ok(Self {
            source_dir,
            build_dir,
            pkgbuild,
        })
    }

    pub fn prepare_build_dir(&self) -> Result<()> {
        println!(":: Preparing build directory...");

        // Create build directory structure
        fs::create_dir_all(&self.build_dir).context("Failed to create build directory")?;
        fs::create_dir_all(self.build_dir.join("src")).context("Failed to create src directory")?;
        fs::create_dir_all(self.build_dir.join("pkg")).context("Failed to create pkg directory")?;

        // Copy PKGBUILD
        fs::copy(
            self.source_dir.join("PKGBUILD"),
            self.build_dir.join("PKGBUILD"),
        )
        .context("Failed to copy PKGBUILD")?;

        // Copy source files (if any exist in source_dir)
        self.copy_sources()?;

        Ok(())
    }

    fn copy_sources(&self) -> Result<()> {
        // Copy any local source files referenced in the PKGBUILD
        for entry in fs::read_dir(&self.source_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Skip PKGBUILD (already copied) and hidden files
            if name_str == "PKGBUILD" || name_str.starts_with('.') {
                continue;
            }

            // Skip src and pkg directories
            if name_str == "src" || name_str == "pkg" {
                continue;
            }

            let dest = self.build_dir.join(&name);

            if path.is_dir() {
                copy_dir_recursive(&path, &dest)?;
            } else {
                fs::copy(&path, &dest)?;
            }
        }

        Ok(())
    }

    pub fn build(&self) -> Result<()> {
        let ctx = BuildContext::new(self.build_dir.clone(), self.pkgbuild.clone());
        ctx.run_all()
    }

    pub fn create_package(&self, destdir: &Path) -> Result<PathBuf> {
        println!(":: Creating package...");

        let arch = get_architecture(&self.pkgbuild);
        let pkgdir = self.build_dir.join("pkg");

        package::create_package(&self.pkgbuild, &pkgdir, destdir, &arch)
    }

    pub fn cleanup(&self) -> Result<()> {
        if self.build_dir.exists() {
            fs::remove_dir_all(&self.build_dir).context("Failed to cleanup build directory")?;
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
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

    builder.prepare_build_dir()?;
    builder.build()?;
    let pkg_path = builder.create_package(destdir)?;

    // Cleanup is handled by Drop

    Ok(pkg_path)
}
