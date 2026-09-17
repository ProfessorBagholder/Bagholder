# Bagholder's web app in a container: the Go binary built from this checkout, on a slim
# Debian image with Chromium for the sign-in. Data lives in /data, mounted from the host.
FROM golang:1.24-bookworm AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/bagholder ./cmd/bagholder

# The runtime base is the one the Python image used, not a bare debian:bookworm-slim.
# Two independent rewrites (this one and a Rust one) replaced that base and both hit a
# sign-in that stalls where the Python image's does not, so the base is held to the
# known-good one until the difference is understood. It costs an interpreter this
# binary never runs.
FROM python:3.12-slim-bookworm

# Chromium for the Wealthsimple sign-in, on a virtual display (Xvfb): the app's page shows
# its window and passes your clicks and keys to it, so nothing is needed on the host but
# Docker. Passkeys need a real browser; sign in with the password and 2FA.
RUN apt-get update && apt-get install -y --no-install-recommends chromium xvfb fonts-liberation ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY docker-entrypoint.sh /usr/local/bin/bagholder-entrypoint
RUN chmod +x /usr/local/bin/bagholder-entrypoint

WORKDIR /app
COPY --from=build /out/bagholder /app/bagholder

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
