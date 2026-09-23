# falcond's own unit, D-Bus policy, sysusers entry and default configuration
# are deliberately NOT installed here. falcond is a separate package and a
# hard dependency of this one; shipping our own copies would overwrite the
# files it owns.
#
# That was not hypothetical: the copy previously installed from this directory
# set PrivateTmp=yes, which gives falcond a private /tmp and therefore hides
# /tmp/falcond_status from every other process — including this application,
# whose status display reads exactly that file. It also installed the default
# config as /etc/falcond/falcond.conf, a filename falcond never reads.

# Desktop file
install_data(
  app_id + '.desktop',
  install_dir: datadir / 'applications',
)

# Metainfo / AppStream
install_data(
  app_id + '.metainfo.xml',
  install_dir: datadir / 'metainfo',
)

# PolicyKit policy
install_data(
  app_id + '.policy',
  install_dir: datadir / 'polkit-1' / 'actions',
)

# D-Bus policy for bigame-daemon
install_data(
  'com.biglinux.BiGameMode.conf',
  install_dir: datadir / 'dbus-1' / 'system.d',
)

# D-Bus service for activation
install_data(
  'com.biglinux.BiGameMode.service',
  install_dir: datadir / 'dbus-1' / 'system-services',
)

# Systemd service for bigame-daemon
install_data(
  'bigame-daemon.service',
  install_dir: prefix / 'lib' / 'systemd' / 'system',
)

# Application icon (scalable SVG)
install_data(
  'icons' / app_id + '.svg',
  install_dir: iconsdir / 'scalable' / 'apps',
)
