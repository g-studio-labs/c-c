"""Filtra la salida de `cargo test` y vigila que una prueba con filtro corriera algo.

Lee la salida por la entrada estándar y escribe las mismas líneas que antes
dejaba pasar el `grep` del plan. Con `--expect-tests` (el plan lo pone cuando
el fichero cambiado tiene pruebas), sale con 1 si en todo el paso no corrió
ni una y el filtro dejó fuera alguna: es la firma de un binario viejo, que
no conoce el módulo nuevo (25-nov-2026, command-and-control-514).

Va metido en el plan con `-c`, no llamado por ruta: la caja sólo tiene lo que
trae el worktree, y uno con una rama vieja no tendría este fichero.
"""

import re
import sys

KEEP = re.compile(r"^test .* FAILED|panicked|test result|^error")
RESULT = re.compile(r"test result: \w+\. (\d+) passed; (\d+) failed;.*?(\d+) filtered out")

MESSAGE = (
    "cargo_guard: el filtro no corrió ninguna prueba ({filtered} descartadas) y el fichero "
    "cambiado las tiene. Es lo que pasa cuando cargo compila un binario viejo: corre "
    "`touch` de los ficheros de ese módulo, o avisa en command-and-control-514."
)


def verdict(lines, expect_tests):
    """Devuelve el mensaje de fallo, o None si todo cuadra."""
    ran = filtered = results = 0
    for line in lines:
        found = RESULT.search(line)
        if found:
            results += 1
            ran += int(found.group(1)) + int(found.group(2))
            filtered += int(found.group(3))
    if expect_tests and results and ran == 0 and filtered > 0:
        return MESSAGE.format(filtered=filtered)
    return None


def main():
    lines = sys.stdin.read().splitlines()
    for line in lines:
        if KEEP.search(line):
            print(line)
    message = verdict(lines, "--expect-tests" in sys.argv)
    if message:
        print(message)
        sys.exit(1)


if __name__ == "__main__":
    main()
