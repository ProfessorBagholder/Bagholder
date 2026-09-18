"""Where the Python app keeps its data: ~/.bagholder, or wherever BAGHOLDER_HOME says."""
import os
import tempfile
import unittest
from pathlib import Path

import store


class Home(unittest.TestCase):
    def setUp(self):
        store.set_home(None)
        self._previous = os.environ.get("BAGHOLDER_HOME")

    def tearDown(self):
        if self._previous is None:
            os.environ.pop("BAGHOLDER_HOME", None)
        else:
            os.environ["BAGHOLDER_HOME"] = self._previous
        store.set_home(None)

    def test_the_default_folder_is_dot_bagholder(self):
        os.environ.pop("BAGHOLDER_HOME", None)
        self.assertEqual(store.home(), Path.home() / ".bagholder")

    def test_bagholder_home_decides_where_the_folder_is(self):
        with tempfile.TemporaryDirectory() as d:
            elsewhere = str(Path(d) / "elsewhere")
            os.environ["BAGHOLDER_HOME"] = elsewhere
            self.assertEqual(store.home(), Path(elsewhere))
            self.assertEqual(store.db_path(), Path(elsewhere) / "bagholder.db")


if __name__ == "__main__":
    unittest.main()
