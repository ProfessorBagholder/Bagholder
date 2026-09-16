"""Run every case's activities through both match_fifo implementations and
compare the closed slices, the open lots and the unmatched fills field by
field. Run from the repository root:

    cargo build --bin difftool && python3 crates/model/fifotest.py
"""
import glob
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import model  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "difftool")


def norm(v):
    """Compare numbers by value, not by their Python or Rust spelling."""
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 9) + 0.0
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, list):
        return [norm(x) for x in v]
    return v


def main():
    cases = sorted(glob.glob(os.path.join(ROOT, "tests", "cases", "*.json")))
    bad = []
    slices = 0
    for path in cases:
        name = os.path.basename(path)[:-5]
        acts = json.load(open(path))["snapshot"]["activities"]
        want = model.match_fifo([dict(a) for a in acts])
        got = json.loads(subprocess.run([BIN, "fifo"], input=json.dumps(acts),
                                        capture_output=True, text=True, check=True).stdout)
        slices += len(want["closed"])
        for section in ("closed", "open", "unmatched"):
            w, g = norm(want[section]), norm(got[section])
            if len(w) != len(g):
                bad.append(f"{name}: {section} has {len(w)} in python, {len(g)} in rust")
                continue
            for i, (wr, gr) in enumerate(zip(w, g)):
                for k in sorted(set(wr) | set(gr)):
                    if wr.get(k) != gr.get(k):
                        bad.append(f"{name}: {section}[{i}].{k} py={wr.get(k)!r} rs={gr.get(k)!r}")
    print(f"{len(cases)} cases, {slices} closed slices")
    for line in bad[:40]:
        print("  " + line)
    print(f"{len(bad)} mismatches")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
