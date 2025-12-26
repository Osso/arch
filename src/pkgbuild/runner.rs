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

# Cargo: use temp dir for writable state, symlink to cached registry/git
export CARGO_HOME="/tmp/.cargo"
mkdir -p "$CARGO_HOME"
[[ -d /opt/cargo/bin ]] && export PATH="/opt/cargo/bin:$PATH"
[[ -d /opt/cargo/registry && ! -e "$CARGO_HOME/registry" ]] && ln -s /opt/cargo/registry "$CARGO_HOME/registry"
[[ -d /opt/cargo/git && ! -e "$CARGO_HOME/git" ]] && ln -s /opt/cargo/git "$CARGO_HOME/git"

# Create pkg directory
mkdir -p "$pkgdir"

# Source PKGBUILD and run build functions
cd /src
source PKGBUILD
{function_calls}

# Calculate installed size and write .PKGINFO
SIZE=$(find "$pkgdir" -type f -exec stat -c%s {{}} + 2>/dev/null | awk '{{s+=$1}} END {{print s+0}}')
cat > "$pkgdir/.PKGINFO" << 'PKGINFO_EOF'
{pkginfo_escaped}
PKGINFO_EOF
sed -i "s/__SIZE__/$SIZE/" "$pkgdir/.PKGINFO"

# Create .MTREE and package archive
cd "$pkgdir"

echo ':: Creating package...'
echo '  Generating .MTREE and archive...'

# Create .MTREE (paths without ./ prefix for compatibility)
find . -mindepth 1 ! -name '.MTREE' -printf '%P\0' | sort -z | \
    bsdtar --create --file - --format=mtree \
        --options '!all,use-set,type,uid,gid,mode,time,size,sha256,link' \
        --null --files-from - --no-recursion | \
    gzip -c -n > .MTREE

# Create package archive
{{
    [[ -f .PKGINFO ]] && echo .PKGINFO
    [[ -f .BUILDINFO ]] && echo .BUILDINFO
    [[ -f .MTREE ]] && echo .MTREE
    [[ -f .INSTALL ]] && echo .INSTALL
    [[ -f .CHANGELOG ]] && echo .CHANGELOG
    find . -mindepth 1 ! -name '.PKGINFO' ! -name '.BUILDINFO' ! -name '.MTREE' ! -name '.INSTALL' ! -name '.CHANGELOG' -printf '%P\n' | sort
}} | bsdtar --create --file - --files-from - --no-recursion | zstd -c -T0 --ultra -20 > "/dest/{filename}"
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
