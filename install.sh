#!/usr/bin/env bash
# One-time setup: builds stepshot and installs the binary, app icon and .desktop.
#
# The .desktop file serves two purposes:
#  1) menu entry (stepshot shows up as an app, click to launch → tray icon)
#  2) authorization for KWin's ScreenShot2 (X-KDE-DBUS-Restricted-Interfaces);
#     KWin matches the resolved executable path against Exec= → copy the binary,
#     don't symlink it.
set -euo pipefail
cd "$(dirname "$0")"

BINDIR="${HOME}/.local/bin"
APPDIR="${HOME}/.local/share/applications"
ICONDIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
BIN="${BINDIR}/stepshot"
DESKTOP="${APPDIR}/org.stepshot.Stepshot.desktop"

echo ">> Building release …"
cargo build --release

echo ">> Installing binary  → ${BIN}"
mkdir -p "${BINDIR}" "${APPDIR}" "${ICONDIR}"
install -m755 target/release/stepshot "${BIN}"

echo ">> Installing icon    → ${ICONDIR}/stepshot.svg"
install -m644 assets/stepshot.svg "${ICONDIR}/stepshot.svg"

echo ">> Writing .desktop   → ${DESKTOP}"
cat > "${DESKTOP}" <<EOF
[Desktop Entry]
Type=Application
Name=stepshot
GenericName=Step Recorder
Comment=Documents your clicks as an illustrated step-by-step guide
Exec=${BIN}
Icon=stepshot
Terminal=false
Categories=Utility;Graphics;
Keywords=screenshot;steps;recorder;tutorial;documentation;
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
EOF

echo ">> Refreshing caches"
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true

echo ""
echo "Done. stepshot is now in your application menu (or run 'stepshot')."
echo "It shows up as a camera icon in the system tray — start/stop recording there."
