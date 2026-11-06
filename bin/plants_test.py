#!/usr/bin/env python3
"""`bin/plants.py` over made-up trees: the plants block read and refused, and each
verdict, with the build and the test stood in by small Python commands so a plant is
judged as the job judges it. Run by tests.yml (plants).

    python3 bin/plants_test.py
"""

import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import Callable

sys.path.insert(0, str(Path(__file__).resolve().parent))
import plants  # noqa: E402

# the tree's code and its test, which says what it ran as cargo's harness does
CODE = "def double(x):\n    return x * 2\n"
TEST = "from code import double\nok = double(3) == 6\nprint('test doubles ... ' + ('ok' if ok else 'FAILED'))\nraise SystemExit(0 if ok else 1)\n"


def block(items: str) -> str:
    return f"The change.\n\n```plants\n{items}\n```\n\nMore words.\n"


def stood_in(*test: str) -> "Callable[[plants.Plant], plants.Steps]":
    """The build: the code compiles; the test: TEST run beside it, or the Python given."""
    argv = (sys.executable, "-c", test[0]) if test else (sys.executable, "test.py")
    return lambda p: plants.Steps(build=(((sys.executable, "-m", "py_compile", "code.py"), "src"),), test=(argv, "src"))


class Read(unittest.TestCase):
    def test_a_description_with_no_plants_block_has_no_plants(self) -> None:
        self.assertEqual(plants.read("A version bump.\n\n```bash\necho hi\n```\n"), [])

    def test_a_block_reads_as_plants_of_each_kind(self) -> None:
        got = plants.read(block('[{"name": "n", "file": "a.rs", "old": "x", "new": "y", "cargo": ["-p", "c", "--test", "t"]},'
                                ' {"name": "m", "file": "w.ts", "old": "a", "new": "", "spec": ["e2e/s.spec.ts"]},'
                                ' {"name": "u", "file": "p.ts", "old": "b", "new": "c", "vitest": ["src/lib/p.test.ts", "-t", "waits"]}]'))
        self.assertEqual(got, [
            plants.Plant("n", "a.rs", "x", "y", "cargo", ("-p", "c", "--test", "t")),
            plants.Plant("m", "w.ts", "a", "", "spec", ("e2e/s.spec.ts",)),
            plants.Plant("u", "p.ts", "b", "c", "vitest", ("src/lib/p.test.ts", "-t", "waits")),
        ])
        # each kind run as the suite runs it: a page unit test with nothing to build first
        self.assertEqual(plants.steps(got[2]), plants.Steps(build=(), test=(("npx", "vitest", "run", "src/lib/p.test.ts", "-t", "waits"), "web")))

    def test_a_block_that_cannot_be_read_is_refused_in_words(self) -> None:
        for items, says in [
            ("[{", "not JSON"),
            ('{"name": "n"}', "a JSON list"),
            ('[{"name": "n", "file": "a.rs", "old": "x", "new": "y"}]', "one of cargo, spec or vitest"),
            ('[{"name": "n", "file": "a.rs", "old": "x", "new": "y", "cargo": ["t"], "spec": ["s"]}]', "one of cargo, spec or vitest"),
            ('[{"name": "n", "file": "a.rs", "old": "", "new": "y", "cargo": ["t"]}]', "old is empty"),
            ('[{"name": "n", "file": "a.rs", "old": "x", "new": "y", "cargo": []}]', "a list of arguments"),
            ('[{"name": "n", "file": "../a.rs", "old": "x", "new": "y", "cargo": ["t"]}]', "inside the repository"),
            ('[{"name": "n", "file": "/a.rs", "old": "x", "new": "y", "cargo": ["t"]}]', "inside the repository"),
            ('[{"name": "n", "file": "a.rs", "old": "x", "new": "y", "cargo": ["t"], "why": "?"}]', "fields a plant has not: why"),
        ]:
            with self.subTest(items=items), self.assertRaises(plants.PlantsError) as e:
                plants.read(block(items))
            self.assertIn(says, str(e.exception))
        with self.assertRaises(plants.PlantsError) as e:
            plants.read(block("[]") + block("[]"))
        self.assertIn("2 plants blocks", str(e.exception))


class Run(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)
        (self.root / "src").mkdir()
        (self.root / "src/code.py").write_text(CODE)
        (self.root / "src/test.py").write_text(TEST)

    def tearDown(self) -> None:
        self.dir.cleanup()

    def judged(self, old: str, new: str, *test: str, limit: float = plants.LIMIT) -> plants.Result:
        p = plants.Plant("p", "src/code.py", old, new, "cargo", ("t",))
        [r] = plants.run([p], self.root, lambda p, root: plants.check(p, root, stood_in(*test), limit))
        # put back as it was, whatever the verdict
        self.assertEqual((self.root / "src/code.py").read_text(), CODE)
        return r

    def test_a_real_plant_is_caught(self) -> None:
        r = self.judged("x * 2", "x + 2")
        self.assertEqual((r.verdict, r.detail), ("caught", "doubles"))

    def test_a_page_unit_test_failing_is_caught_by_the_test_it_names(self) -> None:
        r = self.judged("x * 2", "x + 2", "print(' FAIL  src/lib/a.test.ts > the page > doubles'); print('   × doubles 3ms'); raise SystemExit(1)")
        self.assertEqual(r.verdict, "caught")
        self.assertIn("src/lib/a.test.ts > the page > doubles", r.detail)
        r = self.judged("x * 2", "x + 2", "print('No test files found, exiting with code 1'); raise SystemExit(1)")
        self.assertEqual(r.verdict, "did not run")

    def test_a_spec_failing_is_caught_by_the_test_it_names(self) -> None:
        r = self.judged("x * 2", "x + 2", "print('  ✘  1 [app] › e2e/a.spec.ts:3:1 › doubles (1.2s)'); print('  1 failed'); raise SystemExit(1)")
        self.assertEqual(r.verdict, "caught")
        self.assertIn("e2e/a.spec.ts:3:1 › doubles", r.detail)
        r = self.judged("x * 2", "x + 2", "print('  1) [app] › e2e/a.spec.ts:3:1 › doubles'); print('  1 failed'); raise SystemExit(1)")
        self.assertEqual(r.verdict, "caught")

    def test_a_test_command_failing_with_no_test_named_did_not_run(self) -> None:
        for said in ("Error: No tests found", "error: no test target named `owd`", "running 0 tests"):
            with self.subTest(said=said):
                r = self.judged("x * 2", "x + 2", f"print({said!r}); raise SystemExit(1)")
                self.assertEqual((r.verdict, r.detail), ("did not run", said))

    def test_a_test_that_runs_past_its_limit_is_stopped_with_what_it_started(self) -> None:
        start = time.monotonic()
        r = self.judged("x * 2", "x + 2", "import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); print('waiting', flush=True); time.sleep(60)", limit=1)
        self.assertEqual(r.verdict, "timed out")
        self.assertIn("past 1 s: waiting", r.detail)
        self.assertLess(time.monotonic() - start, 15)

    def test_a_plant_that_changes_nothing_is_not_caught(self) -> None:
        self.assertEqual(self.judged("x * 2", "x * 2").verdict, "not caught")
        self.assertEqual(self.judged("x * 2", "2 * x").verdict, "not caught")

    def test_a_plant_found_twice_or_not_at_all_is_not_applied(self) -> None:
        r = self.judged("x", "y")
        self.assertEqual((r.verdict, r.detail), ("not applied", "old found 2 times"))
        r = self.judged("x * 3", "x")
        self.assertEqual((r.verdict, r.detail), ("not applied", "old found 0 times"))

    def test_a_plant_the_tree_does_not_compile_with_is_said(self) -> None:
        self.assertEqual(self.judged("return x * 2", "return x *").verdict, "did not compile")

    def test_a_file_that_is_not_there_is_not_applied(self) -> None:
        p = plants.Plant("p", "src/gone.py", "x", "y", "cargo", ("t",))
        self.assertEqual(plants.run([p], self.root, lambda p, root: ("caught", "")), [plants.Result(p, "not applied", "no such file", 0)])

    def test_the_file_is_put_back_when_the_judging_fails(self) -> None:
        def broken(p: plants.Plant, root: Path) -> tuple[plants.Verdict, str]:
            raise OSError("no cargo here")

        with self.assertRaises(OSError):
            plants.run([plants.Plant("p", "src/code.py", "x * 2", "x + 2", "cargo", ("t",))], self.root, broken)
        self.assertEqual((self.root / "src/code.py").read_text(), CODE)

    def test_the_table_counts_what_was_caught(self) -> None:
        caught = self.judged("x * 2", "x + 2")
        missed = self.judged("x * 2", "x * 2")
        out = plants.table([caught, missed])
        self.assertTrue(out.startswith("1 of 2 plants caught"))
        self.assertIn("**not caught**", out)


if __name__ == "__main__":
    unittest.main()
