#!/bin/sh
# The virtual display Chromium draws the sign-in window on, then the app as PID 1.
Xvfb :99 -screen 0 1000x1040x24 -nolisten tcp >/dev/null 2>&1 &
exec "$@"
