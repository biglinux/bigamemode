# Maintainer: Rafael Ruscher <rruscher@gmail.com>
# Contributor: BigLinux Team <dev@biglinux.com.br>

pkgname=bigame-mode
pkgver=1.0.0
pkgrel=1
pkgdesc="Gaming mode for BigLinux: per-game profiles on falcond, AI Graphics (OptiScaler), benchmarks and diagnostics in a GTK4/libadwaita app"
arch=('x86_64')
url="https://github.com/ruscher/bigamemode"
license=('GPL-3.0-or-later')
depends=(
    # Runtime libraries the two binaries link against.
    'gcc-libs'
    'glibc'
    'glib2'
    'gtk4'
    'libadwaita'
    'hicolor-icon-theme'

    # The privileged helper: a system-bus service started by systemd, every
    # method authorised through Polkit.
    'dbus'
    'polkit'
    'systemd'

    # System performance is falcond's. BiGame-mode writes its per-game
    # profiles and reads the status it publishes; the power profile it asks
    # for goes through power-profiles-daemon.
    'falcond'
    'power-profiles-daemon'

    # AI Graphics: OptiScaler is fetched from its own release with curl
    # (HTTPS only, pinned SHA-256) and unpacked with bsdtar after its listing
    # is checked.
    'curl'
    'libarchive'

    # Graphics card names: the PCI database (hwdata) and lspci (pciutils).
    'hwdata'
    'pciutils'

    # Network: latency on the dashboard (ping) and the interface's queue
    # discipline in Diagnostics (tc).
    'iputils'
    'iproute2'
)
makedepends=(
    'git'
    'rust'
    'gettext'
    'python'
)
optdepends=(
    'scx-tools: scx_loader, needed for falcond to switch sched-ext schedulers'
    'scx-scheds: sched-ext CPU schedulers (LAVD, bpfland, ...)'
    'gamescope: per-game micro-compositor, resolution and FSR upscaling'
    'mangohud: in-game performance overlay'
    'lsfg-vk: Lossless Scaling frame generation on Vulkan (needs your own Lossless.dll)'
    'vkbasalt: Vulkan post-processing layer'
    'steam: Steam games, detection and launch options'
    'lutris: Lutris games'
    'heroic-games-launcher: Epic, GOG and Amazon games'
    'nvidia-utils: GPU telemetry on NVIDIA cards (nvidia-smi)'
    'supertuxkart: native Linux benchmark'
)
install="${pkgname}.install"
source=("${pkgname}::git+${url}.git")
sha256sums=('SKIP')

_cargo_env() {
    export CARGO_HOME="${srcdir}/cargo-home"
    export CARGO_TARGET_DIR="${srcdir}/${pkgname}/bigame-engine/target"
    export RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }--remap-path-prefix=${srcdir}=."
}

prepare() {
    cd "${srcdir}/${pkgname}"
    _cargo_env
    cargo fetch --locked --target "$(rustc --print host-tuple)" \
        --manifest-path bigame-engine/Cargo.toml
}

build() {
    cd "${srcdir}/${pkgname}"
    _cargo_env
    # The whole workspace: bigame-ui (the application) and bigame-daemon
    # (the root helper). build.rs compiles the gresource bundle with
    # glib-compile-resources from glib2.
    cargo build --release --frozen --workspace \
        --manifest-path bigame-engine/Cargo.toml

    # Verify the translation template is current, then compile the catalogues.
    #
    # It is checked rather than regenerated: a package build is the wrong place
    # to silently change source files, and a stale template should fail the
    # build so it gets fixed in the repository. xgettext has no Rust mode,
    # hence the project's own extractor.
    python3 locale/extract-strings.py --check

    for po in locale/*.po; do
        lang=$(basename "${po}" .po)
        install -d "locale/mo/${lang}/LC_MESSAGES"
        msgfmt --check "${po}" -o "locale/mo/${lang}/LC_MESSAGES/${pkgname}.mo"
    done
}

check() {
    cd "${srcdir}/${pkgname}"
    _cargo_env
    cargo test --release --frozen --workspace \
        --manifest-path bigame-engine/Cargo.toml
}

package() {
    cd "${srcdir}/${pkgname}"

    # Binaries. bigame-ui matches the desktop file's Exec key; bigame-daemon
    # matches the systemd unit and the D-Bus activation file.
    install -Dm755 bigame-engine/target/release/bigame-ui \
        "${pkgdir}/usr/bin/bigame-ui"
    install -Dm755 bigame-engine/target/release/bigame-daemon \
        "${pkgdir}/usr/bin/bigame-daemon"

    # Desktop integration.
    install -Dm644 data/com.biglinux.BiGameMode.desktop \
        "${pkgdir}/usr/share/applications/com.biglinux.BiGameMode.desktop"
    install -Dm644 data/com.biglinux.BiGameMode.metainfo.xml \
        "${pkgdir}/usr/share/metainfo/com.biglinux.BiGameMode.metainfo.xml"

    # Root helper: Polkit actions, system-bus policy, systemd unit and the
    # D-Bus activation file that starts it through that unit.
    install -Dm644 data/com.biglinux.BiGameMode.policy \
        "${pkgdir}/usr/share/polkit-1/actions/com.biglinux.BiGameMode.policy"
    install -Dm644 data/com.biglinux.BiGameMode.conf \
        "${pkgdir}/usr/share/dbus-1/system.d/com.biglinux.BiGameMode.conf"
    install -Dm644 data/bigame-daemon.service \
        "${pkgdir}/usr/lib/systemd/system/bigame-daemon.service"
    install -Dm644 data/com.biglinux.BiGameMode.service \
        "${pkgdir}/usr/share/dbus-1/system-services/com.biglinux.BiGameMode.service"

    # Icons: the application icon and the four tray states. The tray also
    # carries them inside the binary; installing them lets tray hosts that
    # draw by icon name find them in the theme.
    local icon
    for icon in com.biglinux.BiGameMode input-gaming-symbolic-{blue,green,yellow,red}; do
        install -Dm644 "usr/share/icons/hicolor/scalable/apps/${icon}.svg" \
            "${pkgdir}/usr/share/icons/hicolor/scalable/apps/${icon}.svg"
    done

    # Translations, read from /usr/share/locale through the bigame-mode domain.
    local mo lang
    for mo in locale/mo/*/LC_MESSAGES/${pkgname}.mo; do
        lang=$(basename "$(dirname "$(dirname "${mo}")")")
        install -Dm644 "${mo}" \
            "${pkgdir}/usr/share/locale/${lang}/LC_MESSAGES/${pkgname}.mo"
    done

    install -Dm644 README.md "${pkgdir}/usr/share/doc/${pkgname}/README.md"
    install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
