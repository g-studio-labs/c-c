package service

import (
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	lg "github.com/guz-studio/cac/backend/internal/core/logger"
)

// ghRelease: la release tal como la cuentan el evento `release` y la API de
// listado (`GET /repos/{repo}/releases`): la misma forma.
type ghRelease struct {
	TagName     string     `json:"tag_name"`
	Name        string     `json:"name"`
	HTMLURL     string     `json:"html_url"`
	Draft       bool       `json:"draft"`
	Prerelease  bool       `json:"prerelease"`
	PublishedAt *time.Time `json:"published_at"`
	Author      struct {
		Login string `json:"login"`
	} `json:"author"`
}

// onRelease: una release publicada guarda sus commits (los de entre la
// anterior y ésta) y deja una línea en cada tarea que tiene alguno. Marca y
// avisa; no mueve ni cierra nada (ver domain.Release).
//
//   - Sólo `published` escribe. `gh release create` manda `created`,
//     `published` y `released` a la vez, y con uno basta.
//   - Una release en borrador o una prerelease no cuenta: no es lo que
//     instala nadie, y como «anterior» dejaría vacía la final que la sigue.
//   - `edited` cambia el nombre o el enlace; `deleted` y `unpublished` la
//     quitan, con sus commits. Las líneas del hilo se quedan: son historia.
//   - Una reentrega no vuelve a preguntarle a GitHub: la release ya está, y
//     sólo se repasan las líneas, que no se escriben dos veces.
func (s *GitHubService) onRelease(p *ghPayload) error {
	rel := p.Release
	if rel.TagName == "" {
		return nil
	}
	repo := s.orgRepo(p.Repository.ID)
	if repo == nil {
		return nil
	}
	switch p.Action {
	case "edited":
		return s.repo.UpdateReleaseMeta(repo.RepoID, rel.TagName, ghTruncate(rel.Name, 300), safeURL(rel.HTMLURL))
	case "deleted", "unpublished":
		return s.repo.DeleteRelease(repo.RepoID, rel.TagName)
	case "published":
	default:
		return nil
	}
	if rel.Draft || rel.Prerelease {
		return nil
	}
	saved, err := s.repo.FindRelease(repo.RepoID, rel.TagName)
	if err != nil {
		return err
	}
	if saved == nil {
		if saved, err = s.saveRelease(repo, rel); err != nil {
			return err
		}
	}
	return s.announceRelease(repo, saved)
}

// saveRelease lee de GitHub qué commits lleva y la guarda. Si GitHub falla a
// medias no se guarda nada y el error vuelve al webhook: GitHub la marca como
// fallida y se puede reentregar.
func (s *GitHubService) saveRelease(repo *domain.GitHubRepo, rel ghRelease) (*domain.Release, error) {
	published := time.Now()
	if rel.PublishedAt != nil {
		published = *rel.PublishedAt
	}
	r := &domain.Release{
		OrgID: repo.OrgID, RepoID: repo.RepoID, RepoFullName: repo.FullName,
		Tag: rel.TagName, Name: ghTruncate(rel.Name, 300), HTMLURL: safeURL(rel.HTMLURL),
		AuthorLogin: ghTruncate(rel.Author.Login, 120), PublishedAt: published,
		ListedByLine: true,
	}
	var commits []domain.ReleaseCommit
	var inheritUpTo *time.Time
	if s.CanRead() {
		prev, err := s.previousTag(repo, rel.TagName, published)
		if err != nil {
			return nil, err
		}
		if prev.tag != "" {
			r.PrevTag = prev.tag
			var total int
			if commits, total, err = s.compareCommits(repo, prev.tag, rel.TagName); err != nil {
				return nil, err
			}
			r.CommitCount, r.Truncated = total, total > len(commits)
			if prev.lineStart {
				inheritUpTo = &prev.at
			}
		}
	}
	if _, err := s.repo.SaveReleaseInheriting(r, commits, inheritUpTo); err != nil {
		return nil, err
	}
	// Si otra entrega la guardó a la vez, manda la suya.
	return s.repo.FindRelease(repo.RepoID, rel.TagName)
}

// prevRelease: contra qué release se compara una nueva.
type prevRelease struct {
	tag string
	at  time.Time
	// lineStart: no es de su línea, porque la nueva es la primera de la suya
	// (ver GitHubRepository.SaveReleaseInheriting).
	lineStart bool
}

// previousTag: la release contra la que comparar, que es la anterior de su
// misma línea (domain.ReleaseLine): una `desktop-v*` contra la `desktop-v*`
// de antes, no contra la `v*` que salió en medio. Así lo que lista es lo que
// lleva de nuevo esa línea (command-and-control-526).
//
// Primero la nuestra (la anterior que vio cac); si no hay, la anterior
// publicada en GitHub. Si la línea no tiene ninguna, es su primera release:
// se compara con la anterior de cualquier línea y hereda lo de antes. Si
// tampoco hay, es la primera del repo y no hay contra qué comparar: vacío, y
// no se inventa nada.
func (s *GitHubService) previousTag(repo *domain.GitHubRepo, tag string, before time.Time) (prevRelease, error) {
	line := domain.ReleaseLine(tag)
	prev, err := s.repo.PreviousReleaseOfLine(repo.RepoID, line, before)
	if err != nil {
		return prevRelease{}, err
	}
	if prev != nil {
		return prevRelease{tag: prev.Tag, at: prev.PublishedAt}, nil
	}
	var list []ghRelease
	if err := s.asInstallation(repo.InstallationID, http.MethodGet,
		fmt.Sprintf("/repos/%s/releases?per_page=100", repo.FullName), nil, &list); err != nil {
		return prevRelease{}, err
	}
	var other *prevRelease
	for _, r := range list {
		if r.TagName == tag || r.Draft || r.Prerelease || r.PublishedAt == nil || !r.PublishedAt.Before(before) {
			continue
		}
		if domain.ReleaseLine(r.TagName) == line {
			return prevRelease{tag: r.TagName, at: *r.PublishedAt}, nil
		}
		if other == nil {
			other = &prevRelease{tag: r.TagName, at: *r.PublishedAt, lineStart: true}
		}
	}
	if any, err := s.repo.PreviousRelease(repo.RepoID, before); err != nil {
		return prevRelease{}, err
	} else if any != nil {
		return prevRelease{tag: any.Tag, at: any.PublishedAt, lineStart: true}, nil
	}
	if other != nil {
		return *other, nil
	}
	return prevRelease{}, nil
}

// ReleasesRelistedPerRound: cuántas releases viejas se ponen por línea en
// cada vuelta (ver RelistReleasesByLine).
const ReleasesRelistedPerRound = 10

// RelistReleasesByLine: una vuelta de poner por línea las releases guardadas
// antes del 25-nov-2026, que se comparaban contra la anterior de cualquier
// línea (domain.Release.ListedByLine). Sólo en los repos con tabla
// (domain.TracksArtifacts): en los demás «salió» es la primera release que lo
// lleva, y eso ya lo daban.
//
// Por cada una, de la más vieja a la más nueva:
//   - si su anterior ya era de su línea, nada que añadir;
//   - si hay una anterior de su línea, pide a GitHub el `compare` contra ésa
//     y añade lo que falte;
//   - si no hay, es la primera de su línea, y hereda lo de antes de su base.
//
// Nada se quita y lo que ya estaba se queda; la release se marca y no se
// vuelve a mirar. Si GitHub falla, la vuelta para ahí y sigue en la
// siguiente; si ya no conoce una de las dos etiquetas, se marca sin añadir.
func (s *GitHubService) RelistReleasesByLine(limit int) (int, error) {
	if !s.CanRead() {
		return 0, nil
	}
	repos, err := s.repo.TrackedRepos()
	if err != nil {
		return 0, err
	}
	done := 0
	for _, repo := range repos {
		rels, err := s.repo.ReleasesToRelist(repo.RepoID, limit)
		if err != nil {
			return done, err
		}
		for i := range rels {
			if err := s.relistRelease(&repo, &rels[i]); err != nil {
				return done, err
			}
			done++
		}
	}
	return done, nil
}

func (s *GitHubService) relistRelease(repo *domain.GitHubRepo, rel *domain.Release) error {
	line := domain.ReleaseLine(rel.Tag)
	if rel.PrevTag != "" && domain.ReleaseLine(rel.PrevTag) == line {
		return s.repo.RelistRelease(rel, nil, nil)
	}
	prev, err := s.repo.PreviousReleaseOfLine(repo.RepoID, line, rel.PublishedAt)
	if err != nil {
		return err
	}
	if prev != nil {
		commits, total, err := s.compareCommits(repo, prev.Tag, rel.Tag)
		var status *ghStatusError
		if errors.As(err, &status) && (status.Code == http.StatusNotFound || status.Code == http.StatusUnprocessableEntity) {
			lg.Warn(fmt.Sprintf("github: relist %s: compare %s...%s: %d", repo.FullName, prev.Tag, rel.Tag, status.Code))
			return s.repo.RelistRelease(rel, nil, nil)
		}
		if err != nil {
			return err
		}
		rel.PrevTag, rel.CommitCount, rel.Truncated = prev.Tag, total, total > len(commits)
		return s.repo.RelistRelease(rel, commits, nil)
	}
	base, err := s.repo.PreviousRelease(repo.RepoID, rel.PublishedAt)
	if err != nil {
		return err
	}
	if base == nil {
		return s.repo.RelistRelease(rel, nil, nil)
	}
	return s.repo.RelistRelease(rel, nil, &base.PublishedAt)
}

// compareCommits: los commits de entre dos tags (una página, hasta
// domain.MaxReleaseCommits) y cuántos son en total.
func (s *GitHubService) compareCommits(repo *domain.GitHubRepo, base, head string) ([]domain.ReleaseCommit, int, error) {
	var cmp struct {
		TotalCommits int `json:"total_commits"`
		Commits      []struct {
			SHA     string `json:"sha"`
			HTMLURL string `json:"html_url"`
			Commit  struct {
				Message string `json:"message"`
			} `json:"commit"`
		} `json:"commits"`
	}
	if err := s.asInstallation(repo.InstallationID, http.MethodGet,
		fmt.Sprintf("/repos/%s/compare/%s...%s?per_page=%d", repo.FullName,
			url.PathEscape(base), url.PathEscape(head), domain.MaxReleaseCommits), nil, &cmp); err != nil {
		return nil, 0, err
	}
	var out []domain.ReleaseCommit
	for i, c := range cmp.Commits {
		if i >= domain.MaxReleaseCommits || !domain.ValidSha(c.SHA) {
			continue
		}
		out = append(out, domain.ReleaseCommit{
			SHA: c.SHA, Title: ghTruncate(firstLine(c.Commit.Message), 200), HTMLURL: safeURL(c.HTMLURL),
		})
	}
	return out, cmp.TotalCommits, nil
}

// announceRelease deja en cada tarea con commits en la release una línea
// interna, una sola vez: «Released in v1.9.1: 3 commits · owner/repo». En
// inglés, como sus hermanas («PR #3 merged», «deploy `abc` → prod»). Sólo en
// un repo atado a un espacio, como el resto de lo que comenta.
func (s *GitHubService) announceRelease(repo *domain.GitHubRepo, rel *domain.Release) error {
	if repo.SpaceID == "" {
		return nil
	}
	shas, err := s.repo.ReleaseCommitsOf(rel.ID)
	if err != nil {
		return err
	}
	tasks, err := s.repo.TasksOfCommits(repo.RepoID, shas)
	if err != nil {
		return err
	}
	for _, t := range tasks {
		item := t.Item
		noun := "commits"
		if t.Count == 1 {
			noun = "commit"
		}
		line := fmt.Sprintf("Released in [%s](%s): %d %s · %s", mdText(rel.Tag), safeURL(rel.HTMLURL), t.Count, noun, repo.FullName)
		if err := s.addLine(&item, fmt.Sprintf("gh:release:%d:%s", repo.RepoID, rel.Tag), line); err != nil {
			return err
		}
		s.publishGit(&item)
	}
	return nil
}
