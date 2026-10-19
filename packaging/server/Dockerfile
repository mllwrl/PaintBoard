# PaintBoard server with the web version, so a board's link opens it in a browser.
# Build from the repository root:
#   docker build -f packaging/server/Dockerfile -t paintboard-server .
#   docker run -p 8080:8080 -v paintboard:/data paintboard-server
FROM rust:1-bookworm AS build
# clang: a dependency of the board's CRDT compiles C for wasm32.
RUN apt-get update && apt-get install -y --no-install-recommends clang && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
WORKDIR /src
COPY . .
# wasm-bindgen-cli must match the wasm-bindgen crate in Cargo.lock.
RUN version=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/version = "\(.*\)"/\1/p') \
    && cargo install wasm-bindgen-cli --version "$version" --locked
RUN cargo build --release -p pb-server \
    && cargo build --release --target wasm32-unknown-unknown -p pb-app \
    && mkdir -p /web \
    && wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
        --out-dir /web --out-name paintboard target/wasm32-unknown-unknown/release/paintboard.wasm \
    && cp packaging/web/index.html crates/pb-app/assets/icon.png /web/ \
    && gzip -k -9 /web/paintboard_bg.wasm /web/paintboard.js /web/index.html

FROM debian:bookworm-slim
COPY --from=build /src/target/release/paintboard-server /usr/local/bin/
COPY --from=build /web /web
ENV PB_ADDR=0.0.0.0:8080 PB_DB=/data/paintboard.db PB_WEB=/web PB_BACKUP=/data/backups
VOLUME /data
EXPOSE 8080
CMD ["paintboard-server"]
