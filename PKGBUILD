# Maintainer: Yangtse Su <yangtsesu@gmail.com>

pkgname=huobi
pkgver=0.1.1
pkgrel=1
pkgdesc="Offline currency conversion CLI backed by the Frankfurter API (v2)"
arch=('x86_64' 'aarch64')
url="https://github.com/YangtseSu/huobi"
license=('GPL-3.0-only')
depends=()
makedepends=('go')
source=("$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('174baf9cfce0f2adbb9a19fac3dd41e4842a7301e398aa5d98f8e48d5c96bf86')

build() {
  cd "$srcdir/$pkgname-$pkgver"
  CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o huobi .
}

check() {
  cd "$srcdir/$pkgname-$pkgver"
  go vet ./...
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 huobi "$pkgdir/usr/bin/huobi"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
