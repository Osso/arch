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

# Export scalar PKGBUILD metadata for arch-makepkg.
export pkgname pkgbase pkgver pkgrel epoch
export pkgdesc url install

append_array_env() {
    local name="$1"
    local -n values="$name"
    local serialized=""
    local value
    for value in "${values[@]}"; do
        serialized+="${value}"$'\x1f'
    done
    array_env+=("ARCH_PKG_ARRAY_${name}=${serialized%$'\x1f'}")
}

# Run build functions if they exist
type prepare &>/dev/null && { echo ':: Running prepare()...'; prepare; }
type build &>/dev/null && { echo ':: Running build()...'; build; }
type check &>/dev/null && { echo ':: Running check()...'; check; }
echo ':: Running package()...'
package

# Bash cannot export arrays. Serialize them only after the PKGBUILD functions
# have run, so package scripts still see the original arrays.
array_env=()
for array_name in arch license depends makedepends checkdepends optdepends provides conflicts replaces backup; do
    append_array_env "$array_name"
done

# Create package with array metadata passed as explicit scalar environment
# assignments, preserving spaces inside individual elements.
echo ':: Creating package...'
env "${array_env[@]}" /usr/bin/arch-makepkg "$pkgdir" /dest
"#;

    let sandbox = Sandbox::new(source_dir).with_dest_dir(dest_dir);
    sandbox.run(script).context("Failed to build package")?;

    Ok(())
}
