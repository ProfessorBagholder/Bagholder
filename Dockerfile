# Bagholder's web app in a container: Python's own slim Debian image, Chromium for
# the sign-in, the app's files, no build step. Data lives in /data, mounted from the host.
FROM python:3.12-slim-bookworm

# Chromium for the Wealthsimple sign-in, on a virtual display (Xvfb): the app's page shows
# its window and passes your clicks and keys to it, so nothing is needed on the host but
# Docker. Passkeys need a real browser; sign in with the password and 2FA.
RUN apt-get update && apt-get install -y --no-install-recommends chromium xvfb fonts-liberation ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY docker-entrypoint.sh /usr/local/bin/bagholder-entrypoint
RUN chmod +x /usr/local/bin/bagholder-entrypoint

WORKDIR /app
# Every module, not a list of them: the app grew four modules after this file
# was written and each one was missing from the image, which crashed on the
# first import. The repository's Python files are the app's own; the tests, the
# phone apps and the docs are kept out by .dockerignore.
COPY *.py ledger.html lightweight-charts.js favicon.png ./

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
    DISPLAY=:99 \
    PYTHONUNBUFFERED=1

# root inside the container, so a data folder mounted from any host user is writable
# as it is; the process touches nothing but /data, and the port is loopback-only.
VOLUME ["/data"]
EXPOSE 8765
ENTRYPOINT ["bagholder-entrypoint"]
CMD ["python3", "bagholder.py"]
