pkgname=untethered
pkgver=0.1.0
pkgrel=4
pkgdesc='Qt Quick desktop prototype for Matrix using a bundled gomuks backend'
arch=('x86_64')
conflicts=('gomuks-native' 'unteathered')
replaces=('gomuks-native' 'unteathered')
url='https://github.com/gomuks/gomuks'
license=('AGPL-3.0-or-later' 'Apache-2.0')
depends=('qt6-base' 'qt6-declarative' 'qt6-websockets' 'qt6-imageformats' 'qt6-multimedia' 'qt6-multimedia-ffmpeg' 'ca-certificates' 'libpulse' 'curl' 'openssl')
optdepends=('qt6-wayland: Wayland platform plugin'
            'xdg-desktop-portal: screen sharing on Wayland'
            'pipewire: screen sharing on Wayland')
makedepends=('binutils')
_gomuks_release=0.2611.1
_deb="gomuks-desktop-v${_gomuks_release}-linux-amd64.deb"
_livekit_release=1.13.1
_livekit_dir="livekit-sdk-ubuntu-24.04-x64-${_livekit_release}"
_livekit_url="https://github.com/livekit/client-sdk-cpp/releases/download/v${_livekit_release}/${_livekit_dir}.tar.gz"
_livekit_sha256='fac895967a8d5dba7c29659b3963b15ac3ce1eb66a2fd267cf1948be67b32405'
source=('untethered.pro' 'main.cpp' 'matrixrtcwire.h' 'Main.qml' 'ui.qrc' 'untethered.desktop' 'LICENSE'
        "$_livekit_url"
        "${_deb}::https://github.com/gomuks/gomuks/releases/download/v${_gomuks_release}/gomuks-desktop-linux-amd64.deb")
noextract=("$_deb")
options=('!strip' '!debug')
sha256sums=(
    '0e5c6fb089757fd05795a45872fe1cd4d433e04a311a8abd07fa1bc16ed9e328'
    '74ab825bc85f943dec557ac3e8dd245ddf355b827bacecd9b093869f94c9be94'
    '9d5ac95ef727d6d22ee90b144a97c79ad3ee606245102dcc6124e947390d0f7d'
    'ac3aac449a8904654d123a770220a5e1642ef41a4616d4592c80e95ff14e730f'
    '288315478e8401b8efe16eefa0568e2be0c199bb4f9876f01d3eb7a12c77f5dd'
    '636e7f40fa3f8031264be7c0ea51d265cd13f6c48a829f09f1ae006161c8ca3b'
    '0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0'
    "$_livekit_sha256"
    '86a4f78b0cfd31de0c7530e37ee8462527a11ab5aebd8fe2e6cd778bf3345fa7'
)

build() {
    LIVEKIT_SDK="$srcdir/$_livekit_dir" LIVEKIT_RUNPATH=/usr/lib/untethered /usr/lib/qt6/bin/qmake untethered.pro
    make
}

package() {
    local backend_path=./usr/lib/gomuks-desktop/resources/gomuks
    install -Dm755 untethered "$pkgdir/usr/bin/untethered"
    install -Dm644 untethered.desktop "$pkgdir/usr/share/applications/untethered.desktop"
    ar p "$_deb" data.tar.xz | bsdtar -xOf - "$backend_path" > gomuks-backend
    install -Dm755 "$srcdir/$_livekit_dir/lib/liblivekit.so" "$pkgdir/usr/lib/untethered/liblivekit.so"
    install -Dm755 "$srcdir/$_livekit_dir/lib/liblivekit_ffi.so" "$pkgdir/usr/lib/untethered/liblivekit_ffi.so"
    install -Dm755 gomuks-backend "$pkgdir/usr/lib/untethered/gomuks"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
