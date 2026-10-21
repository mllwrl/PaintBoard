#!/bin/sh
# Builds target/macos/PaintBoard-<version>-macos.dmg: PaintBoard.app (a universal binary
# for Apple silicon and Intel Macs) to drag into Applications.
# Runs on macOS with Xcode's command line tools (sips, iconutil, lipo, codesign, hdiutil),
# Rust with both targets (rustup target add aarch64-apple-darwin x86_64-apple-darwin) and
# cargo-about (cargo install cargo-about --locked --features cli). UNIVERSAL=0 builds for
# this Mac's processor only.
# The app is not signed with a Developer ID nor notarized (that needs a paid Apple
# Developer account): only signed ad hoc, so the first start is right-click → Open.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

if [ "${UNIVERSAL:-1}" = 1 ]; then
    targets="aarch64-apple-darwin x86_64-apple-darwin"
else
    targets="$(uname -m | sed 's/arm64/aarch64/')-apple-darwin"
fi
binaries=""
about_targets=""
for t in $targets; do
    cargo build --release -p pb-app --target "$t"
    binaries="$binaries target/$t/release/paintboard"
    about_targets="$about_targets --target $t"
done
# "path+file:///.../crates/pb-app#0.1.0" or "...#pb-app@0.1.0"
version=$(cargo pkgid -p pb-app | sed 's/.*[#@]//')

out="$root/target/macos"
app="$out/PaintBoard.app"
rm -rf "$app" "$out/dmg"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

# shellcheck disable=SC2086 # the lists are meant to split into words
lipo -create -output "$app/Contents/MacOS/paintboard" $binaries
sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# The icon in the sizes macOS uses; the source is 256 px, so that is the largest.
iconset="$out/PaintBoard.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256; do
    sips -z "$size" "$size" crates/pb-app/assets/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    if [ "$double" -le 256 ]; then
        sips -z "$double" "$double" crates/pb-app/assets/icon.png \
            --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    fi
done
iconutil -c icns -o "$app/Contents/Resources/PaintBoard.icns" "$iconset"
rm -rf "$iconset"

# Licenses of everything built into the app, then the font's.
notices="$app/Contents/Resources/THIRD-PARTY.txt"
# shellcheck disable=SC2086
cargo about generate -c packaging/windows/about.toml $about_targets \
    -o "$notices" packaging/windows/about.hbs
{
    printf '\nInter typeface (the app'"'"'s font)\n'
    printf '%s\n\n' '--------------------------------------------------------------------------------'
    cat crates/pb-text/assets/fonts/Inter-OFL.txt
} >> "$notices"

# Ad hoc signature: Apple silicon runs only signed code, and it ties Info.plist (file
# types, microphone text) to the app.
codesign --force --deep --sign - "$app"

# The disk image: the app, a shortcut to Applications to drag it onto, the notices.
mkdir -p "$out/dmg"
cp -R "$app" "$out/dmg/"
ln -s /Applications "$out/dmg/Applications"
cp "$notices" "$out/dmg/THIRD-PARTY.txt"
image="$out/PaintBoard-$version-macos.dmg"
rm -f "$image"
# hdiutil's own guess of the size from -srcfolder is sometimes too small ("No space left
# on device"), so the size is given: the contents and a good margin.
size_mb=$(( $(du -sm "$out/dmg" | cut -f1) * 2 + 20 ))
hdiutil create -volname "PaintBoard $version" -srcfolder "$out/dmg" -size "${size_mb}m" \
    -fs HFS+ -ov -format UDZO "$image"
rm -rf "$out/dmg"
ls -l "$image"
