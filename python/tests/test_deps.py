"""The invisible dependency provisioner: it puts the private dir on the path, detects
what is missing, and never installs when disabled. No real pip or network."""
from __future__ import annotations

import sys
import unittest

import deps


class ActivateTest(unittest.TestCase):
    def test_activate_puts_the_private_dir_on_sys_path_once(self):
        p = str(deps.libs_dir())
        while p in sys.path:
            sys.path.remove(p)
        deps.activate()
        self.assertEqual(sys.path[0], p)
        deps.activate()
        self.assertEqual(sys.path.count(p), 1, "idempotent")


class MissingTest(unittest.TestCase):
    def setUp(self):
        self._req = deps._REQUIRE

    def tearDown(self):
        deps._REQUIRE = self._req

    def test_present_and_absent_are_told_apart(self):
        deps._REQUIRE = {"json-x==1": "json", "no_such_pkg_xyz==1": "no_such_pkg_xyz"}
        self.assertEqual(deps._missing(), ["no_such_pkg_xyz==1"])


class DisabledTest(unittest.TestCase):
    def setUp(self):
        self._d, self._s = deps.DISABLED, deps._started
        deps.DISABLED, deps._started = True, False

    def tearDown(self):
        deps.DISABLED, deps._started = self._d, self._s

    def test_provision_is_a_noop_when_disabled(self):
        deps.provision()
        self.assertFalse(deps._started, "disabled: no install thread is started")


if __name__ == "__main__":
    unittest.main()
