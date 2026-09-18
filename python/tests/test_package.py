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

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))   # python/
REPO = os.path.dirname(ROOT)                                          # the repository: the page, its assets, the other ports
ASSETS = ("ledger.html", "lightweight-charts.js", "favicon.png")


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
        """What python/Dockerfile puts in the image's working directory, as paths
        from the repository root (its build context)."""
        out = []
        for line in open(os.path.join(ROOT, "Dockerfile")):
            m = re.match(r"^COPY\s+(.*?)\s+\./\s*$", line.strip())
            if m:
                out.extend(m.group(1).split())
        return out

    def test_the_image_carries_every_module_the_app_imports(self):
        copied = self.copied_files()
        self.assertTrue(copied, "the Dockerfile copies the app in")
        takes_all_python = any(p == "python/*.py" for p in copied)
        named = {p[len("python/"):-3] for p in copied if p.startswith("python/") and p.endswith(".py")}
        missing = sorted(m for m in imports_of() if not takes_all_python and m not in named)
        self.assertEqual(missing, [], "modules the app imports but the image would not have")

    def test_the_image_carries_the_page_and_its_chart_library(self):
        copied = self.copied_files()
        for needed in ASSETS:
            self.assertIn(needed, copied, "%s is served by the app" % needed)
            self.assertTrue(os.path.isfile(os.path.join(REPO, needed)), "%s at the repository root, the build context" % needed)

    def test_nothing_the_image_needs_is_kept_out_of_it(self):
        ignored = [l.strip() for l in open(os.path.join(REPO, ".dockerignore")) if l.strip() and not l.startswith("#")]
        for module in sorted(imports_of()):
            self.assertNotIn("python/" + module + ".py", ignored)
            self.assertNotIn("python/*.py", ignored)
        for needed in ASSETS + ("python", "python/requirements.txt", "python/docker-entrypoint.sh"):
            self.assertNotIn(needed, ignored)


class ReleaseArchiveTest(unittest.TestCase):
    def test_the_release_archive_carries_every_module_the_app_imports(self):
        """The archive is built from what is tracked; an untracked module would be missing."""
        try:
            tracked = subprocess.run(["git", "-C", REPO, "ls-files"], capture_output=True, text=True, check=True).stdout.split()
        except (OSError, subprocess.CalledProcessError):   # pragma: no cover - not a checkout
            self.skipTest("not a git checkout")
        missing = sorted(m for m in imports_of() if "python/" + m + ".py" not in tracked)
        self.assertEqual(missing, [], "modules the app imports that the release archive would not carry")
        for needed in ASSETS:
            self.assertIn(needed, tracked)

    def test_the_web_archive_is_flat_and_the_updater_installs_it(self):
        """The archive every installed copy's updater downloads: its name, a flat
        layout, and the updater's own extract and check accept it; the extracted
        copy starts from its own files."""
        import shutil
        import tempfile
        sys.path.insert(0, os.path.join(ROOT, "tools"))
        import web_archive
        import bagholder
        from pathlib import Path
        if not os.path.isdir(os.path.join(REPO, ".git")) and not os.path.isfile(os.path.join(REPO, ".git")):
            self.skipTest("not a git checkout")
        with tempfile.TemporaryDirectory() as tmp:
            name, written = web_archive.build("v9.9.9", ref="WORKTREE", out=tmp)
            self.assertEqual(os.path.basename(name), "bagholder-v9.9.9-web.zip")
            self.assertTrue(os.path.isfile(name + ".sha256"))
            want = open(name + ".sha256").read().split()
            import hashlib
            self.assertEqual(want, [hashlib.sha256(open(name, "rb").read()).hexdigest(), "bagholder-v9.9.9-web.zip"])
            self.assertEqual(bagholder.release_assets({"tag_name": "v9.9.9", "assets": [
                {"name": "bagholder-v9.9.9-web.zip", "browser_download_url": "z"},
                {"name": "bagholder-v9.9.9-web.zip.sha256", "browser_download_url": "s"}]}), {"zip": "z", "sha": "s"})
            staging = Path(tmp) / "staging"
            staging.mkdir()
            names = bagholder._extract_release(name, staging)
            bagholder._check_python(staging, names)
            for needed in ("bagholder.py", "requirements.txt") + ASSETS + tuple(m + ".py" for m in imports_of()):
                self.assertIn(needed, names)
            self.assertFalse([n for n in names if n.startswith(("python/", "tests/", "tools/", "rust/")) or n in ("Dockerfile", "docker-entrypoint.sh")], names)
            env = dict(os.environ, BAGHOLDER_HOME=os.path.join(tmp, "data"), BAGHOLDER_NO_BROWSER="1", BAGHOLDER_NO_UPDATE="1")
            r = subprocess.run([sys.executable, "-c", "import bagholder; print(bagholder.ASSET_DIR == bagholder.APP_DIR, bagholder.ledger_path().is_file())"],
                               cwd=str(staging), env=env, capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr[-600:])
            self.assertEqual(r.stdout.split()[-2:], ["True", "True"], "an installed copy serves its own page")


class OneVersionTest(unittest.TestCase):
    def test_the_rust_port_carries_the_same_version_and_protocol(self):
        """Both desktop apps are one product version and speak one protocol with the page."""
        import bagholder
        app = os.path.join(REPO, "rust", "crates", "server", "src", "app.rs")
        if not os.path.isfile(app):   # pragma: no cover - an installed copy has no Rust port beside it
            self.skipTest("no rust/ beside python/")
        src = open(app).read()
        self.assertEqual(re.search(r'pub const APP_VERSION: &str = "([^"]+)";', src).group(1), bagholder.APP_VERSION)
        self.assertEqual(re.search(r'pub const PROTOCOL: &str = "([^"]+)";', src).group(1), bagholder.PROTOCOL)


class RepositoryLayoutTest(unittest.TestCase):
    def test_a_checkout_serves_the_shared_page_and_updates_at_the_repository_root(self):
        import bagholder
        from pathlib import Path
        self.assertEqual(bagholder.APP_DIR, Path(ROOT))
        self.assertEqual(bagholder.ASSET_DIR, Path(REPO))
        self.assertTrue(bagholder.ledger_path().is_file())

    def test_the_root_shim_runs_this_app(self):
        """`python3 bagholder.py` at the root of an existing checkout, and the supervisor
        respawning it after an in-app update, run python/bagholder.py."""
        import tempfile
        shim = os.path.join(REPO, "bagholder.py")
        with tempfile.TemporaryDirectory() as tmp:
            probe = "import runpy,sys; sys.argv=[%r]; import builtins; runpy.run_path=lambda p, run_name=None: print('RUN', p, sys.path[0]); exec(open(%r).read(), {'__file__': %r, '__name__': '__main__'})" % (shim, shim, shim)
            r = subprocess.run([sys.executable, "-c", probe], cwd=tmp, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr[-600:])
        self.assertEqual(r.stdout.split(), ["RUN", os.path.join(ROOT, "bagholder.py"), ROOT])


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
            for name in ASSETS:
                src = os.path.join(REPO, name)
                if os.path.exists(src):
                    shutil.copy(src, tmp)
            env = dict(os.environ, BAGHOLDER_HOME=os.path.join(tmp, "data"), BAGHOLDER_NO_BROWSER="1", BAGHOLDER_NO_UPDATE="1")
            r = subprocess.run([sys.executable, "-c", "import bagholder"], cwd=tmp, env=env, capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr[-600:])


if __name__ == "__main__":   # pragma: no cover
    unittest.main()
