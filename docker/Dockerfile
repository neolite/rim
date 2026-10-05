FROM alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6
RUN apk add --no-cache \
      build-base linux-headers bc flex bison perl openssl-dev elfutils-dev \
      rust cargo \
      erofs-utils mtools e2fsprogs sgdisk ukify systemd-efistub \
      curl tar xz bzip2 coreutils findutils python3 file
# Явная цель: с ней RUSTFLAGS (crt-static) применяются только к целевым бинарникам,
# а не к proc-macro вроде serde_derive, которые musl статикой собрать не может.
ENV CARGO_BUILD_TARGET=aarch64-alpine-linux-musl \
    CARGO_TARGET_DIR=/rim/out/cargo-linux \
    CARGO_HOME=/cargo \
    RUSTFLAGS="-C target-feature=+crt-static" \
    SOURCE_DATE_EPOCH=1767225600
WORKDIR /rim
