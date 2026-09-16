# Maintainer: Your Name <you@example.com>
# Arch Linux package for zava.
#
# Build:  make dist && makepkg -si
#   `make dist` produces zava-$pkgver.tar.gz next to this PKGBUILD, which is the
#   local source below. To package a release tarball instead, replace `source`
#   with the URL and drop the SKIP checksum.

pkgname=zava
pkgver=0.1.0
pkgrel=1
pkgdesc="Console-based audio visualizer - a pure-Rust remake of CAVA"
arch=('x86_64' 'aarch64')
url="https://github.com/<you>/zava"
license=('MIT')
# libasound is linked directly (cpal's ALSA backend); parec/pactl ship in libpulse
# and are used by the pulse/pipewire capture backend.
depends=('alsa-lib' 'libpulse')
makedepends=('cargo' 'rust')
provides=('zava')
conflicts=('zava-git')
source=("zava-$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "$pkgname-$pkgver"
  # Use the distro toolchain rather than a rustup shim, and keep the build
  # reproducible/offline with the committed lockfile.
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_NET_OFFLINE=false
}

build() {
  cd "$pkgname-$pkgver"
  cargo build --release --locked
}

check() {
  cd "$pkgname-$pkgver"
  cargo test --release --locked
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 target/release/zava "$pkgdir/usr/bin/zava"
  install -Dm644 config/zava.conf "$pkgdir/usr/share/zava/config.example"
  install -Dm644 zava.1 "$pkgdir/usr/share/man/man1/zava.1"
  install -Dm644 README.md "$pkgdir/usr/share/doc/zava/README.md"
  install -Dm644 LICENSE "$pkgdir/usr/share/doc/zava/LICENSE"
}