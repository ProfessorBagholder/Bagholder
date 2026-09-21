#!/bin/sh
# The virtual display Chromium draws the sign-in window on, then the app as PID 1.
# A restarted container keeps /tmp, and a stale display lock would stop Xvfb.
rm -f /tmp/.X99-lock /tmp/.X11-unix/X99
Xvfb :99 -screen 0 1000x1040x24 -nolisten tcp >/dev/null 2>&1 &
exec "$@"
