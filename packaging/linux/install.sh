#!/bin/sh
# Installs PaintBoard for the current user from the unpacked tarball: the program into
# ~/.local/bin, a menu entry, the icon and the .pboard/.pbplugin file associations.
# ./install.sh --uninstall removes them again (boards and settings stay).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin"

refresh() {
    command -v update-mime-database >/dev/null && update-mime-database "$data/mime" || true
    command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin/paintboard" "$data/applications/paintboard.desktop" \
        "$data/mime/packages/paintboard.xml" "$data/icons/hicolor/256x256/apps/paintboard.png"
    rm -rf "$data/doc/paintboard"
    refresh
    echo "PaintBoard removed."
    exit 0
fi

mkdir -p "$bin" "$data/applications" "$data/mime/packages" \
    "$data/icons/hicolor/256x256/apps" "$data/doc/paintboard"
install -m 755 "$here/usr/bin/paintboard" "$bin/paintboard"
# The menu entry runs the program by its full path: ~/.local/bin is not always on PATH.
sed "s,^Exec=paintboard,Exec=$bin/paintboard," "$here/usr/share/applications/paintboard.desktop" \
    > "$data/applications/paintboard.desktop"
install -m 644 "$here/usr/share/mime/packages/paintboard.xml" "$data/mime/packages/"
install -m 644 "$here/usr/share/icons/hicolor/256x256/apps/paintboard.png" \
    "$data/icons/hicolor/256x256/apps/"
install -m 644 "$here/usr/share/doc/paintboard/THIRD-PARTY.txt" "$data/doc/paintboard/"
refresh
command -v xdg-mime >/dev/null && \
    xdg-mime default paintboard.desktop application/x-paintboard application/x-paintboard-plugin || true
echo "PaintBoard installed: $bin/paintboard (and in the applications menu)."
