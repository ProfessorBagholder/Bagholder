"""The page's script must parse: a page that does not is a blank app."""
import os
import re
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PAGE = os.path.join(os.path.dirname(os.path.dirname(HERE)), "ledger.html")


class PageScriptTest(unittest.TestCase):
    def test_every_script_on_the_page_parses(self):
        if not shutil.which("node"):
            # a skip here is the page going unchecked, and a page whose script does not parse is a
            # blank app: where the run declares its tools (CI does), the missing tool is a failure
            if os.environ.get("CI") or os.environ.get("BAGHOLDER_REQUIRE_TOOLS"):
                self.fail("node is missing, so the page's script was never parsed")
            self.skipTest("node is needed to parse the page's script")
        html = open(PAGE, encoding="utf-8").read()
        scripts = re.findall(r"<script>(.*?)</script>", html, re.S)
        self.assertTrue(scripts, "the page carries its script inline")
        for i, js in enumerate(scripts):
            with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as f:
                f.write(js)
            try:
                r = subprocess.run(["node", "--check", f.name], capture_output=True, text=True)
            finally:
                os.unlink(f.name)
            self.assertEqual(r.returncode, 0, "script %d does not parse:\n%s" % (i, r.stderr[:2000]))

    def test_no_title_attribute_anywhere_on_the_page(self):
        html = open(PAGE, encoding="utf-8").read()
        self.assertEqual(re.findall(r" title=\\?[\"']", html), [], "nothing on the page gets a browser tooltip")


if __name__ == "__main__":
    unittest.main()
