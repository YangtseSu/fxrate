# Maintainer: Yangtse Su <yangtsesu@gmail.com>

pkgname=huobi
pkgver=0.2.0
pkgrel=1
pkgdesc="Offline currency conversion CLI backed by the Frankfurter API (v2)"
arch=('x86_64' 'aarch64')
url="https://github.com/YangtseSu/huobi"
license=('GPL-3.0-only')
depends=()
makedepends=('rust')
source=("$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('e729a0d35019cbfbf762c10ba71c87befdcba514517319f1ab4fd7e982e9d924')

build() {
  cd "$srcdir/$pkgname-$pkgver"
  # ring's bundled C/assembly does not link with Arch's global LTO flag.
  CFLAGS="${CFLAGS//-flto=auto/}" \
    CXXFLAGS="${CXXFLAGS//-flto=auto/}" \
    cargo build --release --locked
}

check() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo test --locked
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 target/release/huobi "$pkgdir/usr/bin/huobi"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
