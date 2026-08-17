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
sha256sums=('7211aa44f510ebbe0ddd60ea3ab2f6c4bd799448ef683ccf64bd0df81064e8af')

build() {
  cd "$srcdir/$pkgname-$pkgver"
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
