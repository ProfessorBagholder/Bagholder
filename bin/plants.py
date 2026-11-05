#!/usr/bin/env python3
"""The plants a pull request names, each put in, its test run, and taken out again.

A plant is a bug put in on purpose: one exact replacement in one file, and the test
that must catch it. The pull request's description carries its plants as one fenced
block tagged `plants`, a JSON list of `{name, file, old, new, cargo: [...]}`, the
arguments of a `cargo test`, or `{..., spec: [...]}`, the arguments of a
`playwright test`. A plant is caught when the tree built with it and its test failed;
one not applied (its `old` found other than once), one the tree did not compile with,
and one its test did not catch each fail the run. That the named tests pass without
the plants is the suite's to prove (tests.yml, rust and browser). The table goes to
the job's summary (tests.yml, plants).

    python3 bin/plants.py count <body.md> <cargo|spec>   how many plants of that kind
    python3 bin/plants.py run <body.md> <cargo|spec>     run them, from the repository's root
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Literal

Kind = Literal["cargo", "spec"]
KINDS: tuple[Kind, ...] = ("cargo", "spec")
Verdict = Literal["caught", "not applied", "did not compile", "not caught"]

BLOCK = re.compile(r"^```plants[ \t]*\r?\n(.*?)^```[ \t]*$", re.MULTILINE | re.DOTALL)


class PlantsError(Exception):
    """A plants block that cannot be read, in words."""


@dataclass(frozen=True)
class Plant:
    name: str
    file: str
    old: str
    new: str
    kind: Kind
    args: tuple[str, ...]


@dataclass(frozen=True)
class Result:
    plant: Plant
    verdict: Verdict
    detail: str
    seconds: int


@dataclass(frozen=True)
class Steps:
    """How a plant is checked: the commands that build the tree with it (one failing:
    it did not compile), then its test (failing: caught), each `(argv, folder)`."""

    build: tuple[tuple[tuple[str, ...], str], ...]
    test: tuple[tuple[str, ...], str]


def read(body: str) -> list[Plant]:
    """The plants a description names: none without a `plants` block."""
    blocks = BLOCK.findall(body.replace("\r\n", "\n"))
    if not blocks:
        return []
    if len(blocks) > 1:
        raise PlantsError(f"the description has {len(blocks)} plants blocks; give them as one")
    try:
        items = json.loads(blocks[0])
    except json.JSONDecodeError as e:
        raise PlantsError(f"the plants block is not JSON: {e}") from None
    if not isinstance(items, list):
        raise PlantsError("the plants block is a JSON list of plants")
    return [plant(n, item) for n, item in enumerate(items, 1)]


def plant(n: int, item: object) -> Plant:
    """One plant of the block, its fields checked."""
    if not isinstance(item, dict):
        raise PlantsError(f"plant {n} is not an object")
    kinds = [k for k in KINDS if k in item]
    if len(kinds) != 1:
        raise PlantsError(f"plant {n} names its test as one of cargo or spec")
    extra = set(item) - {"name", "file", "old", "new", kinds[0]}
    if extra:
        raise PlantsError(f"plant {n} has fields a plant has not: {', '.join(sorted(extra))}")
    for field in ("name", "file", "old", "new"):
        if not isinstance(item.get(field), str):
            raise PlantsError(f"plant {n}'s {field} is a string")
    if not item["old"]:
        raise PlantsError(f"plant {n}'s old is empty")
    args = item[kinds[0]]
    if not isinstance(args, list) or not args or not all(isinstance(a, str) for a in args):
        raise PlantsError(f"plant {n}'s {kinds[0]} is a list of arguments")
    file = Path(item["file"])
    if file.is_absolute() or ".." in file.parts:
        raise PlantsError(f"plant {n}'s file is a path inside the repository")
    return Plant(item["name"], item["file"], item["old"], item["new"], kinds[0], tuple(args))


def steps(p: Plant) -> Steps:
    """The build and the test of a plant of each kind, as the suite runs them."""
    if p.kind == "cargo":
        return Steps(build=((("cargo", "test", "--locked", "--no-run", *p.args), "rust"),), test=(("cargo", "test", "--locked", *p.args), "rust"))
    # the page is built into the server, so the page is built first
    return Steps(
        build=((("npx", "vite", "build"), "web"), (("cargo", "build", "--locked", "--bin", "bagholder", "--features", "demo"), "rust")),
        test=(("npx", "playwright", "test", "--reporter=line", *p.args), "web"),
    )


def check(p: Plant, root: Path, steps_of: Callable[[Plant], Steps] = steps) -> tuple[Verdict, str]:
    """A plant put in already, judged: did the tree build, and did its test fail."""
    s = steps_of(p)
    for argv, folder in s.build:
        r = subprocess.run(argv, cwd=root / folder, capture_output=True, text=True)
        if r.returncode != 0:
            return "did not compile", said(r.stdout + r.stderr, ("error", "Error"))
    argv, folder = s.test
    r = subprocess.run(argv, cwd=root / folder, capture_output=True, text=True)
    out = r.stdout + r.stderr
    if r.returncode == 0:
        return "not caught", said(out, ("test result:", "passed"))
    failed = sorted({l.split(" ... ")[0].removeprefix("test ") for l in out.splitlines() if l.startswith("test ") and l.endswith("FAILED")})
    failed += [l.strip() for l in out.splitlines() if l.strip().startswith("✘")]
    return "caught", "; ".join(failed) if failed else said(out, ("failed", "FAILED"))


def said(out: str, marks: tuple[str, ...]) -> str:
    """The lines of a command's output that say what happened: those with a mark, else its last."""
    lines = [l.strip() for l in out.splitlines() if l.strip()]
    marked = [l for l in lines if any(m in l for m in marks)]
    return " | ".join((marked or lines[-1:])[:3])


def run(plants: list[Plant], root: Path, judge: Callable[[Plant, Path], tuple[Verdict, str]] = check) -> list[Result]:
    """Each plant put in, judged, and its file put back as it was, before the next."""
    results = []
    for p in plants:
        path = root / p.file
        was = path.read_text() if path.is_file() else None
        found = was.count(p.old) if was is not None else 0
        if was is None or found != 1:
            results.append(Result(p, "not applied", "no such file" if was is None else f"old found {found} times", 0))
            continue
        start = time.monotonic()
        path.write_text(was.replace(p.old, p.new))
        try:
            verdict, detail = judge(p, root)
        finally:
            path.write_text(was)
        results.append(Result(p, verdict, detail, int(time.monotonic() - start)))
    return results


def table(results: list[Result]) -> str:
    """The results as the job summary's table."""
    cell = lambda s: s.replace("|", "\\|").replace("\n", " ")[:300]
    rows = [f"| {n} | {cell(r.plant.name)} | `{cell(r.plant.file)}` | `{cell(' '.join(r.plant.args))}` | **{r.verdict}** {cell(r.detail)} | {r.seconds} |" for n, r in enumerate(results, 1)]
    caught = sum(r.verdict == "caught" for r in results)
    return "\n".join([f"{caught} of {len(results)} plants caught", "", "| | Plant | File | Test | Verdict | s |", "|---|---|---|---|---|---|", *rows, ""])


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[0] not in ("count", "run") or argv[2] not in KINDS:
        print(__doc__, file=sys.stderr)
        return 2
    what, body, kind = argv
    try:
        plants = [p for p in read(Path(body).read_text()) if p.kind == kind]
    except PlantsError as e:
        print(f"plants: {e}", file=sys.stderr)
        return 1
    if what == "count":
        print(len(plants))
        return 0
    root = Path.cwd()
    results = run(plants, root)
    out = table(results)
    print(out)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(f"### {kind} plants\n\n{out}\n")
    # every file put back: the tree as it was checked out
    left = subprocess.run(["git", "status", "--porcelain", "--untracked-files=no"], cwd=root, capture_output=True, text=True).stdout
    if left:
        print(f"plants: the tree was not put back:\n{left}", file=sys.stderr)
        return 1
    return 0 if all(r.verdict == "caught" for r in results) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
