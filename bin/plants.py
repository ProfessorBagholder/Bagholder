#!/usr/bin/env python3
"""The plants a pull request names, each put in, its test run, and taken out again.

A plant is a bug put in on purpose: one exact replacement in one file, and the test
that must catch it. The pull request's description carries its plants as one fenced
block tagged `plants`, a JSON list of `{name, file, old, new, cargo: [...]}`, the
arguments of a `cargo test`, `{..., spec: [...]}`, the arguments of a `playwright
test`, or `{..., vitest: [...]}`, the arguments of a `vitest run` of the page's unit
tests (a file, and `-t` and a test's name). A plant is caught when the tree built with it and its test failed,
naming the test that failed; one not applied (its `old` found other than once), one
the tree did not compile with, one whose test failed naming no test (nothing ran:
a spec path mistyped, a filter that matches nothing, a server that never started),
one that ran past its limit, and one its test did not catch each fail the run. That
the named tests pass without the plants is the suite's to prove (tests.yml, rust,
browser and web). The table goes to the job's summary (tests.yml, plants).

Cargo plants build with the suite's flags (`-D warnings`), so the suite's cache is
their build: a plant builds without a warning, and one that leaves an unused variable
reads `did not compile`, the warning its detail.

    python3 bin/plants.py count <body.md> <cargo|spec|vitest>   how many plants of that kind
    python3 bin/plants.py run <body.md> <cargo|spec|vitest>     run them, from the repository's root
"""

from __future__ import annotations

import json
import os
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Literal

Kind = Literal["cargo", "spec", "vitest"]
KINDS: tuple[Kind, ...] = ("cargo", "spec", "vitest")
Verdict = Literal["caught", "not applied", "did not compile", "did not run", "timed out", "not caught"]

# Each build or test command's limit: a plant can make its test wait for ever (a wait
# that never ends, a lock never released), which would hold the job for GitHub's six
# hours. The longest command a plant runs, the server's test build (125 s on the
# suite's cache), fits it with room.
LIMIT = 30 * 60

# what names a failed test: cargo's `test <name> ... FAILED`, and Playwright's `✘`
# line, its `1) [project] › file › title` heading or its `N failed` count
CARGO_FAILED = re.compile(r"^test (\S+) \.\.\. FAILED$")
SPEC_FAILED = re.compile(r"^(✘.*|\d+\) \[.*› .*|[1-9]\d* failed\b.*)$")
# and vitest's `FAIL  <file> > <describe> > <test>` heading or its `×` line
VITEST_FAILED = re.compile(r"^(FAIL\s+\S+\s+>\s.*|×\s.*)$")
# a terminal's colours, which a test runner sends where it takes the job's log for
# one (vitest on GitHub's runners): never part of what a line says
COLOUR = re.compile(r"\x1b\[[0-9;]*m")

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
        raise PlantsError(f"plant {n} names its test as one of cargo, spec or vitest")
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
    # vitest builds what it tests as it runs: a page that does not build fails naming
    # no test, which is `did not run`
    if p.kind == "vitest":
        return Steps(build=(), test=(("npx", "vitest", "run", *p.args), "web"))
    # the page is built into the server, so the page is built first
    return Steps(
        build=((("npx", "vite", "build"), "web"), (("cargo", "build", "--locked", "--bin", "bagholder", "--features", "demo"), "rust")),
        test=(("npx", "playwright", "test", "--reporter=line", *p.args), "web"),
    )


def check(p: Plant, root: Path, steps_of: Callable[[Plant], Steps] = steps, limit: float = LIMIT) -> tuple[Verdict, str]:
    """A plant put in already, judged: did the tree build, and did its test fail,
    naming the test."""
    s = steps_of(p)
    for argv, folder in s.build:
        code, out = ran(argv, root / folder, limit)
        if code is None:
            return "timed out", f"the build ran past {limit:g} s: {said(out, ())}"
        if code != 0:
            return "did not compile", said(out, ("error", "Error"))
    argv, folder = s.test
    code, out = ran(argv, root / folder, limit)
    if code is None:
        return "timed out", f"the test ran past {limit:g} s: {said(out, ())}"
    if code == 0:
        return "not caught", said(out, ("test result:", "passed"))
    named = failures(out)
    return ("caught", "; ".join(named)) if named else ("did not run", said(out, ()))


def ran(argv: tuple[str, ...], cwd: Path, limit: float) -> tuple[int | None, str]:
    """A command's exit status and output, or no status for one stopped at its limit,
    with every process it started: a test binary or a server holding the output open
    would otherwise keep the wait going."""
    proc = subprocess.Popen(argv, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, start_new_session=True)
    try:
        out, _ = proc.communicate(timeout=limit)
        return proc.returncode, COLOUR.sub("", out)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        out, _ = proc.communicate()
        return None, COLOUR.sub("", out)


def failures(out: str) -> list[str]:
    """The failed tests a test command's output names."""
    lines = [l.strip() for l in out.splitlines()]
    cargo = sorted({m.group(1) for l in lines if (m := CARGO_FAILED.match(l))})
    return cargo + [l for l in lines if SPEC_FAILED.match(l) or VITEST_FAILED.match(l)]


def said(out: str, marks: tuple[str, ...]) -> str:
    """The lines of a command's output that say what happened: the first three with a
    mark, else its last three."""
    lines = [l.strip() for l in out.splitlines() if l.strip()]
    marked = [l for l in lines if any(m in l for m in marks)]
    return " | ".join(marked[:3] or lines[-3:])


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
