#!/usr/bin/env python3
"""Starts Bagholder's server from this checkout: `python3 bagholder.py`.

The same file is at the repository root and at python/bagholder.py. It runs,
in this order: `cargo run --release --bin bagholder` in rust/ when cargo is on
the PATH (building the page in web/ first when it has not been built), else the
server already built at rust/target/release/bagholder, else it says what to run
and exits with 1. The environment and the arguments are passed through, and the
server takes this process's place, so whatever started this file is running the
server.
"""
import os
import shutil
import subprocess
import sys


def checkout():
    here = os.path.dirname(os.path.abspath(__file__))
    for d in (here, os.path.dirname(here)):
        if os.path.isfile(os.path.join(d, "rust", "Cargo.toml")):
            return d
    return None


def run(argv, cwd):
    """This process becomes argv, run in cwd; on Windows, where a process cannot
    be replaced, argv runs as a child and its exit status is this one's."""
    os.chdir(cwd)
    if os.name == "nt":
        sys.exit(subprocess.call(argv))
    os.execvp(argv[0], argv)


def main():
    root = checkout()
    if root is None:
        sys.stderr.write("bagholder: no rust/Cargo.toml beside %s; this file starts the server from a checkout of the repository\n" % os.path.abspath(__file__))
        return 1
    rust = os.path.join(root, "rust")
    web = os.path.join(root, "web")
    args = sys.argv[1:]
    cargo = shutil.which("cargo")
    if cargo:
        if not os.path.isfile(os.path.join(web, "dist", "index.html")):
            npm = shutil.which("npm")
            if not npm:
                sys.stderr.write("bagholder: the page is not built and npm is not on the PATH. Install Node.js, then run:\n"
                                 "  cd %s && npm ci && npm run build\n" % web)
                return 1
            for step in (["ci"], ["run", "build"]):
                if subprocess.call([npm] + step, cwd=web) != 0:
                    sys.stderr.write("bagholder: npm %s failed in %s\n" % (" ".join(step), web))
                    return 1
        run([cargo, "run", "--release", "--bin", "bagholder", "--"] + args, rust)
    exe = os.path.join(rust, "target", "release", "bagholder.exe" if os.name == "nt" else "bagholder")
    if os.path.isfile(exe):
        run([exe] + args, root)
    sys.stderr.write("bagholder: the server is not built and cargo is not on the PATH. Install Rust (https://rustup.rs) and Node.js, then run:\n"
                     "  cd %s && npm ci && npm run build\n"
                     "  cd %s && cargo run --release --bin bagholder\n" % (web, rust))
    return 1


if __name__ == "__main__":
    sys.exit(main())
