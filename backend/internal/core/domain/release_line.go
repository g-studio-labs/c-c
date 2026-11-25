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
// release que la llevan dentro. Sale de los `Cargo.toml`, de los
// `include_str!`/`include_bytes!` que se llevan ficheros de otra carpeta, y de
// qué compila cada workflow de release:
//
//   - `app/src-tauri` depende de core, mcp y ops; `desktop/` de core, mcp, ops
//     y voice; `mcp/` de core. `ops/` depende de mcp, no al revés.
//   - `core/` también entra en la app de Tauri y en el MCP, pero lo que cambia
//     ahí es de la nativa: contarlo con `v*` es justo el fallo que esto
//     arregla, así que sólo sale con `desktop-v*` (el criterio de la tarea).
//   - `mcp/` sale suelto (`mcp-v*`) y dentro de las dos apps (`cac --mcp`).
//   - `.claude/`: el MCP mete los hooks en el binario (`hooks_install.rs`).
//   - Una clave con `/` es un fichero suelto que otro artefacto se lleva:
//     `app/src/index.css` lo compila la nativa (`desktop/src/theme.rs`). Se
//     apunta aparte en vez de darle `desktop-v*` a toda `app/`, para que un
//     commit sólo de Tauri no espere a la nativa por ese fichero. Cuenta además
//     de su carpeta: tocarlo pide `v*` (por `app`) y `desktop-v*`.
//
// Lo que no está (backend/, infra/, docs/, scripts/, los ficheros de la
// raíz…) sale con `v*` (cacRestLines): así una tarea mixta, con su commit de
// backend o de STATUS.md, sale cuando sale la app, como antes.
var cacArtifactLines = map[string][]string{
	"app":               {"v"},
	"app/src/index.css": {"desktop-v"},
	"desktop":           {"desktop-v"},
	"core":              {"desktop-v"},
	"voice":             {"desktop-v"},
	"ops":               {"v", "desktop-v"},
	"mcp":               {"v", "mcp-v", "desktop-v"},
	".claude":           {"v", "mcp-v", "desktop-v"},
}

// cacRestLines: las líneas de lo que no está en cacArtifactLines.
var cacRestLines = []string{"v"}

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

// CommitKeys: lo que se apunta de un commit de ese repo: sus carpetas de
// primer nivel y, en el de cac, los ficheros sueltos de la tabla que toca
// (`app/src/index.css`). En orden y sin repetir.
func CommitKeys(repoID int64, paths []string) []string {
	out := TopDirs(paths)
	table, ok := artifactLinesOf(repoID)
	if !ok {
		return out
	}
	for _, p := range paths {
		if _, shared := table[p]; shared && strings.Contains(p, "/") && !slices.Contains(out, p) {
			out = append(out, p)
		}
	}
	sort.Strings(out)
	return out
}

// CommitDirs: qué carpetas de primer nivel toca un commit (ver CommitKeys). Se
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

// SplitDirs nunca da nil: una fila con Dirs vacío es «sólo la raíz», que se
// sabe, y nil es «no se sabe».
func SplitDirs(s string) []string {
	if s == "" {
		return []string{}
	}
	return strings.Split(s, ",")
}

// CommitShipped: en qué release quedó publicado un commit, o nil.
//
//   - dirs: lo que toca (CommitKeys); nil si no se sabe, y no saberlo no es
//     salir. Vacío es que sólo toca la raíz.
//   - containing: las releases del repo que lo llevan, de cualquier línea (las
//     que tienen su fila en `release_commits`).
//
// Es contenido, no fechas: una release cuenta para un commit si lo lleva, no
// si se publicó después. Los workflows de la nativa y del MCP fijan el sha al
// lanzarse y publican al terminar, así que una `desktop-v*` puede salir
// después de una `v*` sin llevar lo que lleva ésa.
//
// En un repo sin tabla, la primera que lo lleva. En el de cac, cada cosa que
// toca tiene que ir en una release de alguna de sus líneas: de cada una
// cuenta la primera que lo lleva, y de todas la más tardía, que es cuando
// quedó entero fuera.
func CommitShipped(repoID int64, dirs []string, containing []ReleaseRef) *ReleaseRef {
	if len(containing) == 0 {
		return nil
	}
	table, ok := artifactLinesOf(repoID)
	if !ok {
		return earliestIn(containing, nil)
	}
	if dirs == nil {
		return nil
	}
	keys := dirs
	if len(keys) == 0 {
		keys = []string{""}
	}
	var out *ReleaseRef
	for _, k := range keys {
		lines, ok := table[k]
		if !ok {
			lines = cacRestLines
		}
		first := earliestIn(containing, lines)
		if first == nil {
			return nil
		}
		if out == nil || first.At.After(out.At) {
			out = first
		}
	}
	return out
}

// earliestIn: la más temprana de esas releases que es de alguna de esas
// líneas (de cualquiera con lines nil), o nil.
func earliestIn(releases []ReleaseRef, lines []string) *ReleaseRef {
	var out *ReleaseRef
	for i := range releases {
		r := releases[i]
		if lines != nil && !slices.Contains(lines, ReleaseLine(r.Tag)) {
			continue
		}
		if out == nil || r.At.Before(out.At) {
			out = &r
		}
	}
	return out
}
