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
RUN cargo build --release -p pb-server && sh packaging/web/build-web.sh /web

FROM debian:bookworm-slim
# TLS and certificates for link previews (PB_PREVIEWS).
RUN apt-get update && apt-get install -y --no-install-recommends libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/paintboard-server /usr/local/bin/
COPY --from=build /web /web
ENV PB_ADDR=0.0.0.0:8080 PB_DB=/data/paintboard.db PB_WEB=/web PB_BACKUP=/data/backups
VOLUME /data
EXPOSE 8080
CMD ["paintboard-server"]
