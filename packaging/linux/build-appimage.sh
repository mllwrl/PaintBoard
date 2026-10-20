#!/bin/sh
# Builds the Linux packages into target/linux:
#   PaintBoard-<version>-x86_64.AppImage  one file that runs on most distributions;
#   PaintBoard-<version>-linux-x86_64.tar.gz  the same files with install.sh, which puts
#     the program into ~/.local (menu entry, icon, .pboard/.pbplugin file associations).
# Needs: Rust, cargo-about (cargo install cargo-about --locked --features cli), the
# development files of ALSA and OpenSSL and pkg-config (Debian/Ubuntu:
# sudo apt install pkg-config libasound2-dev libssl-dev), and curl or wget to fetch
# appimagetool (MIT; set APPIMAGETOOL to use one already at hand). The program needs the
# glibc of the system it was built on or newer: build on the oldest one to support.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
arch=$(uname -m)

cargo build --release -p pb-app
# "path+file:///.../crates/pb-app#0.1.0" or "...#pb-app@0.1.0"
version=$(cargo pkgid -p pb-app | sed 's/.*[#@]//')

out="$root/target/linux"
app="$out/PaintBoard.AppDir"
rm -rf "$app"
mkdir -p "$app/usr/bin" "$app/usr/share/applications" "$app/usr/share/mime/packages" \
    "$app/usr/share/icons/hicolor/256x256/apps" "$app/usr/share/doc/paintboard"

install -m 755 target/release/paintboard "$app/usr/bin/paintboard"
install -m 644 packaging/linux/paintboard.desktop "$app/usr/share/applications/"
install -m 644 packaging/linux/paintboard.xml "$app/usr/share/mime/packages/"
install -m 644 crates/pb-app/assets/icon.png "$app/usr/share/icons/hicolor/256x256/apps/paintboard.png"

# Licenses of everything built into the app, then the font's.
notices="$app/usr/share/doc/paintboard/THIRD-PARTY.txt"
cargo about generate -c packaging/windows/about.toml --target "$arch-unknown-linux-gnu" \
    -o "$notices" packaging/windows/about.hbs
{
    printf '\nInter typeface (the app'"'"'s font)\n'
    printf '%s\n\n' '--------------------------------------------------------------------------------'
    cat crates/pb-text/assets/fonts/Inter-OFL.txt
} >> "$notices"

# What AppImage tools look for at the top of the AppDir.
cp packaging/linux/paintboard.desktop "$app/paintboard.desktop"
cp crates/pb-app/assets/icon.png "$app/paintboard.png"
cat > "$app/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/paintboard" "$@"
EOF
chmod 755 "$app/AppRun"

# The tarball: the same tree with an installer for the current user.
install -m 755 packaging/linux/install.sh "$app/install.sh"
cp "$notices" "$app/THIRD-PARTY.txt"
tarball="$out/PaintBoard-$version-linux-$arch.tar.gz"
tar -czf "$tarball" -C "$out" --transform "s,^PaintBoard.AppDir,PaintBoard-$version," \
    PaintBoard.AppDir
rm "$app/install.sh" "$app/THIRD-PARTY.txt"

# The AppImage.
tool="${APPIMAGETOOL:-}"
if [ -z "$tool" ]; then
    tool="$out/appimagetool-$arch.AppImage"
    if [ ! -x "$tool" ]; then
        url="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
        if command -v curl >/dev/null; then curl -fsSL -o "$tool" "$url"; else wget -q -O "$tool" "$url"; fi
        chmod 755 "$tool"
    fi
fi
image="$out/PaintBoard-$version-$arch.AppImage"
# Without FUSE (containers, CI) the tool unpacks itself instead of mounting.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tool" --no-appstream "$app" "$image"

ls -l "$image" "$tarball"
