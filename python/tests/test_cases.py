"""The shared model cases in tests/cases, run through the Python model: the
same files a Swift or Kotlin implementation runs through its own model, so the
three cannot disagree without a failing test. Regenerate with
tests/make_cases.py after an intended model change and review the diff."""
import glob
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
# the shared model cases, at the repository root beside every implementation
SHARED = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "tests")
import make_cases  # noqa: E402


class FixtureTest(unittest.TestCase):
    def test_every_case_matches(self):
        paths = sorted(glob.glob(os.path.join(SHARED, "cases", "*.json")))
        self.assertTrue(paths, "no cases found")
        for path in paths:
            with open(path) as f:
                doc = json.load(f)
            got = make_cases.expect_from(doc["snapshot"], doc["market"], doc["today"], doc["filters"], doc.get("journal"))
            self.assertEqual(got, doc["expect"], os.path.basename(path))

    def test_cases_are_current(self):
        """The files on disk are what the generator writes now: a model change means regenerating them on purpose."""
        for name, case in make_cases.CASES.items():
            with open(os.path.join(SHARED, "cases", name + ".json")) as f:
                self.assertEqual(json.load(f)["expect"], make_cases.expect(case), name)


if __name__ == "__main__":
    unittest.main()
