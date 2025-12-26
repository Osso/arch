use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::sandbox::Sandbox;
use super::types::Pkgbuild;

pub struct BuildContext {
    pub source_dir: PathBuf,
    pub pkgbuild: Pkgbuild,
}

impl BuildContext {
    pub fn new(source_dir: PathBuf, pkgbuild: Pkgbuild) -> Self {
        Self {
            source_dir,
            pkgbuild,
        }
    }

    /// Build package and create archive in a single sandbox session
    /// The pkg directory only exists in /tmp inside the sandbox
    pub fn build_and_package(&self, dest_path: &Path, arch: &str, pkginfo: &str) -> Result<()> {
        let filename = format!(
            "{}-{}-{}.pkg.tar.zst",
            self.pkgbuild.package_name(),
            self.pkgbuild.full_version(),
            arch
        );

        // Build functions to call
        let mut functions = Vec::new();
        if self.pkgbuild.has_prepare {
            functions.push("prepare");
        }
        if self.pkgbuild.has_build {
            functions.push("build");
        }
        if self.pkgbuild.has_check {
            functions.push("check");
        }
        functions.push("package");

        // Generate function calls with logging
        let function_calls: String = functions
            .iter()
            .map(|f| format!("echo ':: Running {}()...'\n{}", f, f))
            .collect::<Vec<_>>()
            .join("\n");

        // Escape pkginfo for shell heredoc
        let pkginfo_escaped = pkginfo.replace("'", "'\\''");

        let script = format!(
            r#"
set -e
export srcdir="/src"
export pkgdir="/tmp/pkg"
export startdir="/src"
export pkgbase="{pkgbase}"
export pkgname="{pkgname}"
export pkgver="{pkgver}"
export pkgrel="{pkgrel}"

# Rustup: point to sandboxed location
export RUSTUP_HOME="/opt/rustup"

# Cargo: use the bound ~/.cargo
export CARGO_HOME="/opt/cargo"
[[ -d /opt/cargo/bin ]] && export PATH="/opt/cargo/bin:$PATH"

# Add arch helper binaries to PATH
export PATH="/opt/arch:$PATH"

# Create pkg directory
mkdir -p "$pkgdir"

# Source PKGBUILD and run build functions
cd /src
source PKGBUILD
{function_calls}

# Write .PKGINFO (arch-makepkg will calculate and fill in __SIZE__)
cat > "$pkgdir/.PKGINFO" << 'PKGINFO_EOF'
{pkginfo_escaped}
PKGINFO_EOF

# Create package archive using arch-makepkg
echo ':: Creating package...'
arch-makepkg "$pkgdir" "/dest/{filename}"
"#,
            pkgbase = self.pkgbuild.pkgbase,
            pkgname = self.pkgbuild.package_name(),
            pkgver = self.pkgbuild.pkgver,
            pkgrel = self.pkgbuild.pkgrel,
            function_calls = function_calls,
            pkginfo_escaped = pkginfo_escaped,
            filename = filename,
        );

        let sandbox = Sandbox::new(&self.source_dir).with_dest_dir(dest_path);
        sandbox
            .run(&script)
            .context("Failed to build package")?;

        Ok(())
    }
}
