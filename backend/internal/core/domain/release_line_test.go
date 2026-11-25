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
// commit del repo de cac según lo que toca.
//
// Roturas que atrapa: la regla vieja (cualquier etiqueta vale), contar un
// commit de varios artefactos cuando sale el primero y no el último, contar
// uno sin carpetas que salgan (backend/, docs/…) o sin carpetas sabidas, y
// tomar una release anterior a la que trajo el commit.
func TestACommitShipsWhenEveryArtifactItTouchesHasShipped(t *testing.T) {
	day := time.Date(2026, 11, 25, 0, 0, 0, 0, time.UTC)
	at := func(h int) time.Time { return day.Add(time.Duration(h) * time.Hour) }
	v135 := ReleaseRef{Tag: "v1.35.0", At: at(0)}
	desk := ReleaseRef{Tag: "desktop-v0.1.0", At: at(2)}
	mcp := ReleaseRef{Tag: "mcp-v1.33.0", At: at(1)}
	v136 := ReleaseRef{Tag: "v1.36.0", At: at(3)}
	old := ReleaseRef{Tag: "desktop-v0.0.9", At: at(-5)}

	cases := []struct {
		name     string
		first    ReleaseRef
		dirs     []string
		releases []ReleaseRef
		want     string
	}{
		{"desktop/ bajo v*", v135, []string{"desktop"}, []ReleaseRef{v135}, ""},
		{"desktop/ bajo desktop-v*", v135, []string{"desktop"}, []ReleaseRef{v135, desk}, "desktop-v0.1.0"},
		{"core/ bajo v*", v135, []string{"core"}, []ReleaseRef{v135}, ""},
		{"voice/ bajo v*", v135, []string{"voice"}, []ReleaseRef{v135}, ""},
		{"mcp/ bajo v*", v135, []string{"mcp"}, []ReleaseRef{v135}, "v1.35.0"},
		{"mcp/ bajo mcp-v*", mcp, []string{"mcp"}, []ReleaseRef{mcp}, "mcp-v1.33.0"},
		{"app/ bajo v*", v135, []string{"app"}, []ReleaseRef{v135}, "v1.35.0"},
		{"app/ bajo desktop-v*", desk, []string{"app"}, []ReleaseRef{desk}, ""},
		{"core/ y desktop/ bajo v*", v135, []string{"core", "desktop"}, []ReleaseRef{v135}, ""},
		{"app/ y desktop/: el último que sale", v135, []string{"app", "desktop"}, []ReleaseRef{v135, desk}, "desktop-v0.1.0"},
		{"app/ y desktop/ al revés", desk, []string{"app", "desktop"}, []ReleaseRef{desk, v136}, "v1.36.0"},
		{"backend/ bajo todas", v135, []string{"backend", "infra", "docs"}, []ReleaseRef{v135, desk, mcp, v136}, ""},
		{"backend/ y app/", v135, []string{"app", "backend"}, []ReleaseRef{v135}, "v1.35.0"},
		{"sólo la raíz", v135, []string{}, []ReleaseRef{v135, desk, mcp}, ""},
		{"carpetas sin saber", v135, nil, []ReleaseRef{v135, desk, mcp}, ""},
		{"app/ y desktop/ sin desktop-v*", v135, []string{"app", "desktop"}, []ReleaseRef{v135, v136}, ""},
		{"una desktop-v* de antes no lo lleva", v135, []string{"desktop"}, []ReleaseRef{old, v135}, ""},
	}
	for _, c := range cases {
		first := c.first
		got := CommitShipped(CacRepoID, &first, c.dirs, c.releases)
		tag := ""
		if got != nil {
			tag = got.Tag
		}
		if tag != c.want {
			t.Errorf("%s: salió en %q, se esperaba %q", c.name, tag, c.want)
		}
	}
	if CommitShipped(CacRepoID, nil, []string{"app"}, []ReleaseRef{v135}) != nil {
		t.Error("un commit que no está en ninguna release no salió")
	}
}

// En el resto de repos de las orgs no hay tabla: una release lleva lo que
// trae, sepa o no cac qué carpetas toca.
//
// Rotura que atrapa: aplicar la tabla de cac a todos los repos (nada de otro
// repo saldría nunca, porque sus carpetas no son las de cac).
func TestAnotherRepoShipsWithItsFirstRelease(t *testing.T) {
	first := ReleaseRef{Tag: "v2.0.0", At: time.Now()}
	if got := CommitShipped(100, &first, nil, nil); got == nil || got.Tag != "v2.0.0" {
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
}

// La tabla sale de los Cargo.toml: qué crates compila cada línea de release,
// siguiendo las dependencias por ruta. Si un crate gana o pierde una
// dependencia, esta prueba lo dice y la tabla se pone al día.
//
// La única excepción es a propósito y está en la tabla: core/ entra también
// en la app de Tauri y en el MCP, pero sólo cuenta con la nativa.
//
// Rotura que atrapa: una línea de más o de menos en cacArtifactLines, o una
// dependencia nueva entre crates que la tabla no sigue.
func TestTheArtifactTableFollowsTheCrates(t *testing.T) {
	root := filepath.Join("..", "..", "..", "..")
	dep := regexp.MustCompile(`path\s*=\s*"((?:\.\./)+)([a-z-]+)"`)
	var closure func(dir string, seen map[string]bool)
	closure = func(dir string, seen map[string]bool) {
		top := filepath.ToSlash(dir)
		if i := strings.IndexByte(top, '/'); i > 0 {
			top = top[:i]
		}
		seen[top] = true
		b, err := os.ReadFile(filepath.Join(root, dir, "Cargo.toml"))
		if err != nil {
			t.Fatal(err)
		}
		for _, m := range dep.FindAllStringSubmatch(string(b), -1) {
			if !seen[m[2]] {
				closure(m[2], seen)
			}
		}
	}
	lines := map[string]string{"v": "app/src-tauri", "desktop-v": "desktop", "mcp-v": "mcp"}
	want := map[string][]string{}
	for line, crate := range lines {
		seen := map[string]bool{}
		closure(crate, seen)
		for d := range seen {
			if d == "core" && line != "desktop-v" {
				continue
			}
			want[d] = append(want[d], line)
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
		t.Errorf("la tabla dice\n  %v\nlos Cargo.toml dicen\n  %v", got, want)
	}
}
