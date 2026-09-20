# Bagholder's web app in a container: the server built from source, then a slim
# Debian image with Chromium for the sign-in. Data lives in /data, mounted from the host.
# Built from the repository root: docker build -f rust/Dockerfile .
FROM rust:1-bookworm AS build
# BoringSSL (the browser helper's TLS) builds with cmake and clang; the market
# client links the system OpenSSL
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang libclang-dev pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY rust/rust-toolchain.toml rust/Cargo.toml rust/Cargo.lock ./
COPY rust/crates crates
RUN cargo build --release --locked --bin bagholder --bin bagholder-browser --bin disclosures-mcp --bin sedar

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
RUN apt-get update && apt-get install -y --no-install-recommends chromium xvfb fonts-liberation ca-certificates libssl3 poppler-utils \
    && rm -rf /var/lib/apt/lists/*
COPY rust/docker-entrypoint.sh /usr/local/bin/bagholder-entrypoint
RUN chmod +x /usr/local/bin/bagholder-entrypoint

WORKDIR /app
COPY --from=build /src/target/release/bagholder /src/target/release/bagholder-browser /src/target/release/disclosures-mcp /src/target/release/sedar ./
COPY ledger.html lightweight-charts.js favicon.png ./
COPY --from=web /web/dist ./web/dist

RUN mkdir -p /data

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
    DISPLAY=:99

# root inside the container, so a data folder mounted from any host user is writable
# as it is; the process touches nothing but /data, and the port is loopback-only.
VOLUME ["/data"]
EXPOSE 8765
ENTRYPOINT ["bagholder-entrypoint"]
CMD ["/app/bagholder"]
