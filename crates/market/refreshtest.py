"""Both implementations fetch the real series into their own empty store, and
the two stores are compared date by date and value by value.

This one goes out to the network on purpose: the parsers can be checked
against fixtures, but whether the client is one these hosts will answer cannot
be. FRED in particular replies to an OpenSSL handshake and stonewalls others,
and answers a keep-alive request only when it asks for `identity` encoding.

    cargo build -p bagholder-market && python3 crates/market/refreshtest.py
"""
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
BIN = os.path.join(ROOT, "target", "debug", "refreshtool")


def snap(path):
    c = sqlite3.connect(path)
    fx = dict(c.execute("SELECT date, rate FROM fx_rates"))
    bench = {}
    for sym, d, v in c.execute("SELECT symbol, date, close FROM benchmark_prices"):
        bench.setdefault(sym, {})[d] = v
    c.close()
    return fx, bench


def main():
    work = tempfile.mkdtemp(prefix="refreshtest-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)

    import store
    import market
    for home in (pyhome, rshome):
        store.set_home(home)
        store.ensure()
        store.close_all()

    rs = json.loads(subprocess.run([BIN, "all", os.path.join(rshome, "bagholder.db")],
                                   capture_output=True, text=True, check=True).stdout)

    store.set_home(pyhome)
    py = {"fx": market.refresh_fx(), "benchmark": market.refresh_benchmark() + market.refresh_tsx()}
    store.close_all()

    bad = []
    if py["fx"] != rs["fx"] or py["benchmark"] != rs["benchmark"]:
        bad.append(f"row counts: py={py} rs={rs}")

    (afx, abench), (bfx, bbench) = snap(os.path.join(pyhome, "bagholder.db")), snap(os.path.join(rshome, "bagholder.db"))
    if afx != bfx:
        only = set(afx) ^ set(bfx)
        moved = [d for d in set(afx) & set(bfx) if abs(afx[d] - bfx[d]) > 1e-9]
        bad.append(f"fx: {len(only)} dates on one side only, {len(moved)} values differ")
    for sym in sorted(set(abench) | set(bbench)):
        x, y = abench.get(sym, {}), bbench.get(sym, {})
        only = set(x) ^ set(y)
        moved = [d for d in set(x) & set(y) if abs(x[d] - y[d]) > 1e-9]
        if only or moved:
            bad.append(f"{sym}: py {len(x)} rs {len(y)}, {len(only)} dates on one side only, {len(moved)} values differ")

    for line in bad:
        print("  " + line)
    counts = ", ".join(f"{s} {len(abench.get(s, {}))}" for s in sorted(abench))
    print(f"{len(afx)} fx rates, {counts}, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
