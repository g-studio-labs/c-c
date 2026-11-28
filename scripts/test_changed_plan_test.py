"""Pruebas de `test_changed_plan.py`: qué corre la caja por cada cambio.

    python3 -I scripts/test_changed_plan_test.py
"""

import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PLANNER = os.path.join(HERE, "test_changed_plan.py")
GUARD = os.path.join(HERE, "cargo_guard.py")

FILES = {
    "core/Cargo.toml": "",
    "core/src/lib.rs": "pub mod dates;\npub mod chat;\n\npub fn shared() {}\n",
    "core/src/dates.rs": "",
    "core/src/chat.rs": "#[cfg(test)]\nmod tests {\n    #[test]\n    fn one() {}\n}\n",
    "core/tests/chat.rs": "",
    "mcp/src/lib.rs": "",
    "ops/src/lib.rs": "",
    "desktop/src/main.rs": "mod chat;\nmod workspace;\n",
    "desktop/src/chat.rs": "",
    "desktop/src/workspace.rs": 'mod tests {\n    include!("docs_tests.rs");\n}\n',
    "desktop/src/docs_tests.rs": "#[gpui_kit::test]\nfn the_index_opens(cx: &mut T) {}\n\n#[test]\nfn plain_one() {}\n",
    "desktop/src/platform/mod.rs": "",
    "desktop/src/platform/provision.rs": "",
    "diagram/Cargo.toml": "",
    "diagram/src/lib.rs": "mod cache;\n",
    "diagram/src/cache.rs": "#[cfg(test)]\nmod tests {\n    #[test]\n    fn one() {}\n}\n",
}


class Planner(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        root = self.dir.name
        for path, text in FILES.items():
            full = os.path.join(root, path)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w", encoding="utf-8") as handle:
                handle.write(text)
        git = lambda *args: subprocess.run(["git", "-C", root, *args], check=True, capture_output=True)
        git("init", "-q")
        git("-c", "user.email=t@t", "-c", "user.name=t", "add", ".")
        git("-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false", "commit", "-qm", "base")

    def tearDown(self):
        self.dir.cleanup()

    def edit(self, path, text):
        with open(os.path.join(self.dir.name, path), "a", encoding="utf-8") as handle:
            handle.write(text)

    def plan(self, *changed, local=False):
        args = [sys.executable, "-I", PLANNER] + (["--local"] if local else [])
        out = subprocess.run(args, input="\n".join(changed), capture_output=True, text=True, cwd=self.dir.name, check=True)
        return out.stdout

    def crate(self, plan, name):
        line = next((l for l in plan.splitlines() if l.startswith(f'echo "== {name}"')), "")
        return line

    def test_a_screen_runs_only_its_module(self):
        plan = self.plan("desktop/src/chat.rs")
        self.assertIn("cargo test --quiet --bins -- chat::", self.crate(plan, "desktop"))
        self.assertEqual(self.crate(plan, "core"), "")

    def test_a_nested_module_is_named_by_its_path(self):
        plan = self.plan("desktop/src/platform/provision.rs", "desktop/src/platform/mod.rs")
        self.assertIn("-- platform:: platform::provision::", self.crate(plan, "desktop"))

    def test_an_included_test_file_runs_its_tests_by_name(self):
        line = self.crate(self.plan("desktop/src/docs_tests.rs"), "desktop")
        self.assertIn("the_index_opens", line)
        self.assertIn("plain_one", line)
        self.assertNotIn("docs_tests::", line)

    def test_a_core_module_brings_its_integration_test_and_the_screen_and_checks_dependents(self):
        plan = self.plan("core/src/chat.rs")
        self.assertIn("cargo test --quiet --lib -- chat::", self.crate(plan, "core"))
        self.assertIn("cargo test --quiet --test chat", self.crate(plan, "core"))
        self.assertIn("--bins -- chat::", self.crate(plan, "desktop"))
        for dependent in ("mcp", "ops"):
            self.assertIn("cargo check --quiet --tests", self.crate(plan, dependent))

    def test_the_diagrams_run_their_module_and_check_the_app_that_draws_them(self):
        plan = self.plan("diagram/src/cache.rs")
        self.assertIn("cargo test --quiet --lib -- cache::", self.crate(plan, "diagram"))
        self.assertIn("cargo check --quiet --tests", self.crate(plan, "desktop"))
        self.assertRegex(self.crate(self.plan("diagram/Cargo.toml"), "diagram"), r"cargo test --quiet 2>&1")

    def test_a_shared_file_runs_the_whole_crate(self):
        for shared in ("core/Cargo.toml", "desktop/src/testing.rs"):
            name = shared.split("/")[0]
            line = self.crate(self.plan(shared), name)
            self.assertRegex(line, r"cargo test --quiet 2>&1", shared)

    def test_a_root_that_only_declares_modules_is_compiled_not_run_whole(self):
        self.edit("core/src/lib.rs", "pub mod extra;\n")
        self.assertIn("cargo check --quiet --tests", self.crate(self.plan("core/src/lib.rs"), "core"))
        self.edit("core/src/lib.rs", "pub fn other() {}\n")
        self.assertRegex(self.crate(self.plan("core/src/lib.rs"), "core"), r"cargo test --quiet 2>&1")

    def test_a_failure_makes_the_whole_run_fail(self):
        plan = self.plan("desktop/src/chat.rs")
        self.assertIn("fail=1", plan)
        self.assertIn("PIPESTATUS", plan)
        self.assertTrue(plan.rstrip().endswith('exit "$fail"'))

    def test_nothing_native_plans_nothing_in_the_box(self):
        self.assertEqual(self.plan("backend/main.go", "app/src/App.tsx").strip(), "")

    def test_the_rest_runs_here_by_suite(self):
        self.assertEqual(self.plan("app/src/App.tsx", local=True).splitlines(), ["cd app && npx tsc --noEmit && bun run test"])
        self.assertEqual(self.plan("app/src-tauri/src/lib.rs", local=True).splitlines(), ["cd app/src-tauri && cargo check --locked"])
        self.assertIn("go test -timeout 30m ./...", self.plan("backend/internal/x.go", local=True))
        self.assertIn("hooks_test.py", self.plan(".claude/hooks/cac_hooks.py", local=True))
        self.assertEqual(self.plan("docs/x.md", local=True).strip(), "")

    def test_what_tauri_links_checks_tauri(self):
        for crate in ("core/src/chat.rs", "mcp/src/lib.rs", "ops/src/lib.rs"):
            self.assertIn("cd app/src-tauri && cargo check", self.plan(crate, local=True), crate)
        self.assertNotIn("src-tauri", self.plan("desktop/src/chat.rs", local=True))

    def test_the_go_domain_runs_the_core_test_files(self):
        self.assertIn("cargo test --quiet --tests", self.crate(self.plan("backend/internal/core/domain/task.go"), "core"))

    def test_a_filter_over_a_file_with_tests_is_guarded(self):
        self.assertIn("cac_guard --expect-tests", self.crate(self.plan("core/src/chat.rs"), "core"))

    def test_a_filter_over_a_file_without_tests_is_not_guarded(self):
        line = self.crate(self.plan("core/src/dates.rs"), "core")
        self.assertIn("cac_guard;", line)
        self.assertNotIn("--expect-tests", line)

    def run_plan(self, plan, cargo_output, cargo_status=0):
        """Corre el guion de verdad, con un `cargo` falso que escribe esa salida."""
        for crate in ("core", "desktop"):
            os.makedirs(os.path.join(self.dir.name, crate), exist_ok=True)
        bin_dir = os.path.join(self.dir.name, "fakebin")
        os.makedirs(bin_dir, exist_ok=True)
        with open(os.path.join(bin_dir, "cargo"), "w", encoding="utf-8") as handle:
            handle.write(f"#!/bin/sh\ncat <<'EOF'\n{cargo_output}\nEOF\nexit {cargo_status}\n")
        os.chmod(os.path.join(bin_dir, "cargo"), 0o755)
        env = dict(os.environ, PATH=bin_dir + os.pathsep + os.environ["PATH"])
        return subprocess.run(["bash", "-c", plan], capture_output=True, text=True, cwd=self.dir.name, env=env)

    def test_a_stale_binary_fails_the_run_with_a_reason(self):
        plan = self.plan("core/src/chat.rs")
        stale = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 236 filtered out; finished in 0.00s"
        done = self.run_plan(plan, stale)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("el filtro no corrió ninguna prueba", done.stdout)

    def test_a_filter_that_ran_something_passes(self):
        plan = self.plan("core/src/chat.rs")
        done = self.run_plan(plan, "test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 229 filtered out; finished in 0.00s")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_a_cargo_failure_still_fails_the_run(self):
        plan = self.plan("core/src/chat.rs")
        done = self.run_plan(plan, "test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s", cargo_status=101)
        self.assertNotEqual(done.returncode, 0)


def load_guard():
    import importlib.util

    spec = importlib.util.spec_from_file_location("cargo_guard", GUARD)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Guard(unittest.TestCase):
    def setUp(self):
        self.verdict = load_guard().verdict

    def test_nothing_run_with_something_filtered_out_is_a_stale_binary(self):
        self.assertIsNotNone(self.verdict(["test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 236 filtered out"], True))

    def test_a_binary_that_ran_tests_is_fine(self):
        self.assertIsNone(self.verdict(["test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 229 filtered out"], True))

    def test_a_failing_test_counts_as_having_run(self):
        self.assertIsNone(self.verdict(["test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out"], True))

    def test_one_binary_with_nothing_is_fine_if_another_ran(self):
        lines = [
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out",
            "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out",
        ]
        self.assertIsNone(self.verdict(lines, True))
        self.assertIsNone(self.verdict(lines[::-1], True))

    def test_without_the_expectation_nothing_is_demanded(self):
        self.assertIsNone(self.verdict(["test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 236 filtered out"], False))

    def test_no_result_lines_at_all_is_left_to_cargo(self):
        self.assertIsNone(self.verdict(["error: could not compile"], True))


class Script(unittest.TestCase):
    """`test-changed.sh` corrido de verdad contra suites falsas: un rojo de
    dentro tiene que salir como código distinto de 0 y con su nombre
    (command-and-control-529)."""

    SCRIPT = os.path.join(HERE, "test-changed.sh")

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = self.dir.name
        scripts = os.path.join(self.root, "scripts")
        os.makedirs(scripts)
        for name in ("test-changed.sh", "test_changed_plan.py", "cargo_guard.py"):
            with open(os.path.join(HERE, name), encoding="utf-8") as src, open(os.path.join(scripts, name), "w", encoding="utf-8") as dst:
                dst.write(src.read())
        # La caja, aquí, es esta misma máquina.
        remote = os.path.join(scripts, "remote.sh")
        with open(remote, "w", encoding="utf-8") as handle:
            handle.write('#!/usr/bin/env bash\nshift\nexec "$@"\n')
        os.chmod(remote, 0o755)
        self.bin = os.path.join(self.root, "bin")
        os.makedirs(self.bin)
        self.git("init", "-q")
        self.write(".claude/hooks/hooks_test.py", "import sys\nsys.exit(0)\n")
        self.write("core/Cargo.toml", "")
        self.write("core/src/lib.rs", "")
        self.git("-c", "user.email=t@t", "-c", "user.name=t", "add", ".")
        self.git("-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false", "commit", "-qm", "base")

    def tearDown(self):
        self.dir.cleanup()

    def git(self, *args):
        subprocess.run(["git", "-C", self.root, *args], check=True, capture_output=True)

    def write(self, path, text):
        full = os.path.join(self.root, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8") as handle:
            handle.write(text)

    def fake_cargo(self, code, line):
        self.write("bin/cargo", f'#!/usr/bin/env bash\necho "{line}"\nexit {code}\n')
        os.chmod(os.path.join(self.bin, "cargo"), 0o755)

    def run_script(self, *args):
        env = {**os.environ, "PATH": self.bin + os.pathsep + os.environ["PATH"]}
        return subprocess.run(["bash", os.path.join(self.root, "scripts", "test-changed.sh"), *args], cwd=self.root, env=env, capture_output=True, text=True)

    def test_a_failing_local_suite_fails_the_script_and_names_it(self):
        self.write(".claude/hooks/hooks_test.py", "import sys\nsys.exit(1)\n")
        done = self.run_script()
        self.assertNotEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertIn("FALLÓ", done.stderr)
        self.assertIn("hooks_test.py", done.stderr)

    def test_a_green_local_suite_exits_zero(self):
        self.write(".claude/hooks/hooks_test.py", "import sys\nsys.exit(0)\n")
        done = self.run_script()
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_one_red_suite_among_green_ones_is_not_hidden(self):
        self.write(".claude/hooks/hooks_test.py", "import sys\nsys.exit(0)\n")
        self.write("scripts/test_changed_plan_test.py", "import sys\nsys.exit(1)\n")
        done = self.run_script()
        self.assertNotEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_a_failing_crate_in_the_box_fails_the_script(self):
        self.write("core/src/lib.rs", "pub fn x() {}\n")
        self.fake_cargo(101, "error: nope")
        done = self.run_script()
        self.assertNotEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertIn("FALLÓ", done.stderr)

    def test_a_failing_crate_with_quiet_output_still_fails_under_all(self):
        self.fake_cargo(101, "nothing the filter keeps")
        done = self.run_script("--all")
        self.assertNotEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_all_passes_when_cargo_does(self):
        self.fake_cargo(0, "test result: ok. 1 passed")
        done = self.run_script("--all")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_an_unknown_since_reference_is_an_error_not_nothing_to_test(self):
        done = self.run_script("--since", "no-such-ref")
        self.assertNotEqual(done.returncode, 0)
        self.assertNotIn("nada que probar", done.stdout)


if __name__ == "__main__":
    unittest.main()
