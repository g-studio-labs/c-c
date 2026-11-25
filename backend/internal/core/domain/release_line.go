package domain

import (
	"slices"
	"sort"
	"strings"
	"time"
	"unicode"
)

// Qué artefacto lleva cada commit (command-and-control-526).
//
// Del repo de cac salen tres cosas con tres líneas de etiquetas: la app de
// Tauri (`v*`), la app nativa (`desktop-v*`) y el MCP suelto (`mcp-v*`). Hasta
// el 25-nov-2026 un commit contaba como publicado en cuanto caía bajo
// cualquier etiqueta, y las tareas de la nativa salían «ya salió en v1.35.0»
// sin que la nativa tuviera release.

// CacRepoID: el repo de cac en GitHub (hoy g-studio-labs/c-c). Por id y no por
// nombre: ya cambió de org una vez, y el id no cambia con eso.
const CacRepoID int64 = 1182990045

// cacArtifactLines: por carpeta de primer nivel del repo de cac, las líneas de
// release que la llevan dentro. Sale de los `Cargo.toml` y de qué compila cada
// workflow de release:
//
//   - `app/src-tauri` depende de core, mcp y ops; `desktop/` de core, mcp, ops
//     y voice; `mcp/` de core. `ops/` depende de mcp, no al revés.
//   - `core/` también entra en la app de Tauri y en el MCP, pero lo que cambia
//     ahí es de la nativa: contarlo con `v*` es justo el fallo que esto
//     arregla, así que sólo sale con `desktop-v*` (el criterio de la tarea).
//   - `mcp/` sale suelto (`mcp-v*`) y dentro de las dos apps (`cac --mcp`).
//
// Lo que no está (backend/, infra/, swarm-manage/, docs/, scripts/, .claude/,
// los ficheros de la raíz…) no sale con ninguna etiqueta: se despliega con el
// push o no se publica. Un commit que sólo toca eso no cuenta nunca como
// publicado por una release; para esas tareas manda el handoff.
var cacArtifactLines = map[string][]string{
	"app":     {"v"},
	"desktop": {"desktop-v"},
	"core":    {"desktop-v"},
	"voice":   {"desktop-v"},
	"ops":     {"v", "desktop-v"},
	"mcp":     {"v", "mcp-v", "desktop-v"},
}

// artifactLinesOf: la tabla de ese repo, y si la tiene. Sólo el de cac: en el
// resto de repos de las orgs una release es una release y lleva todo lo que
// trae (la regla de antes).
func artifactLinesOf(repoID int64) (map[string][]string, bool) {
	if repoID == CacRepoID {
		return cacArtifactLines, true
	}
	return nil, false
}

// TrackedRepoIDs: los repos con tabla.
func TrackedRepoIDs() []int64 { return []int64{CacRepoID} }

// TracksArtifacts: si en ese repo un commit necesita saber qué carpetas toca
// para saber si salió. Es lo que decide qué commits se rellenan.
func TracksArtifacts(repoID int64) bool {
	_, ok := artifactLinesOf(repoID)
	return ok
}

// ReleaseLine: la línea de una etiqueta, lo que va antes del número:
// `v1.35.0` → `v`, `desktop-v0.1.0` → `desktop-v`, `mcp-v1.33.0` → `mcp-v`.
func ReleaseLine(tag string) string {
	i := strings.IndexFunc(tag, unicode.IsDigit)
	if i < 0 {
		return tag
	}
	return tag[:i]
}

// TopDirs: las carpetas de primer nivel de esos ficheros, sin repetir y en
// orden. Un fichero de la raíz no tiene carpeta y no suma nada.
func TopDirs(paths []string) []string {
	seen := map[string]bool{}
	var out []string
	for _, p := range paths {
		i := strings.IndexByte(p, '/')
		if i <= 0 {
			continue
		}
		if d := p[:i]; !seen[d] {
			seen[d] = true
			out = append(out, d)
		}
	}
	sort.Strings(out)
	return out
}

// CommitDirs: qué carpetas de primer nivel toca un commit (ver TopDirs). Se
// guarda al llegar el push y, para los commits de antes, se rellena
// preguntándole a GitHub.
//
// Se guardan las carpetas y no el artefacto ya resuelto a propósito: así la
// tabla carpeta → líneas vive sólo en cacArtifactLines y se aplica al leer.
// Si cambia (un crate nuevo, una dependencia), no hay que reescribir nada.
//
// Una fila por commit, no una columna en `task_git_links`: el mismo commit
// puede estar en dos tareas y en un incidente, y es el mismo dato.
//
// Que exista la fila es saberlo; Dirs vacío es «sólo toca la raíz». Sin fila
// no se sabe, y un commit que no se sabe no cuenta como publicado.
type CommitDirs struct {
	RepoID int64  `gorm:"primaryKey;autoIncrement:false" json:"repoId"`
	SHA    string `gorm:"column:sha;primaryKey;type:varchar(40)" json:"sha"`
	// Dirs: las carpetas separadas por comas (`core,desktop`).
	Dirs      string    `gorm:"type:text;not null;default:''" json:"dirs"`
	CreatedAt time.Time `json:"createdAt"`
}

func (CommitDirs) TableName() string { return "commit_dirs" }

// JoinDirs / SplitDirs: cómo viajan las carpetas en CommitDirs.Dirs.
func JoinDirs(dirs []string) string { return strings.Join(dirs, ",") }

func SplitDirs(s string) []string {
	if s == "" {
		return nil
	}
	return strings.Split(s, ",")
}

// CommitShipped: en qué release quedó publicado un commit, o nil.
//
//   - first: la primera release del repo, de cualquier línea, que lo trajo
//     (la que tiene su fila en `release_commits`); nil si ninguna.
//   - dirs: sus carpetas; nil si no se saben (y no se sabe no es salir).
//   - releases: todas las releases del repo.
//
// En un repo sin tabla, first y ya (la regla de antes). En el de cac, cada
// artefacto que toca tiene que haber salido en una release de alguna de sus
// líneas publicada no antes que first, y cuenta la más tardía de esas: es
// cuando quedó entero fuera.
//
// «No antes que first» es «la contiene» porque todas las releases se cortan
// desde main, en orden (`gh release create --target $GITHUB_SHA` en los
// workflows, y `/soltar`): una release lleva todo lo que llevaba la anterior.
// Por eso un commit que llegó primero en una `v*` también está en la
// `desktop-v*` siguiente, aunque `compare` no lo vuelva a listar ahí.
func CommitShipped(repoID int64, first *ReleaseRef, dirs []string, releases []ReleaseRef) *ReleaseRef {
	if first == nil {
		return nil
	}
	table, ok := artifactLinesOf(repoID)
	if !ok {
		return first
	}
	var out *ReleaseRef
	touched := 0
	for _, d := range dirs {
		lines, ok := table[d]
		if !ok {
			continue
		}
		touched++
		var earliest *ReleaseRef
		for i := range releases {
			r := releases[i]
			if r.At.Before(first.At) || !slices.Contains(lines, ReleaseLine(r.Tag)) {
				continue
			}
			if earliest == nil || r.At.Before(earliest.At) {
				earliest = &r
			}
		}
		if earliest == nil {
			return nil
		}
		if out == nil || earliest.At.After(out.At) {
			out = earliest
		}
	}
	if touched == 0 {
		return nil
	}
	return out
}
