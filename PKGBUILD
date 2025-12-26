pkgname=arch-tools
pkgver=0.1.0
pkgrel=1
pkgdesc="Arch Linux package management tools"
arch=('x86_64')
license=('MIT')
depends=('pacman' 'bubblewrap' 'zstd')
makedepends=('cargo' 'rust')

build() {
    cd "$startdir"
    cargo build --release
}

package() {
    cd "$startdir"
    install -Dm755 target/release/arch "$pkgdir/usr/bin/arch"
    install -Dm755 target/release/arch-fakeroot "$pkgdir/usr/bin/arch-fakeroot"
    install -Dm755 target/release/arch-makepkg "$pkgdir/usr/bin/arch-makepkg"
}
