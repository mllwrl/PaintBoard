#!/bin/sh
# Builds the web version into the folder given (target/web by default), like
# build-web.ps1 does on Windows; used by the server's Docker image and by CI.
# Needs the wasm32 target, clang, and wasm-bindgen-cli of the version in Cargo.lock.
set -eu
out="${1:-target/web}"
cargo build --release --target wasm32-unknown-unknown -p pb-app
mkdir -p "$out"
# Function names only help reading stack traces; without them the file is ~10% smaller.
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
    --out-dir "$out" --out-name paintboard target/wasm32-unknown-unknown/release/paintboard.wasm
cp packaging/web/index.html crates/pb-app/assets/icon.png "$out/"
# The service worker keeps this build: its version is the build's own, so browsers notice
# a new one.
version=$(sha256sum "$out/paintboard_bg.wasm" | cut -c1-16)
sed "s/__VERSION__/$version/" packaging/web/sw.js > "$out/sw.js"
# Compressed copies for servers that send them as they are (paintboard-server does).
gzip -k -f -9 "$out/paintboard_bg.wasm" "$out/paintboard.js" "$out/index.html"
