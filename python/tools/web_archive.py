#!/usr/bin/env python3
"""Builds the Python app's release archive from a git ref.

    python3 python/tools/web_archive.py vX.Y.Z [--ref <commit>] [--out <dir>]

writes bagholder-vX.Y.Z-web.zip and bagholder-vX.Y.Z-web.zip.sha256. The archive is
flat, as every installed copy's updater expects: the modules of python/, its
requirements.txt and mcp/ manifest, and the page with its assets, all at the root.
The tests, the tools and the container files stay out.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import os
import subprocess
import sys
import tarfile
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ASSETS = ("ledger.html", "lightweight-charts.js", "favicon.png")


def shipped(path):
    """The archive name for a tracked path, or None when it does not ship."""
    if path in ASSETS:
        return path
    if not path.startswith("python/"):
        return None
    rel = path[len("python/"):]
    if "/" not in rel:
        return rel if rel.endswith(".py") or rel == "requirements.txt" else None
    return rel if rel.startswith("mcp/") else None


def _tracked_tar():
    """The tracked files of the working tree as a tar, for building before a commit."""
    names = subprocess.run(["git", "-C", ROOT, "ls-files", "-z", "--", "python", *ASSETS],
                           capture_output=True, check=True).stdout.decode().split("\0")
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as t:
        for n in names:
            if n and os.path.isfile(os.path.join(ROOT, n)):
                t.add(os.path.join(ROOT, n), arcname=n)
    return buf.getvalue()


def build(tag, ref=None, out="."):
    """`ref` is a commit, or "WORKTREE" for the tracked files as they are on disk."""
    if ref == "WORKTREE":
        tar = _tracked_tar()
    else:
        tar = subprocess.run(["git", "-C", ROOT, "archive", "--format=tar", ref or tag, "--", "python", *ASSETS],
                             capture_output=True, check=True).stdout
    name = os.path.join(out, "bagholder-%s-web.zip" % tag)
    written = []
    with tarfile.open(fileobj=io.BytesIO(tar)) as t, zipfile.ZipFile(name, "w", zipfile.ZIP_DEFLATED) as z:
        for m in sorted(t.getmembers(), key=lambda m: m.name):
            arc = shipped(m.name) if m.isfile() else None
            if arc:
                info = zipfile.ZipInfo(arc, date_time=(1980, 1, 1, 0, 0, 0))
                info.external_attr = (m.mode & 0o777) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                z.writestr(info, t.extractfile(m).read())
                written.append(arc)
    digest = hashlib.sha256(open(name, "rb").read()).hexdigest()
    with open(name + ".sha256", "w") as f:
        f.write("%s  %s\n" % (digest, os.path.basename(name)))
    return name, written


def main(argv):
    p = argparse.ArgumentParser()
    p.add_argument("tag")
    p.add_argument("--ref", help="the commit to build from (WORKTREE: the tracked files on disk); the tag itself by default")
    p.add_argument("--out", default=".")
    a = p.parse_args(argv)
    name, written = build(a.tag, a.ref, a.out)
    print("%s: %d files" % (name, len(written)))


if __name__ == "__main__":
    main(sys.argv[1:])
