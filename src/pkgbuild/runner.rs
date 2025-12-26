use std::path::Path;

use anyhow::{Context, Result};

use super::sandbox::Sandbox;

/// Run PKGBUILD build in sandbox
/// Returns path to created package (found by globbing destdir)
pub fn build_in_sandbox(source_dir: &Path, dest_dir: &Path) -> Result<()> {
    let script = r#"
set -e
export srcdir="/src"
export pkgdir="/tmp/pkg"
export startdir="/src"

# Create pkg directory
mkdir -p "$pkgdir"

# Source PKGBUILD
cd /src
source PKGBUILD

# Export all PKGBUILD variables for arch-makepkg
export pkgname pkgbase pkgver pkgrel epoch
export pkgdesc url arch license
export depends makedepends checkdepends optdepends
export provides conflicts replaces backup

# Run build functions if they exist
type prepare &>/dev/null && { echo ':: Running prepare()...'; prepare; }
type build &>/dev/null && { echo ':: Running build()...'; build; }
type check &>/dev/null && { echo ':: Running check()...'; check; }
echo ':: Running package()...'
package

# Create package
echo ':: Creating package...'
arch-makepkg "$pkgdir" /dest
"#;

    let sandbox = Sandbox::new(source_dir).with_dest_dir(dest_dir);
    sandbox.run(script).context("Failed to build package")?;

    Ok(())
}
