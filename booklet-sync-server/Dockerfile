# Build the sync server only (not the QtQuick app) — its dependency graph is
# Qt-free. Build context is the workspace root, so path deps resolve.
FROM rust:1-bookworm AS build
# cmake + a C toolchain are needed by the rustls crypto backend (aws-lc-rs) and
# by zstd-sys, which compile C during the build.
RUN apt-get update && apt-get install -y --no-install-recommends cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release -p booklet-sync-server

# Slim runtime: the binary links glibc + needs CA certs for outbound TLS
# (Stripe/SMTP). No Qt, no OpenSSL (TLS is rustls).
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/booklet-sync-server /usr/local/bin/booklet-sync-server

# Blobs live here; mount a volume so they survive a container replace.
ENV BOOKLET_BLOB_DIR=/data/blobs
VOLUME ["/data"]

# 8080 = sync API (reverse-proxied to the world); 8081 = admin panel (loopback).
EXPOSE 8080 8081

ENTRYPOINT ["booklet-sync-server"]
CMD ["serve"]
