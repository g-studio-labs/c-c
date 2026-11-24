"""Qué pruebas tocan unos ficheros cambiados: lo usa `test-changed.sh`.

Lee las rutas por la entrada estándar (relativas a la raíz del repo) y
escribe el guion de bash que corre la caja. Vacío si nada de la app nativa
cambió. Las reglas, con su porqué, están en la cabecera de `test-changed.sh`.
"""

import os
import re
import subprocess
import sys

CRATES = ["voice", "core", "mcp", "ops", "desktop"]
# Quién depende de quién por `path = "../…"` en cada Cargo.toml.
DEPENDENTS = {
    "core": ["mcp", "ops", "desktop"],
    "mcp": ["ops", "desktop"],
    "ops": ["desktop"],
    "voice": ["desktop"],
    "desktop": [],
}
WHOLE = {"Cargo.toml", "build.rs", "src/testing.rs"}
# Cambian casi siempre por una dependencia o un módulo nuevo, y eso lo prueban
# el fichero del módulo y la compilación; si cambió algo más, todo el crate.
ROOTS = {"src/lib.rs", "src/main.rs"}
DECLARATION = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(mod|use)\s+[\w:{}, *]+;\s*$|^\s*$|^\s*//")
TEST_ATTR = re.compile(r"#\[(?:gpui_kit::)?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+(\w+)")
INCLUDE = re.compile(r'include!\("([^"]+)"\)')


class Crate:
    def __init__(self):
        self.whole = False
        self.filters = set()
        self.test_files = set()
        self.all_test_files = False
        self.check = False

    def empty(self):
        return not (self.whole or self.filters or self.test_files or self.all_test_files)


def included_files(crate):
    """Los `*_tests.rs` que otro fichero mete con `include!`: su módulo no es
    el suyo, así que se corren por el nombre de cada prueba."""
    out = set()
    for base, _, files in os.walk(os.path.join(crate, "src")):
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(base, name)
            with open(path, encoding="utf-8") as handle:
                for target in INCLUDE.findall(handle.read()):
                    out.add(os.path.normpath(os.path.join(os.path.dirname(path), target)))
    return out


def module_of(rel):
    """`src/platform/provision.rs` → `platform::provision`."""
    parts = rel[len("src/"):-len(".rs")].split("/")
    if parts[-1] == "mod":
        parts = parts[:-1]
    return "::".join(parts)


def only_declarations(path):
    """Si lo que cambió de `lib.rs`/`main.rs` son sólo líneas `mod`/`use`."""
    base = os.environ.get("SINCE") or "HEAD"
    diff = subprocess.run(["git", "diff", base, "--", path], capture_output=True, text=True).stdout
    if not diff and not subprocess.run(["git", "ls-files", "--error-unmatch", path], capture_output=True).returncode:
        return True
    lines = [l[1:] for l in diff.splitlines() if l[:1] in "+-" and not l.startswith(("+++", "---"))]
    return bool(lines) and all(DECLARATION.match(l) for l in lines)


def tests_in(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return TEST_ATTR.findall(handle.read())
    except OSError:
        return []


def main():
    changed = [line.strip() for line in sys.stdin if line.strip()]
    plan = {name: Crate() for name in CRATES}
    touched = set()
    includes = {}

    for path in changed:
        top, _, rel = path.partition("/")
        if path.startswith("app/src/locales/"):
            plan["core"].filters.add("texts::")
            touched.add("core")
            continue
        if top not in plan or not rel:
            continue
        crate = plan[top]
        if rel == "Cargo.lock" or (rel in ROOTS and only_declarations(path)):
            touched.add(top)
            crate.check = True
            continue
        if rel in WHOLE or rel in ROOTS or rel.startswith("tests/fixtures/"):
            if rel.startswith("tests/fixtures/"):
                crate.all_test_files = True
            else:
                crate.whole = True
            touched.add(top)
            continue
        if rel.startswith("tests/") and rel.endswith(".rs"):
            crate.test_files.add(os.path.basename(rel)[:-3])
            touched.add(top)
            continue
        if not (rel.startswith("src/") and rel.endswith(".rs")):
            continue
        touched.add(top)
        if top not in includes:
            includes[top] = included_files(top)
        if os.path.normpath(path) in includes[top]:
            names = tests_in(path)
            if names:
                crate.filters.update(names)
            else:
                crate.whole = True
            continue
        module = module_of(rel)
        crate.filters.add(module + "::")
        if top == "core":
            stem = module.split("::")[-1]
            if os.path.exists(os.path.join("core", "tests", stem + ".rs")):
                crate.test_files.add(stem)
            if os.path.exists(os.path.join("desktop", "src", stem + ".rs")):
                plan["desktop"].filters.add(stem + "::")

    check = set()
    for name in touched:
        check.update(DEPENDENTS[name])

    lines = []
    for name in CRATES:
        crate = plan[name]
        if not os.path.isdir(name):
            continue
        steps = []
        if crate.whole:
            steps.append("cargo test --quiet")
        else:
            if crate.filters:
                targets = [flag for flag, file in (("--lib", "lib.rs"), ("--bins", "main.rs")) if os.path.exists(os.path.join(name, "src", file))]
                steps.append(f"cargo test --quiet {' '.join(targets)} -- " + " ".join(sorted(crate.filters)))
            if crate.all_test_files:
                steps.append("cargo test --quiet --tests")
            for test in sorted(crate.test_files):
                steps.append(f"cargo test --quiet --test {test}")
        if not steps and (name in check or crate.check):
            steps.append("cargo check --quiet --tests")
        if not steps:
            continue
        # `-E` y no `-q`: lo que no casa se calla, pero el código de salida
        # de cargo es el que manda (PIPESTATUS).
        joined = " && ".join(f"{{ {step} 2>&1 | grep -E '^test .* FAILED|panicked|test result|^error'; test ${{PIPESTATUS[0]}} -eq 0; }}" for step in steps)
        lines.append(f'echo "== {name}"; (cd {name} && {joined}) || fail=1')
    if lines:
        lines.insert(0, "fail=0")
        lines.append('exit "$fail"')
    print("\n".join(lines))


if __name__ == "__main__":
    main()
