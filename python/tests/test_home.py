"""One data folder per build: the marker that names the build a folder belongs to."""
import os
import tempfile
import unittest
from pathlib import Path

import store


class HomeBuildMarker(unittest.TestCase):
    def test_a_folder_with_a_foreign_marker_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "build").write_text("rust\n", encoding="utf-8")
            refusal = store.claim_home(d)
            self.assertEqual(
                refusal,
                "%s belongs to the rust build of Bagholder; run that build, or point this one "
                "elsewhere with BAGHOLDER_HOME=<another folder>" % d,
            )
            self.assertEqual((Path(d) / "build").read_text(encoding="utf-8"), "rust\n")

    def test_a_folder_without_a_marker_is_adopted_and_marked(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "bagholder.db").write_bytes(b"")
            self.assertIsNone(store.claim_home(d))
            self.assertEqual((Path(d) / "build").read_text(encoding="utf-8").strip(), "python")

    def test_the_marker_is_not_rewritten_when_it_already_names_this_build(self):
        with tempfile.TemporaryDirectory() as d:
            marker = Path(d) / "build"
            marker.write_text("python\n", encoding="utf-8")
            before = marker.stat().st_mtime_ns
            os.utime(marker, ns=(before - 10_000_000_000, before - 10_000_000_000))
            stamp = marker.stat().st_mtime_ns
            self.assertIsNone(store.claim_home(d))
            self.assertEqual(marker.stat().st_mtime_ns, stamp)

    def test_bagholder_home_decides_where_the_folder_is(self):
        with tempfile.TemporaryDirectory() as d:
            elsewhere = str(Path(d) / "elsewhere")
            store.set_home(None)
            previous = os.environ.get("BAGHOLDER_HOME")
            os.environ["BAGHOLDER_HOME"] = elsewhere
            try:
                self.assertEqual(store.home(), Path(elsewhere))
                self.assertIsNone(store.claim_home(store.home()))
            finally:
                if previous is None:
                    del os.environ["BAGHOLDER_HOME"]
                else:
                    os.environ["BAGHOLDER_HOME"] = previous
            self.assertEqual((Path(elsewhere) / "build").read_text(encoding="utf-8").strip(), "python")


if __name__ == "__main__":
    unittest.main()
