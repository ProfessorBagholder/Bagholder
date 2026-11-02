# Bagholder's web app in a container: the server built from source, then a slim
# Debian image with Chromium for the sign-in. Data lives in /data, mounted from the host.
# Built from the repository root: docker build -f rust/Dockerfile .
# A release builds it from the binaries its Linux archives carry
# (`--build-arg BINARIES=prebuilt`, the binaries under rust/prebuilt/<arch>/),
# so the image is never a second compile of what the release already built.
ARG BINARIES=build
FROM rust:1-bookworm AS build
# BoringSSL (the browser helper's TLS) builds with cmake and clang; the market
# client links the system OpenSSL
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang libclang-dev pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY rust/rust-toolchain.toml rust/Cargo.toml rust/Cargo.lock ./
COPY rust/crates crates
RUN cargo build --release --locked --bin bagholder --bin bagholder-browser --bin sedar

# The release's own binaries, for the platform being built.
FROM scratch AS prebuilt
ARG TARGETARCH
COPY rust/prebuilt/${TARGETARCH}/ /src/target/release/

FROM ${BINARIES} AS binaries

# The Svelte frontend, built to web/dist and served by the binary.
FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM debian:bookworm-slim
# Chromium for the Wealthsimple sign-in, on a virtual display (Xvfb): the app's page shows
# its window and passes your clicks and keys to it, so nothing is needed on the host but
# Docker. Passkeys need a real browser; sign in with the password and 2FA.
RUN apt-get update && apt-get install -y --no-install-recommends chromium xvfb fonts-liberation ca-certificates libssl3 poppler-utils tzdata \
    && rm -rf /var/lib/apt/lists/*
# the user the app runs as, at home in its data folder
RUN useradd --uid 1000 --user-group --home-dir /data --no-create-home --shell /usr/sbin/nologin bagholder
COPY rust/docker-entrypoint.sh /usr/local/bin/bagholder-entrypoint
RUN chmod +x /usr/local/bin/bagholder-entrypoint

WORKDIR /app
COPY --from=binaries /src/target/release/bagholder /src/target/release/bagholder-browser /src/target/release/sedar ./
COPY --from=web /web/dist ./web/dist

RUN mkdir -p /data && chown bagholder:bagholder /data && chmod 700 /data

# /data holds the database and the login; the server answers every interface of
# the container (compose publishes it on the host's loopback only); no browser
# opens at start, and a release is announced in the header but never installed
# into the container: a new release is a new image, pulled.
ENV BAGHOLDER_HOME=/data \
    BAGHOLDER_PORT=8765 \
    BAGHOLDER_BIND=0.0.0.0 \
    BAGHOLDER_NO_BROWSER=1 \
    BAGHOLDER_NO_UPDATE=1 \
    BAGHOLDER_LOGIN_VIEW=1 \
    BAGHOLDER_CHROME=/usr/bin/chromium \
    DISPLAY=:99 \
    HOME=/data

# Started as root only long enough to take the mounted data folder over for the
# `bagholder` user (uid 1000), whoever made it on the host; the app runs as that user
# and touches nothing but /data and /tmp (bagholder-entrypoint).
VOLUME ["/data"]
EXPOSE 8765
ENTRYPOINT ["bagholder-entrypoint"]
CMD ["/app/bagholder"]
