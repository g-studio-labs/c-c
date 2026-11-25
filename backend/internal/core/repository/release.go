package repository

import (
	"errors"
	"strings"
	"time"

	"github.com/google/uuid"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Releases de GitHub y sus commits (ver domain.Release): se guardan al
// publicarse y se cruzan al leer con los commits de las tareas y de los
// incidentes. Nada de aquí escribe en esas tablas.

// EnsureReleaseIndexes: el sha de un incidente puede ser corto (7 a 40), así
// que se busca por prefijo; con `varchar_pattern_ops` el `LIKE 'abc1234%'`
// usa el índice aunque la base no esté en colación C.
func EnsureReleaseIndexes(db *gorm.DB) error {
	return db.Exec(`CREATE INDEX IF NOT EXISTS idx_release_commits_sha_prefix
		ON release_commits (sha varchar_pattern_ops)`).Error
}

// FindRelease: la release de ese repo con ese tag; nil si no está.
func (r *GitHubRepository) FindRelease(repoID int64, tag string) (*domain.Release, error) {
	var rel domain.Release
	err := r.db.Where("repo_id = ? AND tag = ?", repoID, tag).First(&rel).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	return &rel, err
}

// PreviousRelease: la última release del repo publicada antes de `before`.
func (r *GitHubRepository) PreviousRelease(repoID int64, before time.Time) (*domain.Release, error) {
	var rel domain.Release
	err := r.db.Where("repo_id = ? AND published_at < ?", repoID, before).
		Order("published_at DESC").First(&rel).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil
	}
	return &rel, err
}

// SaveRelease guarda la release y sus commits de una vez. Si ya estaba (una
// reentrega que corre a la vez que otra), no escribe nada y lo dice.
//
// Un commit que ya salió en otra release no se mueve: sale en la primera que
// lo trajo (índice único por repo y sha).
func (r *GitHubRepository) SaveRelease(rel *domain.Release, commits []domain.ReleaseCommit) (bool, error) {
	created := false
	err := r.db.Transaction(func(tx *gorm.DB) error {
		res := tx.Clauses(clause.OnConflict{
			Columns:   []clause.Column{{Name: "repo_id"}, {Name: "tag"}},
			DoNothing: true,
		}).Create(rel)
		if res.Error != nil || res.RowsAffected == 0 {
			return res.Error
		}
		created = true
		for i := range commits {
			commits[i].ID = uuid.NewString()
			commits[i].ReleaseID = rel.ID
			commits[i].RepoID = rel.RepoID
		}
		if len(commits) == 0 {
			return nil
		}
		return tx.Clauses(clause.OnConflict{
			Columns:   []clause.Column{{Name: "repo_id"}, {Name: "sha"}},
			DoNothing: true,
		}).CreateInBatches(commits, 100).Error
	})
	return created, err
}

// UpdateReleaseMeta: una release editada en GitHub (su nombre o su enlace).
func (r *GitHubRepository) UpdateReleaseMeta(repoID int64, tag, name, url string) error {
	return r.db.Model(&domain.Release{}).Where("repo_id = ? AND tag = ?", repoID, tag).
		Updates(map[string]any{"name": name, "html_url": url, "updated_at": time.Now()}).Error
}

// DeleteRelease: una release borrada o despublicada deja de contar, con sus
// commits. Lo que se escribió en los hilos se queda: es historia.
func (r *GitHubRepository) DeleteRelease(repoID int64, tag string) error {
	return r.db.Transaction(func(tx *gorm.DB) error {
		var rel domain.Release
		if err := tx.Where("repo_id = ? AND tag = ?", repoID, tag).First(&rel).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return nil
			}
			return err
		}
		if err := tx.Where("release_id = ?", rel.ID).Delete(&domain.ReleaseCommit{}).Error; err != nil {
			return err
		}
		return tx.Delete(&rel).Error
	})
}

// ReleaseCommitsOf: los shas de una release, para volver a comentar en una
// reentrega sin preguntarle nada a GitHub.
func (r *GitHubRepository) ReleaseCommitsOf(releaseID string) ([]string, error) {
	var shas []string
	err := r.db.Model(&domain.ReleaseCommit{}).Where("release_id = ?", releaseID).Pluck("sha", &shas).Error
	return shas, err
}

// TaskCommits: las tareas que tienen enlazados algunos de esos commits del
// repo, con cuántos de ellos cada una.
type TaskCommits struct {
	Item  domain.Item
	Count int
}

// TasksOfCommits: ver TaskCommits. Las archivadas y borradas no: no hay a
// quién avisar.
func (r *GitHubRepository) TasksOfCommits(repoID int64, shas []string) ([]TaskCommits, error) {
	if len(shas) == 0 {
		return nil, nil
	}
	type row struct {
		ItemID string
		N      int
	}
	var rows []row
	if err := r.db.Model(&domain.TaskGitLink{}).Select("item_id, count(*) AS n").
		Where("repo_id = ? AND kind = ? AND key IN ?", repoID, domain.GitLinkCommit, shas).
		Group("item_id").Order("item_id").Scan(&rows).Error; err != nil {
		return nil, err
	}
	var out []TaskCommits
	for _, x := range rows {
		var it domain.Item
		if err := r.db.Where("id = ? AND archived_at IS NULL AND deleted_at IS NULL", x.ItemID).First(&it).Error; err != nil {
			continue
		}
		out = append(out, TaskCommits{Item: it, Count: x.N})
	}
	return out, nil
}

// hasReleases: una base sin las tablas de releases (una de pruebas que no las
// migra) contesta «nada salió» en vez de caerse. `GitLinksOf` lo necesita: el
// detalle de una tarea no puede caerse por esto, y
// `handler.TestTheGitHubLinksOfATaskAreFenced` migra sólo tareas y enlaces.
func hasReleases(db *gorm.DB) bool {
	return db.Migrator().HasTable(&domain.Release{}) && db.Migrator().HasTable(&domain.ReleaseCommit{})
}

// ReleasesOfCommits: en qué release quedó publicado cada uno de esos commits
// del repo (domain.CommitShipped: en el de cac, cuando salió cada artefacto
// que toca). Los que no salieron no están en el mapa.
func ReleasesOfCommits(db *gorm.DB, repoID int64, shas []string) (map[string]domain.ReleaseRef, error) {
	out := map[string]domain.ReleaseRef{}
	if len(shas) == 0 || !hasReleases(db) {
		return out, nil
	}
	type row struct {
		SHA         string `gorm:"column:sha"`
		Tag         string
		PublishedAt time.Time
		HTMLURL     string `gorm:"column:html_url"`
	}
	var rows []row
	if err := db.Table("release_commits rc").
		Select("rc.sha, r.tag, r.published_at, r.html_url").
		Joins("JOIN releases r ON r.id = rc.release_id").
		Where("rc.repo_id = ? AND rc.sha IN ?", repoID, shas).Scan(&rows).Error; err != nil {
		return out, err
	}
	firsts := map[commitKey]domain.ReleaseRef{}
	for _, x := range rows {
		firsts[commitKey{repoID, x.SHA}] = domain.ReleaseRef{Tag: x.Tag, At: x.PublishedAt, URL: x.HTMLURL}
	}
	shipped, err := shippedCommits(db, firsts)
	for k, r := range shipped {
		out[k.sha] = r
	}
	return out, err
}

// commitKey: un commit de un repo.
type commitKey struct {
	repoID int64
	sha    string
}

// shippedCommits: de esos commits, con la primera release que trajo cada uno,
// en qué release quedó publicado (domain.CommitShipped). Los que no, fuera
// del mapa.
func shippedCommits(db *gorm.DB, firsts map[commitKey]domain.ReleaseRef) (map[commitKey]domain.ReleaseRef, error) {
	out := map[commitKey]domain.ReleaseRef{}
	byRepo := map[int64][]string{}
	for k := range firsts {
		byRepo[k.repoID] = append(byRepo[k.repoID], k.sha)
	}
	for repoID, shas := range byRepo {
		var dirs map[string][]string
		var releases []domain.ReleaseRef
		if domain.TracksArtifacts(repoID) {
			var err error
			if dirs, err = commitDirsOf(db, repoID, shas); err != nil {
				return out, err
			}
			if releases, err = releasesOfRepo(db, repoID); err != nil {
				return out, err
			}
		}
		for _, sha := range shas {
			k := commitKey{repoID, sha}
			first := firsts[k]
			if r := domain.CommitShipped(repoID, &first, dirs[sha], releases); r != nil {
				out[k] = *r
			}
		}
	}
	return out, nil
}

// commitDirsOf: las carpetas de los commits que se saben. Sin la tabla (una
// base de pruebas que no la migra), ninguno.
func commitDirsOf(db *gorm.DB, repoID int64, shas []string) (map[string][]string, error) {
	out := map[string][]string{}
	if len(shas) == 0 || !db.Migrator().HasTable(&domain.CommitDirs{}) {
		return out, nil
	}
	var rows []domain.CommitDirs
	if err := db.Where("repo_id = ? AND sha IN ?", repoID, shas).Find(&rows).Error; err != nil {
		return out, err
	}
	for _, r := range rows {
		out[r.SHA] = domain.SplitDirs(r.Dirs)
	}
	return out, nil
}

// releasesOfRepo: todas las releases del repo. Son pocas (una por versión).
func releasesOfRepo(db *gorm.DB, repoID int64) ([]domain.ReleaseRef, error) {
	var rows []domain.Release
	if err := db.Select("tag, published_at, html_url").Where("repo_id = ?", repoID).Find(&rows).Error; err != nil {
		return nil, err
	}
	out := make([]domain.ReleaseRef, len(rows))
	for i, r := range rows {
		out[i] = domain.ReleaseRef{Tag: r.Tag, At: r.PublishedAt, URL: r.HTMLURL}
	}
	return out, nil
}

// SaveCommitDirs apunta las carpetas de esos commits. Lo que ya estaba no se
// toca: un commit no cambia, y la primera fuente que lo supo vale.
func (r *GitHubRepository) SaveCommitDirs(rows []domain.CommitDirs) error {
	if len(rows) == 0 {
		return nil
	}
	return r.db.Clauses(clause.OnConflict{DoNothing: true}).CreateInBatches(rows, 200).Error
}

// TrackedRepos: los repos de alguna org cuyos commits necesitan sus carpetas
// (domain.TrackedRepoIDs).
func (r *GitHubRepository) TrackedRepos() ([]domain.GitHubRepo, error) {
	var repos []domain.GitHubRepo
	err := r.db.Where("repo_id IN ? AND org_id <> ''", domain.TrackedRepoIDs()).Find(&repos).Error
	return repos, err
}

// CommitsMissingDirs: hasta `limit` commits del repo que hacen falta para
// saber si algo salió y cuyas carpetas no se saben. Sólo los que están en
// una release (los demás no han salido, y si salen entrarán aquí) y que
// alguien enlaza: una tarea o un incidente (éste con su sha corto, y con el
// repo o sin él).
func (r *GitHubRepository) CommitsMissingDirs(repoID int64, fullName string, limit int) ([]string, error) {
	if !hasReleases(r.db) || !r.db.Migrator().HasTable(&domain.CommitDirs{}) {
		return nil, nil
	}
	linked := []string{`EXISTS (SELECT 1 FROM task_git_links l
		WHERE l.repo_id = rc.repo_id AND l.kind = 'commit' AND l.key = rc.sha)`}
	if r.db.Migrator().HasTable(&domain.IncidentCommit{}) {
		linked = append(linked, `EXISTS (SELECT 1 FROM incident_commits ic
		WHERE (ic.repo = '' OR ic.repo = @name)
		  AND (ic.sha = rc.sha OR (length(ic.sha) < 40 AND rc.sha LIKE ic.sha || '%')))`)
	}
	var shas []string
	err := r.db.Raw(`
		SELECT rc.sha FROM release_commits rc
		WHERE rc.repo_id = @repo
		  AND NOT EXISTS (SELECT 1 FROM commit_dirs cd WHERE cd.repo_id = rc.repo_id AND cd.sha = rc.sha)
		  AND (`+strings.Join(linked, " OR ")+`)
		ORDER BY rc.created_at DESC, rc.sha
		LIMIT @limit`,
		map[string]any{"repo": repoID, "name": fullName, "limit": limit}).Scan(&shas).Error
	return shas, err
}

// ownedCommit: un commit enlazado de algo (una tarea, un incidente) y, si
// salió, en qué release.
type ownedCommit struct {
	ownerID string
	shipped *domain.ReleaseRef
}

// shippedFrom agrega los commits por dueño con domain.ShippedOf.
func shippedFrom(rows []ownedCommit) map[string]*domain.ReleaseRef {
	total := map[string]int{}
	released := map[string][]domain.ReleaseRef{}
	for _, x := range rows {
		total[x.ownerID]++
		if x.shipped != nil {
			released[x.ownerID] = append(released[x.ownerID], *x.shipped)
		}
	}
	out := map[string]*domain.ReleaseRef{}
	for id, n := range total {
		if s := domain.ShippedOf(n, released[id]); s != nil {
			out[id] = s
		}
	}
	return out
}

// linkedRow: un commit enlazado de algo, con la primera release que lo trajo
// si alguna. Tag vacío: no está en ninguna.
type linkedRow struct {
	OwnerID     string
	RepoID      int64
	SHA         string `gorm:"column:sha"`
	Tag         string
	PublishedAt *time.Time
	HTMLURL     string `gorm:"column:html_url"`
}

// ownedFrom resuelve cada fila con shippedCommits.
func ownedFrom(db *gorm.DB, rows []linkedRow) ([]ownedCommit, error) {
	firsts := map[commitKey]domain.ReleaseRef{}
	for _, x := range rows {
		if x.Tag != "" && x.PublishedAt != nil {
			firsts[commitKey{x.RepoID, x.SHA}] = domain.ReleaseRef{Tag: x.Tag, At: *x.PublishedAt, URL: x.HTMLURL}
		}
	}
	shipped, err := shippedCommits(db, firsts)
	out := make([]ownedCommit, len(rows))
	for i, x := range rows {
		out[i].ownerID = x.OwnerID
		if r, ok := shipped[commitKey{x.RepoID, x.SHA}]; ok {
			out[i].shipped = &r
		}
	}
	return out, err
}

// ShippedOfTasks: de esas tareas, las que tienen todos sus commits enlazados
// ya publicados, y en qué release (ver domain.ShippedOf).
func (r *GitHubRepository) ShippedOfTasks(itemIDs []string) (map[string]*domain.ReleaseRef, error) {
	return shippedOfTasks(r.db, itemIDs)
}

func shippedOfTasks(db *gorm.DB, itemIDs []string) (map[string]*domain.ReleaseRef, error) {
	if len(itemIDs) == 0 || !hasReleases(db) || !db.Migrator().HasTable(&domain.TaskGitLink{}) {
		return map[string]*domain.ReleaseRef{}, nil
	}
	var rows []linkedRow
	err := db.Table("task_git_links l").
		Select("l.item_id AS owner_id, l.repo_id, l.key AS sha, coalesce(r.tag, '') AS tag, r.published_at, coalesce(r.html_url, '') AS html_url").
		Joins("LEFT JOIN release_commits rc ON rc.repo_id = l.repo_id AND rc.sha = l.key").
		Joins("LEFT JOIN releases r ON r.id = rc.release_id").
		Where("l.kind = ? AND l.item_id IN ?", domain.GitLinkCommit, itemIDs).Scan(&rows).Error
	if err != nil {
		return map[string]*domain.ReleaseRef{}, err
	}
	owned, err := ownedFrom(db, rows)
	if err != nil {
		return map[string]*domain.ReleaseRef{}, err
	}
	return shippedFrom(owned), nil
}

// ShippedOfIncidents: lo mismo para incidentes. Su sha puede ser corto y su
// repo puede faltar (`domain.ParseCommitRef`): se cruza por prefijo, y por
// repo sólo cuando lo dice. Un sha corto que casara con dos commits de
// releases distintas cuenta una vez, con la más reciente. Lo que decide si
// salió es el commit entero que casó (sus carpetas se guardan por sha
// completo).
func ShippedOfIncidents(db *gorm.DB, incidentIDs []string) (map[string]*domain.ReleaseRef, error) {
	if len(incidentIDs) == 0 || !hasReleases(db) || !db.Migrator().HasTable(&domain.IncidentCommit{}) {
		return map[string]*domain.ReleaseRef{}, nil
	}
	var rows []linkedRow
	err := db.Raw(`
		SELECT ic.incident_id AS owner_id, coalesce(x.repo_id, 0) AS repo_id, coalesce(x.sha, '') AS sha,
		       coalesce(x.tag, '') AS tag, x.published_at, coalesce(x.html_url, '') AS html_url
		FROM incident_commits ic
		LEFT JOIN LATERAL (
			SELECT rc.repo_id, rc.sha, r.tag, r.published_at, r.html_url
			FROM release_commits rc JOIN releases r ON r.id = rc.release_id
			WHERE (rc.sha = ic.sha OR (length(ic.sha) < 40 AND rc.sha LIKE ic.sha || '%'))
			  AND (ic.repo = '' OR r.repo_full_name = ic.repo)
			ORDER BY r.published_at DESC LIMIT 1
		) x ON true
		WHERE ic.incident_id IN ?`, incidentIDs).Scan(&rows).Error
	if err != nil {
		return map[string]*domain.ReleaseRef{}, err
	}
	owned, err := ownedFrom(db, rows)
	if err != nil {
		return map[string]*domain.ReleaseRef{}, err
	}
	return shippedFrom(owned), nil
}
