package repository

import (
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Una base de producción trae el índice único por (repo, sha) de antes del
// 25-nov-2026. Tras migrar, el mismo commit cabe en dos releases (una `v*` y
// la `desktop-v*` que también lo lleva), y la misma release no lo apunta dos
// veces.
//
// Roturas que atrapa: no quitar el índice viejo (la segunda release no
// apuntaría el commit, y en silencio por el `ON CONFLICT DO NOTHING`), y
// perder la unicidad por release.
func TestACommitFitsInEveryReleaseThatCarriesIt(t *testing.T) {
	db, cleanup := freshDB(t)
	defer cleanup()
	if err := db.AutoMigrate(&domain.Release{}); err != nil {
		t.Fatal(err)
	}
	if err := db.Exec(`CREATE TABLE release_commits (
		id varchar(36) PRIMARY KEY,
		release_id varchar(36) NOT NULL,
		repo_id bigint NOT NULL,
		sha varchar(40) NOT NULL,
		title varchar(200),
		html_url varchar(400),
		created_at timestamptz)`).Error; err != nil {
		t.Fatal(err)
	}
	if err := db.Exec(`CREATE UNIQUE INDEX idx_release_commits_sha ON release_commits (repo_id, sha)`).Error; err != nil {
		t.Fatal(err)
	}
	if err := db.AutoMigrate(&domain.ReleaseCommit{}); err != nil {
		t.Fatal(err)
	}
	if err := EnsureReleaseIndexes(db); err != nil {
		t.Fatal(err)
	}

	repo := NewGitHubRepository(db)
	sha := "abc1234def5678abc1234def5678abc1234def56"
	day := time.Date(2026, 11, 25, 0, 0, 0, 0, time.UTC)
	for i, tag := range []string{"v1.35.0", "desktop-v0.1.0"} {
		rel := &domain.Release{OrgID: "org-1", RepoID: 1, RepoFullName: "o/r", Tag: tag, PublishedAt: day.Add(time.Duration(i) * time.Hour)}
		if _, err := repo.SaveRelease(rel, []domain.ReleaseCommit{{SHA: sha}, {SHA: sha}}); err != nil {
			t.Fatalf("%s: %v", tag, err)
		}
	}
	var n int64
	db.Model(&domain.ReleaseCommit{}).Where("sha = ?", sha).Count(&n)
	if n != 2 {
		t.Errorf("el commit está en %d releases, se esperaban 2 (una por release)", n)
	}
}
