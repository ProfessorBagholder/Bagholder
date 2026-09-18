"""What holds for every test, installed once when the suite is discovered.

A test run must not touch anything outside itself. Three things have gone wrong here before and
each is closed at the root rather than per test: a suite wrote its fixtures into the person's live
database; tests read live sources, so the suite passed or failed on whether a public site answered
and on how fast it did; and an exception in a background thread left its test green because the
thread was not the one being watched.
"""
from __future__ import annotations

import atexit
import os
import shutil
import socket
import sys
import tempfile
import threading
import unittest

# The app reads these at import. A test run never installs packages and never opens a browser,
# whatever the machine running it is set up to do. `BAGHOLDER_DRY_ORDERS` is deliberately NOT set:
# the app ships with orders live and two tests assert that default, and nothing can reach a broker
# anyway because the network guard below refuses it.
os.environ.setdefault("BAGHOLDER_NO_DEPS", "1")
os.environ.setdefault("BAGHOLDER_NO_BROWSER", "1")

# A home of the run's own, so a test that forgets to make one writes here and not into the person's
# `~/.bagholder`. Importing the app is enough to create files (deps puts a package dir under the
# home), so this is set before anything is imported, and put back after every test in case one
# cleared it. `store.guard_home` still refuses the real folder if both of these are ever undone.
SUITE_HOME = tempfile.mkdtemp(prefix="bagholder-tests-")
os.environ.setdefault("BAGHOLDER_HOME", SUITE_HOME)
atexit.register(shutil.rmtree, SUITE_HOME, True)

# One escape hatch, for a session deliberately probing a live source by hand; CI never sets it.
LIVE = bool(os.environ.get("BAGHOLDER_TEST_NET"))
_LOOPBACK = ("127.0.0.1", "::1", "localhost", "0.0.0.0", "")


class WentOutside(AssertionError):
    """A test tried to reach the network."""


def _local(address):
    host = address[0] if isinstance(address, tuple) else address
    return not isinstance(host, str) or host in _LOOPBACK or host.startswith("127.")


def _guard_network():
    """Refuse every connection a test makes to anywhere but this machine. The failure names the
    test, so a source that a test quietly depended on is a red line rather than a slow suite."""
    if LIVE:
        return
    connect, create = socket.socket.connect, socket.create_connection

    def refuse(where):
        return WentOutside(
            "a test tried to reach %s. Tests read fixtures, not the internet: stub the reader "
            "(mock.patch.object on the module's fetch), or set BAGHOLDER_TEST_NET=1 to probe a "
            "live source by hand." % (where,))

    def guarded_connect(self, address, *a, **k):
        if not _local(address):
            raise refuse(address)
        return connect(self, address, *a, **k)

    def guarded_create(address, *a, **k):
        if not _local(address):
            raise refuse(address)
        return create(address, *a, **k)

    socket.socket.connect = guarded_connect
    socket.create_connection = guarded_create


_thread_errors = []


def _watch_threads():
    """An exception in a background thread fails the test it happened in. The app runs its work on
    threads, so without this a broken loop is invisible to the suite."""
    def hook(args):
        if args.exc_type is SystemExit:
            return
        _thread_errors.append("%s in %s: %s" % (args.exc_type.__name__, getattr(args.thread, "name", "?"), args.exc_value))
        sys.__stderr__.write("thread %s raised %s: %s\n" % (getattr(args.thread, "name", "?"), args.exc_type.__name__, args.exc_value))
    threading.excepthook = hook


def thread_errors(clear=True):
    """What background threads raised since the last check."""
    out = list(_thread_errors)
    if clear:
        _thread_errors.clear()
    return out


JOIN_SECONDS = 1.0        # how long a test waits for a job it started before it is judged


def _jobs():
    """The app's background jobs running right now: a kick or a sweep, not the endless loops a
    started server owns (those run until the process ends and are nobody's test to wait for)."""
    me = threading.current_thread()
    return [t for t in threading.enumerate()
            if t is not me and t.name.startswith("bagholder-") and not t.name.endswith("-loop")]


def _settle(before):
    """Wait only for the jobs this test started, so its own thread's failure is attributed to it
    rather than to whichever test runs next."""
    for t in _jobs():
        if t not in before:
            t.join(JOIN_SECONDS)


def _restore_home():
    """Put the run's own home back after a test that cleared it, so the next test cannot fall
    through to the person's own folder."""
    if os.environ.get("BAGHOLDER_HOME") != SUITE_HOME:
        os.environ["BAGHOLDER_HOME"] = SUITE_HOME
    store = sys.modules.get("store")
    if store is not None:
        store.set_home(None)


def _fail_on_thread_errors():
    """Fold what the app's threads raised into the result of the test they happened in."""
    run = unittest.TestCase.run

    def guarded(self, result=None):
        thread_errors()
        before = _jobs()
        out = run(self, result)
        _settle(before)
        _restore_home()
        errs = thread_errors()
        if errs and result is not None and result.wasSuccessful():
            result.addFailure(self, (AssertionError, AssertionError("a background thread raised: " + "; ".join(errs)), None))
        return out

    unittest.TestCase.run = guarded


_guard_network()
_watch_threads()
_fail_on_thread_errors()
