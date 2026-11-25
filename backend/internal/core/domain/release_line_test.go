package domain

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"sort"
	"strings"
	"testing"
	"time"
)

// Lo que pide la tarea (command-and-control-526): con qué etiqueta sale cada
// commit del repo de cac según lo que toca, mirando qué releases lo llevan.
//
// Roturas que atrapa: la regla vieja (cualquier etiqueta vale), contar un
// commit de varios artefactos cuando sale el primero y no el último, tomar
// la última release de la línea que lo lleva en vez de la primera, contar una
// release por su fecha sin llevarlo, dejar sin salir lo que no está en la
// tabla (backend/, la raíz) en vez de darle `v*`, y contar un commit sin
// carpetas sabidas.
func TestACommitShipsWhenEveryArtifactItTouchesHasShipped(t *testing.T) {
	day := time.Date(2026, 11, 25, 0, 0, 0, 0, time.UTC)
	at := func(h int) time.Time { return day.Add(time.Duration(h) * time.Hour) }
	v135 := ReleaseRef{Tag: "v1.35.0", At: at(0)}
	mcp := ReleaseRef{Tag: "mcp-v1.33.0", At: at(1)}
	desk := ReleaseRef{Tag: "desktop-v0.1.0", At: at(2)}
	v136 := ReleaseRef{Tag: "v1.36.0", At: at(3)}
	desk2 := ReleaseRef{Tag: "desktop-v0.2.0", At: at(4)}
	v137 := ReleaseRef{Tag: "v1.37.0", At: at(5)}

	cases := []struct {
		name       string
		dirs       []string
		containing []ReleaseRef
		want       string
	}{
		{"desktop/ en v*", []string{"desktop"}, []ReleaseRef{v135}, ""},
		{"desktop/ en desktop-v*", []string{"desktop"}, []ReleaseRef{v135, desk}, "desktop-v0.1.0"},
		{"desktop/: la primera desktop-v* que lo lleva", []string{"desktop"}, []ReleaseRef{v135, desk2, desk}, "desktop-v0.1.0"},
		{"core/ en v*", []string{"core"}, []ReleaseRef{v135}, ""},
		{"voice/ en v*", []string{"voice"}, []ReleaseRef{v135}, ""},
		{"mcp/ en v*", []string{"mcp"}, []ReleaseRef{v135}, "v1.35.0"},
		{"mcp/ en mcp-v*", []string{"mcp"}, []ReleaseRef{mcp}, "mcp-v1.33.0"},
		{"app/ en v*", []string{"app"}, []ReleaseRef{v135}, "v1.35.0"},
		{"app/: la primera v* que lo lleva", []string{"app"}, []ReleaseRef{v137, v136}, "v1.36.0"},
		{"app/ en desktop-v*", []string{"app"}, []ReleaseRef{desk}, ""},
		{"app/src/index.css sin desktop-v*", []string{"app", "app/src/index.css"}, []ReleaseRef{v135}, ""},
		{"app/src/index.css con las dos", []string{"app", "app/src/index.css"}, []ReleaseRef{v135, desk}, "desktop-v0.1.0"},
		{".claude/ en mcp-v*", []string{".claude"}, []ReleaseRef{mcp}, "mcp-v1.33.0"},
		{".claude/ en desktop-v*", []string{".claude"}, []ReleaseRef{desk}, "desktop-v0.1.0"},
		{"core/ y desktop/ en v*", []string{"core", "desktop"}, []ReleaseRef{v135}, ""},
		{"app/ y desktop/: el último que sale", []string{"app", "desktop"}, []ReleaseRef{v135, desk}, "desktop-v0.1.0"},
		{"app/ y desktop/ al revés", []string{"app", "desktop"}, []ReleaseRef{desk, v136}, "v1.36.0"},
		{"app/ y desktop/: una desktop-v* posterior que no lo lleva", []string{"app", "desktop"}, []ReleaseRef{v136}, ""},
		{"backend/ en v*", []string{"backend", "docs", "infra"}, []ReleaseRef{v135}, "v1.35.0"},
		{"backend/ sólo en desktop-v* y mcp-v*", []string{"backend"}, []ReleaseRef{desk, mcp}, ""},
		{"backend/ y desktop/", []string{"backend", "desktop"}, []ReleaseRef{v135, desk}, "desktop-v0.1.0"},
		{"sólo la raíz en v*", []string{}, []ReleaseRef{v135}, "v1.35.0"},
		{"sólo la raíz en desktop-v*", []string{}, []ReleaseRef{desk}, ""},
		{"carpetas sin saber", nil, []ReleaseRef{v135, desk, mcp}, ""},
		{"en ninguna", []string{"app"}, nil, ""},
	}
	for _, c := range cases {
		got := CommitShipped(CacRepoID, c.dirs, c.containing)
		tag := ""
		if got != nil {
			tag = got.Tag
		}
		if tag != c.want {
			t.Errorf("%s: salió en %q, se esperaba %q", c.name, tag, c.want)
		}
	}
}

// En el resto de repos de las orgs no hay tabla: sale con la primera release
// que lo lleva, sepa o no cac qué carpetas toca.
//
// Roturas que atrapa: aplicar la tabla de cac a todos los repos (nada de otro
// repo saldría nunca, porque sus carpetas no son las de cac), y quedarse con
// la release que llega última en vez de la más temprana.
func TestAnotherRepoShipsWithItsFirstRelease(t *testing.T) {
	now := time.Now()
	later := ReleaseRef{Tag: "v2.1.0", At: now.Add(time.Hour)}
	first := ReleaseRef{Tag: "v2.0.0", At: now}
	if got := CommitShipped(100, nil, []ReleaseRef{later, first}); got == nil || got.Tag != "v2.0.0" {
		t.Errorf("otro repo: %+v", got)
	}
	if TracksArtifacts(100) || !TracksArtifacts(CacRepoID) {
		t.Error("sólo el repo de cac tiene tabla")
	}
}

// Roturas que atrapa: confundir la línea con la etiqueta entera, contar un
// fichero de la raíz como carpeta, o repetir carpetas.
func TestReleaseLinesAndTopDirs(t *testing.T) {
	for tag, want := range map[string]string{"v1.35.0": "v", "desktop-v0.1.0": "desktop-v", "mcp-v1.33.0": "mcp-v"} {
		if got := ReleaseLine(tag); got != want {
			t.Errorf("ReleaseLine(%q) = %q, se esperaba %q", tag, got, want)
		}
	}
	got := TopDirs([]string{"desktop/src/a.rs", "CLAUDE.md", "core/x.rs", "desktop/b.rs", "/raro"})
	if fmt.Sprint(got) != "[core desktop]" {
		t.Errorf("TopDirs = %v", got)
	}
	paths := []string{"app/src/index.css", "app/src/main.ts", ".claude/hooks/cac-edit", "app/src/index.css"}
	if got := CommitKeys(CacRepoID, paths); fmt.Sprint(got) != "[.claude app app/src/index.css]" {
		t.Errorf("CommitKeys(cac) = %v", got)
	}
	if got := CommitKeys(100, paths); fmt.Sprint(got) != "[.claude app]" {
		t.Errorf("CommitKeys(otro repo) = %v", got)
	}
	if got := SplitDirs(""); got == nil || len(got) != 0 {
		t.Errorf("SplitDirs(\"\") = %#v: una fila vacía es «sólo la raíz», no «no se sabe»", got)
	}
}

// La tabla sale de los Cargo.toml (qué crates compila cada línea de release,
// siguiendo las dependencias por ruta) y de los `include_str!` /
// `include_bytes!` de esos crates que se llevan ficheros de otra carpeta de
// primer nivel: esa carpeta sale también con la línea. Si el fichero tiene
// clave propia en la tabla (`app/src/index.css`), la línea va a esa clave y
// no a toda la carpeta. Sólo cuenta el código que se compila en la release:
// lo que va detrás del primer `#[cfg(test)]` de un fichero, y `tests/`, no.
//
// La única excepción es a propósito y está en la tabla: core/ entra también
// en la app de Tauri y en el MCP, pero sólo cuenta con la nativa.
//
// Rotura que atrapa: una línea de más o de menos en cacArtifactLines, una
// dependencia nueva entre crates que la tabla no sigue, o un fichero de otra
// carpeta metido en un binario (los hooks de `.claude/` en el MCP, el CSS de
// la app en la nativa) sin que su carpeta salga con esa línea.
func TestTheArtifactTableFollowsTheCrates(t *testing.T) {
	root := filepath.Join("..", "..", "..", "..")
	dep := regexp.MustCompile(`path\s*=\s*"((?:\.\./)+)([a-z-]+)"`)
	include := regexp.MustCompile(`include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)`)
	topOf := func(p string) string {
		p = filepath.ToSlash(p)
		if i := strings.IndexByte(p, '/'); i > 0 {
			return p[:i]
		}
		return p
	}
	var closure func(dir string, crates map[string]bool)
	closure = func(dir string, crates map[string]bool) {
		crates[dir] = true
		b, err := os.ReadFile(filepath.Join(root, dir, "Cargo.toml"))
		if err != nil {
			t.Fatal(err)
		}
		for _, m := range dep.FindAllStringSubmatch(string(b), -1) {
			if !crates[m[2]] {
				closure(m[2], crates)
			}
		}
	}
	// includes: lo que el crate se lleva de fuera de su carpeta, como ruta
	// desde la raíz del repo.
	includes := func(crate string) []string {
		var out []string
		src := filepath.Join(root, crate, "src")
		err := filepath.WalkDir(src, func(path string, d os.DirEntry, err error) error {
			if err != nil || d.IsDir() || !strings.HasSuffix(path, ".rs") {
				return err
			}
			b, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			code, _, _ := strings.Cut(string(b), "#[cfg(test)]")
			for _, m := range include.FindAllStringSubmatch(code, -1) {
				rel, err := filepath.Rel(root, filepath.Join(filepath.Dir(path), m[1]))
				if err != nil {
					return err
				}
				if rel = filepath.ToSlash(rel); topOf(rel) != topOf(crate) {
					out = append(out, rel)
				}
			}
			return nil
		})
		if err != nil {
			t.Fatal(err)
		}
		return out
	}
	lines := map[string]string{"v": "app/src-tauri", "desktop-v": "desktop", "mcp-v": "mcp"}
	want := map[string][]string{}
	add := func(key, line string) {
		if !slices.Contains(want[key], line) {
			want[key] = append(want[key], line)
		}
	}
	for line, crate := range lines {
		crates := map[string]bool{}
		closure(crate, crates)
		for c := range crates {
			if d := topOf(c); d != "core" || line == "desktop-v" {
				add(d, line)
			}
			for _, f := range includes(c) {
				if _, own := cacArtifactLines[f]; own {
					add(f, line)
				} else {
					add(topOf(f), line)
				}
			}
		}
	}
	for d := range want {
		sort.Strings(want[d])
	}
	got := map[string][]string{}
	for d, ls := range cacArtifactLines {
		got[d] = slices.Sorted(slices.Values(ls))
	}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("la tabla dice\n  %v\nlos Cargo.toml y los include dicen\n  %v", got, want)
	}
}
