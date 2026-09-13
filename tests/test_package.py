"""What a published copy of the app must contain.

The image once shipped a hand-written list of files. The app grew four modules
after that list was written, every image built for three days crashed on its
first import, and nothing here noticed. These tests fail if a published copy
would be missing anything the app imports.
"""
from __future__ import annotations

import ast
import os
import re
import subprocess
import sys
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def local_modules():
    return {f[:-3] for f in os.listdir(ROOT) if f.endswith(".py")}


def imports_of(entry="bagholder"):
    """Every module in the repository that `entry` needs, directly or not."""
    local, need, seen = local_modules(), set(), set()

    def walk(name):
        if name in seen:
            return
        seen.add(name)
        path = os.path.join(ROOT, name + ".py")
        if not os.path.exists(path):
            return
        for node in ast.walk(ast.parse(open(path).read())):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name in local:
                        need.add(alias.name)
                        walk(alias.name)
            elif isinstance(node, ast.ImportFrom) and node.module in local:
                need.add(node.module)
                walk(node.module)

    walk(entry)
    return need


class DockerImageTest(unittest.TestCase):
    def copied_files(self):
        """What the Dockerfile puts in the image's working directory."""
        out = []
        for line in open(os.path.join(ROOT, "Dockerfile")):
            m = re.match(r"^COPY\s+(.*?)\s+\./\s*$", line.strip())
            if m:
                out.extend(m.group(1).split())
        return out

    def test_the_image_carries_every_module_the_app_imports(self):
        copied = self.copied_files()
        self.assertTrue(copied, "the Dockerfile copies the app in")
        takes_all_python = any(p == "*.py" for p in copied)
        named = {p[:-3] for p in copied if p.endswith(".py")}
        missing = sorted(m for m in imports_of() if not takes_all_python and m not in named)
        self.assertEqual(missing, [], "modules the app imports but the image would not have")

    def test_the_image_carries_the_page_and_its_chart_library(self):
        copied = self.copied_files()
        for needed in ("ledger.html", "lightweight-charts.js", "favicon.png"):
            self.assertIn(needed, copied, "%s is served by the app" % needed)

    def test_nothing_the_image_needs_is_kept_out_of_it(self):
        ignored = [l.strip() for l in open(os.path.join(ROOT, ".dockerignore")) if l.strip() and not l.startswith("#")]
        for module in sorted(imports_of()):
            self.assertNotIn(module + ".py", ignored)
            self.assertNotIn("*.py", ignored)


class ReleaseArchiveTest(unittest.TestCase):
    def test_the_release_archive_carries_every_module_the_app_imports(self):
        """`git archive` ships what is tracked; an untracked module would be missing."""
        try:
            tracked = subprocess.run(["git", "-C", ROOT, "ls-files"], capture_output=True, text=True, check=True).stdout.split()
        except (OSError, subprocess.CalledProcessError):   # pragma: no cover - not a checkout
            self.skipTest("not a git checkout")
        missing = sorted(m for m in imports_of() if m + ".py" not in tracked)
        self.assertEqual(missing, [], "modules the app imports that the release archive would not carry")
        for needed in ("ledger.html", "lightweight-charts.js", "favicon.png"):
            self.assertIn(needed, tracked)


class StartsFromItsOwnFilesTest(unittest.TestCase):
    def test_the_app_imports_cleanly_from_a_copy_of_what_ships(self):
        """The real failure, reproduced: a copy holding only the published files
        is asked to import the app. A missing module raises here, not on a user's
        machine."""
        import shutil
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            for name in sorted(local_modules()):
                shutil.copy(os.path.join(ROOT, name + ".py"), tmp)
            for name in ("ledger.html", "lightweight-charts.js", "favicon.png"):
                src = os.path.join(ROOT, name)
                if os.path.exists(src):
                    shutil.copy(src, tmp)
            env = dict(os.environ, BAGHOLDER_HOME=os.path.join(tmp, "data"), BAGHOLDER_NO_BROWSER="1", BAGHOLDER_NO_UPDATE="1")
            r = subprocess.run([sys.executable, "-c", "import bagholder"], cwd=tmp, env=env, capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr[-600:])


if __name__ == "__main__":   # pragma: no cover
    unittest.main()
