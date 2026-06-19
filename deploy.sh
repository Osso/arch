#!/usr/bin/env bash
# Build and install the arch CLI and its helper binaries to /usr/local/bin.
# /usr/local/bin precedes /usr/bin on PATH, so this overrides any pacman-packaged
# copy in /usr/bin. Use the PKGBUILD (/usr/bin via `arch install .`) for the
# pacman-tracked install instead.
set -euo pipefail

cd "$(dirname "$0")"

cargo build --release

DEST=/usr/local/bin
for bin in arch arch-fakeroot arch-makepkg; do
    authsudo install -Dm755 "target/release/$bin" "$DEST/$bin"
done

echo "Installed arch, arch-fakeroot, arch-makepkg to $DEST"
