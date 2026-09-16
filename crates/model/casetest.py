"""The shared cases, run through the Rust model.

Each file in tests/cases holds a snapshot and the figures the spec says it
produces. tests/test_cases.py runs them through the Python model; this runs the
same files through the Rust one and compares against the same `expect`, so the
two cannot disagree without a failing test.

    cargo build --bin casetool && python3 crates/model/casetest.py
"""
import glob
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.path.join(ROOT, "target", "debug", "casetool")


def diff(path, want, got, out):
    """Every leaf that differs, named by its path through the document."""
    if isinstance(want, dict) and isinstance(got, dict):
        for k in sorted(set(want) | set(got)):
            if k not in want:
                out.append(f"{path}.{k}: only rust has it ({got[k]!r})")
            elif k not in got:
                out.append(f"{path}.{k}: only python has it ({want[k]!r})")
            else:
                diff(f"{path}.{k}", want[k], got[k], out)
    elif isinstance(want, list) and isinstance(got, list):
        if len(want) != len(got):
            out.append(f"{path}: {len(want)} rows in python, {len(got)} in rust")
            return
        for i, (w, g) in enumerate(zip(want, got)):
            diff(f"{path}[{i}]", w, g, out)
    elif isinstance(want, bool) or isinstance(got, bool):
        if want is not got:
            out.append(f"{path}: py={want!r} rs={got!r}")
    elif isinstance(want, (int, float)) and isinstance(got, (int, float)):
        if abs(float(want) - float(got)) > 5e-7:
            out.append(f"{path}: py={want!r} rs={got!r}")
    elif want != got:
        out.append(f"{path}: py={want!r} rs={got!r}")


def main():
    paths = sorted(glob.glob(os.path.join(ROOT, "tests", "cases", "*.json")))
    if not paths:
        print("no cases found")
        return 1
    failures = 0
    for path in paths:
        name = os.path.basename(path)[:-5]
        doc = json.load(open(path))
        got = json.loads(subprocess.run(
            [BIN], input=json.dumps(doc), capture_output=True, text=True, check=True).stdout)
        out = []
        diff(name, doc["expect"], got, out)
        if out:
            failures += 1
            for line in out[:8]:
                print("  " + line)
            if len(out) > 8:
                print(f"  ... and {len(out) - 8} more in {name}")
    print(f"{len(paths)} cases, {failures} failing")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
