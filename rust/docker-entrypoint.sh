#!/bin/sh
# Started as root, the container takes the data folder over for its own user, then
# runs as that user from here on: only what another owns changes hands, so a data
# folder already taken over isn't rewritten at every start. Started as another user
# (`user:` in compose), it runs as that one, as it is.
set -e
if [ "$(id -u)" = 0 ]; then
  find /data \! -user bagholder -exec chown -h bagholder:bagholder {} +
  exec setpriv --reuid=bagholder --regid=bagholder --init-groups --inh-caps=-all "$0" "$@"
fi
# The virtual display Chromium draws the sign-in window on, then the app as PID 1.
# A restarted container keeps /tmp, and a stale display lock would stop Xvfb.
rm -f /tmp/.X99-lock /tmp/.X11-unix/X99
Xvfb :99 -screen 0 1000x1040x24 -nolisten tcp >/dev/null 2>&1 &
exec "$@"
