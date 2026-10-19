# PaintBoard server in a small image. Build from the repository root:
#   docker build -f packaging/server/Dockerfile -t paintboard-server .
#   docker run -p 8080:8080 -v paintboard:/data paintboard-server
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p pb-server

FROM debian:bookworm-slim
COPY --from=build /src/target/release/paintboard-server /usr/local/bin/
ENV PB_ADDR=0.0.0.0:8080 PB_DB=/data/paintboard.db
VOLUME /data
EXPOSE 8080
CMD ["paintboard-server"]
