"""The suite's own guards. Each of these closed a hole a test run fell through before: fixtures
written into the person's live database, tests that passed or failed on whether a public site
answered, and an exception on a background thread leaving its test green."""
from __future__ import annotations

import os
import socket
import threading
import unittest
from pathlib import Path
from urllib.request import urlopen

import deps
import store
import tests


class NetworkGuardTest(unittest.TestCase):
    def test_a_connection_to_anywhere_but_this_machine_is_refused(self):
        with self.assertRaises(tests.WentOutside):
            socket.create_connection(("example.com", 80), 1)
        with self.assertRaises(tests.WentOutside):
            socket.socket().connect(("93.184.216.34", 80))

    def test_the_refusal_reaches_a_reader_that_goes_through_urllib(self):
        # what the app's own readers do. The refusal is not an OSError, so urllib does not fold it
        # into a URLError a reader's own `except` would swallow: it comes out of the call as it is.
        with self.assertRaises(tests.WentOutside) as caught:
            urlopen("https://example.com/", timeout=1)
        self.assertIn("test tried to reach", str(caught.exception))

    def test_this_machine_is_still_reachable(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        try:
            socket.create_connection(listener.getsockname(), 1).close()
        finally:
            listener.close()


class HomeGuardTest(unittest.TestCase):
    def test_the_run_has_a_home_of_its_own(self):
        self.assertEqual(os.environ.get("BAGHOLDER_HOME"), tests.SUITE_HOME)
        self.assertTrue(Path(tests.SUITE_HOME).is_dir())
        self.assertNotEqual(Path(store.home()).resolve(), (Path.home() / ".bagholder").resolve())

    def test_the_real_folder_is_refused_to_a_test(self):
        with self.assertRaises(RuntimeError):
            store.guard_home(Path.home() / ".bagholder")

    def test_the_private_package_dir_lands_in_the_run_s_home(self):
        self.assertEqual(Path(deps.libs_dir()).parent.resolve(), Path(tests.SUITE_HOME).resolve())

    def test_a_test_that_clears_the_home_gets_it_back(self):
        os.environ.pop("BAGHOLDER_HOME", None)      # what several tearDowns do
        tests._restore_home()
        self.assertEqual(os.environ.get("BAGHOLDER_HOME"), tests.SUITE_HOME)


class ThreadGuardTest(unittest.TestCase):
    def test_what_a_background_thread_raises_is_recorded(self):
        tests.thread_errors()                        # start from clean
        t = threading.Thread(target=lambda: (_ for _ in ()).throw(ValueError("from a thread")), name="bagholder-test-job")
        t.start()
        t.join()
        errs = tests.thread_errors()
        self.assertEqual(len(errs), 1)
        self.assertIn("from a thread", errs[0])


if __name__ == "__main__":
    unittest.main()
