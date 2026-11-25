package domain

import "time"

// Release: una release publicada en GitHub de un repo de la org, con los
// commits que lleva (los de entre la anterior y ésta).
//
// Es lo que deja a cac contestar «¿esto ya salió?» sin que nadie se acuerde
// de mirarlo: hasta el 2-nov-2026, una tarea commiteada y sin release se
// quedaba «en curso» hasta que una sesión volvía a comprobarlo con
// `git tag --contains`, y un incidente «abierto hasta la release» igual
// (command-and-control-295).
//
// **Marca y avisa; no mueve ni cierra nada** (decidido con jose el
// 2-nov-2026). Sigue la regla de la casa de `TaskGitLink`: lo que llega de
// GitHub sólo se apunta. Cada commit sabe en qué release salió
// (`TaskGitLink.ReleasedIn`), una tarea en curso o un incidente con todo
// publicado lo dice (`Shipped`), y quien la cierra es la sesión o la persona.
//
// Nada de esto se escribe en `task_git_links` ni en `incident_commits`: se
// calcula al leer, cruzando por (repo, sha). Así una release borrada o vuelta
// a cortar no deja nada colgado.
type Release struct {
	BaseModel
	OrgID        string    `gorm:"type:varchar(36);index;not null" json:"orgId"`
	RepoID       int64     `gorm:"not null;uniqueIndex:idx_releases_tag,priority:1" json:"repoId"`
	RepoFullName string    `gorm:"type:varchar(200);not null" json:"repoFullName"`
	Tag          string    `gorm:"type:varchar(120);not null;uniqueIndex:idx_releases_tag,priority:2" json:"tag"`
	Name         string    `gorm:"type:varchar(300)" json:"name"`
	HTMLURL      string    `gorm:"column:html_url;type:varchar(400)" json:"htmlUrl"`
	AuthorLogin  string    `gorm:"type:varchar(120)" json:"authorLogin"`
	PublishedAt  time.Time `gorm:"index;not null" json:"publishedAt"`
	// PrevTag: la release contra la que se compararon los commits. Vacía en la
	// primera de un repo: sin base no se sabe qué trae, y no se inventa.
	PrevTag     string `gorm:"type:varchar(120)" json:"prevTag"`
	CommitCount int    `json:"commitCount"`
	// Truncated: trae más commits de los que se leen (MaxReleaseCommits).
	Truncated bool `json:"truncated"`
	// ListedByLine: sus commits se compararon contra la anterior de su misma
	// línea (ver ReleaseLine). Las guardadas antes del 25-nov-2026 se
	// comparaban contra la anterior de cualquier línea, y una vuelta en
	// segundo plano las vuelve a listar una vez (command-and-control-526).
	ListedByLine bool `gorm:"not null;default:false" json:"-"`
}

func (Release) TableName() string { return "releases" }

// ReleaseCommit: un commit que lleva una release. Único por (release, sha):
// el mismo commit está en cada release de cada línea que lo lleva (una `v*` y
// la `desktop-v*` siguiente, por ejemplo), porque «salió» se pregunta por
// línea (command-and-control-526). Hasta el 25-nov-2026 era único por (repo,
// sha) y un commit sólo constaba en la primera release que lo traía.
type ReleaseCommit struct {
	ID        string `gorm:"primaryKey;type:varchar(36)" json:"id"`
	ReleaseID string `gorm:"type:varchar(36);index;not null;uniqueIndex:idx_release_commits_release_sha,priority:1" json:"releaseId"`
	RepoID    int64  `gorm:"not null;index:idx_release_commits_repo_sha,priority:1" json:"repoId"`
	SHA       string `gorm:"column:sha;type:varchar(40);not null;uniqueIndex:idx_release_commits_release_sha,priority:2;index:idx_release_commits_repo_sha,priority:2" json:"sha"`
	Title     string `gorm:"type:varchar(200)" json:"title"`
	HTMLURL   string `gorm:"column:html_url;type:varchar(400)" json:"htmlUrl"`
	// Inherited: no lo listó su `compare`, lo hereda. La primera release de
	// una línea lleva todo lo que llevaban las de antes de su base (ver
	// GitHubRepository.SaveReleaseInheriting). No se anuncia en los hilos:
	// serían cientos de líneas en tareas que salieron hace meses.
	Inherited bool      `gorm:"not null;default:false" json:"-"`
	CreatedAt time.Time `json:"createdAt"`
}

func (ReleaseCommit) TableName() string { return "release_commits" }

// ReleaseRef: en qué release salió algo. Es lo que viaja como `releasedIn`
// (un commit) y como `shipped` (una tarea o un incidente).
type ReleaseRef struct {
	Tag string    `json:"tag"`
	At  time.Time `json:"at"`
	URL string    `json:"url,omitempty"`
}

// MaxReleaseCommits: cuántos commits de una release se leen (una página de
// `compare`). Una release de la app sale por feature (scripts/next-version.sh)
// y no llega; si llegara, la release lo dice (`Truncated`).
const MaxReleaseCommits = 250

// ShippedOf: si algo con `total` commits enlazados ya salió entero, y en qué
// release. Nil si no tiene commits (nada que haya podido salir) o si alguno
// no salió todavía; si salieron todos, la release más reciente de ellos, que
// es cuando quedó entero fuera.
//
// Es la definición de «salió» en un solo sitio: la usan las tareas del mapa y
// los incidentes. Cada commit llega ya resuelto con CommitShipped.
func ShippedOf(total int, released []ReleaseRef) *ReleaseRef {
	if total == 0 || len(released) < total {
		return nil
	}
	latest := released[0]
	for _, r := range released[1:] {
		if r.At.After(latest.At) {
			latest = r
		}
	}
	return &latest
}
