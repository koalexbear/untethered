# Local package: build the current checkout's committed Rust frontend.
# The version names that commit (0.1.0.r<commit count>.g<short hash>), so
# pacman and paru see every new commit as an upgrade. No pkgver() function:
# makepkg would write its result over this line.
pkgname=untethered
_commit=$(git -C "$startdir" rev-parse HEAD)
pkgver=0.1.0.r$(git -C "$startdir" rev-list --count "$_commit").g$(git -C "$startdir" rev-parse --short=7 "$_commit")
pkgrel=1
pkgdesc='Native Rust/Iced Matrix client'
arch=('x86_64')
conflicts=('gomuks-native' 'unteathered')
replaces=('gomuks-native' 'unteathered')
url='https://github.com/koalexbear/untethered'
license=('AGPL-3.0-or-later' 'OFL-1.1' 'MIT')
optdepends=('xdg-desktop-portal: file pickers, file drops, window sharing and camera access'
            'gst-plugin-pipewire: read portal-authorized camera and window streams'
            'gst-plugins-good: MP4 and WebM videos in the media viewer'
            'gst-libav: H.264 and other video codecs in the media viewer'
            'libpulse: microphone capture and playback for encrypted DM calls')
depends=('ca-certificates' 'fontconfig' 'gcc-libs' 'glibc' 'gstreamer' 'gst-plugins-base-libs'
         'libpipewire' 'libx11' 'libxcb' 'libxcursor' 'libxi' 'libxkbcommon-x11' 'libxrandr' 'wayland')
makedepends=('cargo' 'git' 'clang' 'glib2' 'cmake' 'protobuf')
options=('!debug' '!lto')

source=("untethered-src::git+file://$startdir#commit=$_commit")
sha256sums=('SKIP')

build() {
    cd "$srcdir/untethered-src/rust-client"
    cargo build --release --locked --target-dir "$srcdir/target" -j "${UNTETHERED_BUILD_JOBS:-3}"
}

check() {
    cd "$srcdir/untethered-src/rust-client"
    cargo test --release --locked --target-dir "$srcdir/target" -j "${UNTETHERED_BUILD_JOBS:-3}"
}

package() {
    cd "$srcdir/untethered-src"
    install -Dm755 "$srcdir/target/release/untethered-iced" "$pkgdir/usr/bin/untethered"
    install -Dm755 "$srcdir/target/release/untethered-cameras" "$pkgdir/usr/lib/untethered/untethered-cameras"
    install -Dm644 io.github.koalexbear.Untethered.desktop "$pkgdir/usr/share/applications/io.github.koalexbear.Untethered.desktop"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/untethered/LICENSE"
    install -Dm644 rust-client/assets/LICENSE "$pkgdir/usr/share/licenses/untethered/Liberation-fonts"
    install -Dm644 rust-client/vendor/smithay-clipboard/LICENSE "$pkgdir/usr/share/licenses/untethered/smithay-clipboard"
}
