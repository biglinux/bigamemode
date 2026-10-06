# Maintainer: Rafael Ruscher <rruscher@gmail.com>

pkgname=bigame-mode
pkgver=2.3.0
pkgrel=1
pkgdesc="Big Game Mode, BigLinux's game mode: Turbo and per-game profiles on falcond, AI Graphics (OptiScaler) and a live view of what each game really gets"
arch=('x86_64')
url="https://github.com/ruscher/bigamemode"
license=('GPL-3.0-or-later')
# Every dependency is in Arch's own repositories (core, extra). What only
# some distributions carry, and what only some features need, is optional:
# the application says what is missing and how to install it.
depends=(
    # Runtime libraries the two binaries link against.
    'libgcc'
    'glibc'
    'glib2'
    'gtk4'
    'libadwaita>=1:1.7'
    'hicolor-icon-theme'

    # The privileged helper: a system-bus service started by systemd, every
    # method authorised through Polkit.
    'dbus'
    'polkit'
    'systemd'

    # AI Graphics: OptiScaler is fetched from its own release with curl
    # (HTTPS only, pinned SHA-256) and unpacked with bsdtar after its listing
    # is checked.
    'curl'
    'libarchive'

    # Graphics card names: the PCI database.
    'hwdata'

    # Network, on the Details page: latency (ping) and the interface's queue
    # discipline (tc).
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
    # System performance is falcond's: Turbo switches it on, and Big Game
    # Mode writes the per-game profiles it applies. BigLinux's repositories
    # carry it; on Arch it is in the AUR.
    'falcond: per-game performance profiles (without it Turbo applies only the general settings)'
    'power-profiles-daemon: the power profile games run with'
    # scx_loader is how falcond switches CPU schedulers; without it every
    # switch fails with ServiceUnknown.
    'scx-scheds: sched-ext CPU schedulers falcond switches to per game'
    'scx-tools: scx_loader, through which falcond switches schedulers'
    # What the launch settings drive.
    'gamescope: Gamescope per game and in Turbo presets'
    'mangohud: the overlay, its frame cap and Measure the difference'
    'lib32-mangohud: MangoHud in 32-bit games'
    'vkbasalt: CAS sharpening and the Nara Linux look'
    # Frame generation, with the user's own Lossless.dll from Lossless
    # Scaling, which is never shipped or downloaded. lsfg-vk 1.x and 2.x.
    'lsfg-vk: frame generation (Lossless Scaling) per game'
    # Only for NVIDIA cards, and it conflicts with the legacy NVIDIA driver
    # packages (nvidia-470xx-utils and the like), so it cannot be required.
    'nvidia-utils: GPU telemetry on NVIDIA cards (its NVML library)'
    # nmcli, as the user: the DNS comparison can make a resolver the
    # connection's DNS server. Without it the option says it is unavailable.
    'networkmanager: set the DNS server of the connection from the DNS comparison'
    # Proton and Wine synchronise through /dev/ntsync when it exists; on Arch
    # only wine pulls this in, so a Steam-only system may lack the device.
    'ntsync-autoload: load the NTSync driver at boot (Proton/Wine synchronisation)'
)
install="${pkgname}.install"
source=("${pkgname}::git+${url}.git#tag=v${pkgver}")
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

    # The desktop entry and the AppStream file with every catalogue in
    # locale/LINGUAS merged in, by gettext's own rules for both formats.
    install -d locale/merged
    msgfmt --desktop -d locale --template=data/com.biglinux.BiGameMode.desktop \
        -o locale/merged/com.biglinux.BiGameMode.desktop
    msgfmt --xml -d locale --template=data/com.biglinux.BiGameMode.metainfo.xml \
        -o locale/merged/com.biglinux.BiGameMode.metainfo.xml
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
    install -Dm644 locale/merged/com.biglinux.BiGameMode.desktop \
        "${pkgdir}/usr/share/applications/com.biglinux.BiGameMode.desktop"
    install -Dm644 locale/merged/com.biglinux.BiGameMode.metainfo.xml \
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
    # Where the helper writes falcond's global configuration; falcond's
    # package does not ship it. Owned here so removal takes it away when empty.
    install -dm755 "${pkgdir}/etc/falcond"
    # The per-game profiles the helper writes, the only part of falcond's
    # profile tree its sandbox can write. Owned here so it exists before the
    # helper starts, with falcond installed or not.
    install -dm755 "${pkgdir}/usr/share/falcond/profiles/user"

    # Icons: the application icon and the tray's. The tray gives its icon by
    # name only, so the panel draws the symbolic icon in its own colours; it
    # has to be in the theme for that.
    local icon
    for icon in com.biglinux.BiGameMode bigamemode-symbolic; do
        install -Dm644 "usr/share/icons/hicolor/scalable/apps/${icon}.svg" \
            "${pkgdir}/usr/share/icons/hicolor/scalable/apps/${icon}.svg"
    done

    # Translations, read from /usr/share/locale through the bigame-mode domain.
    local mo lang
    for mo in locale/mo/*/LC_MESSAGES/"${pkgname}".mo; do
        lang=$(basename "$(dirname "$(dirname "${mo}")")")
        install -Dm644 "${mo}" \
            "${pkgdir}/usr/share/locale/${lang}/LC_MESSAGES/${pkgname}.mo"
    done

    install -Dm644 README.md "${pkgdir}/usr/share/doc/${pkgname}/README.md"
    install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
