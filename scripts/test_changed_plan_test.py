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

FILES = {
    "core/Cargo.toml": "",
    "core/src/lib.rs": "pub mod dates;\npub mod chat;\n\npub fn shared() {}\n",
    "core/src/dates.rs": "",
    "core/src/chat.rs": "",
    "core/tests/chat.rs": "",
    "mcp/src/lib.rs": "",
    "ops/src/lib.rs": "",
    "desktop/src/main.rs": "mod chat;\nmod workspace;\n",
    "desktop/src/chat.rs": "",
    "desktop/src/workspace.rs": 'mod tests {\n    include!("docs_tests.rs");\n}\n',
    "desktop/src/docs_tests.rs": "#[gpui_kit::test]\nfn the_index_opens(cx: &mut T) {}\n\n#[test]\nfn plain_one() {}\n",
    "desktop/src/platform/mod.rs": "",
    "desktop/src/platform/provision.rs": "",
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

    def plan(self, *changed):
        out = subprocess.run([sys.executable, "-I", PLANNER], input="\n".join(changed), capture_output=True, text=True, cwd=self.dir.name, check=True)
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
        self.assertIn("|| fail=1", plan)
        self.assertIn("PIPESTATUS[0]", plan)
        self.assertTrue(plan.rstrip().endswith('exit "$fail"'))

    def test_nothing_native_plans_nothing(self):
        self.assertEqual(self.plan("backend/main.go", "app/src/App.tsx").strip(), "")


if __name__ == "__main__":
    unittest.main()
