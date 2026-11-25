package service

import (
	"errors"
	"fmt"
	"net/http"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Qué carpetas toca cada commit, para saber qué artefacto lleva y con qué
// etiqueta sale (ver domain.CommitDirs y domain.CommitShipped,
// command-and-control-526).

// commitDirsOnPush: el push ya trae los ficheros de cada commit; se apuntan
// sin preguntarle nada a GitHub. Sólo en los repos que lo necesitan
// (domain.TracksArtifacts), y de un repo de alguna org.
//
// Un commit sin ningún fichero en el payload no se apunta: «no toca nada»
// sería saberlo, y lo dejaría fuera para siempre. Sin fila, el relleno lo
// pregunta.
func (s *GitHubService) commitDirsOnPush(p *ghPayload) error {
	if !domain.TracksArtifacts(p.Repository.ID) || len(p.Commits) == 0 {
		return nil
	}
	repo := s.orgRepo(p.Repository.ID)
	if repo == nil {
		return nil
	}
	var rows []domain.CommitDirs
	for _, c := range p.Commits {
		if !domain.ValidSha(c.ID) || len(c.ID) != 40 {
			continue
		}
		paths := append(append(append([]string{}, c.Added...), c.Modified...), c.Removed...)
		if len(paths) == 0 {
			continue
		}
		rows = append(rows, domain.CommitDirs{RepoID: repo.RepoID, SHA: c.ID, Dirs: domain.JoinDirs(domain.CommitKeys(repo.RepoID, paths))})
	}
	return s.repo.SaveCommitDirs(rows)
}

// CommitDirsPerRound: cuántos commits pregunta a GitHub cada vuelta del
// relleno. Con una vuelta por minuto son 600 llamadas a la hora por réplica
// como mucho, de las 5000 que tiene la instalación: corre en cada réplica (lo
// que guarda no pisa, así que dos a la vez sólo gastan más) y deja sitio a
// todo lo demás.
const CommitDirsPerRound = 10

// commitFilesPages: hasta cuántas páginas de cien ficheros se leen de un
// commit: las 3000 que GitHub da como mucho.
const commitFilesPages = 30

// errCommitGone: GitHub no conoce ese commit (404/422). No es un fallo de la
// API: volver a preguntar no lo arregla.
var errCommitGone = errors.New("github: commit not found")

// BackfillCommitDirs: una vuelta del relleno. Por cada repo que lo necesita,
// pregunta a GitHub por hasta `limit` commits enlazados y publicados cuyas
// carpetas no se saben, y las apunta. Sólo rellena lo vacío y nunca reescribe
// (SaveCommitDirs no pisa).
//
// Si la API falla, la vuelta para ahí y lo que falta se pregunta en la
// siguiente; mientras, ese commit no se sabe y no cuenta como publicado. Un
// commit que GitHub no conoce se recuerda en memoria y no se vuelve a pedir
// hasta que el proceso arranque otra vez.
func (s *GitHubService) BackfillCommitDirs(limit int) (int, error) {
	if !s.CanRead() {
		return 0, nil
	}
	repos, err := s.repo.TrackedRepos()
	if err != nil {
		return 0, err
	}
	done := 0
	for _, repo := range repos {
		shas, err := s.repo.CommitsMissingDirs(repo.RepoID, repo.FullName, limit+s.goneCount())
		if err != nil {
			return done, err
		}
		asked := 0
		for _, sha := range shas {
			if asked >= limit {
				break
			}
			if s.isGoneCommit(repo.RepoID, sha) {
				continue
			}
			asked++
			dirs, err := s.commitDirsFromAPI(&repo, sha)
			if errors.Is(err, errCommitGone) {
				s.markGoneCommit(repo.RepoID, sha)
				continue
			}
			if err != nil {
				return done, err
			}
			if err := s.repo.SaveCommitDirs([]domain.CommitDirs{{RepoID: repo.RepoID, SHA: sha, Dirs: domain.JoinDirs(dirs)}}); err != nil {
				return done, err
			}
			done++
		}
	}
	return done, nil
}

// commitDirsFromAPI: las carpetas de un commit según `GET /commits/{sha}`,
// leyendo todas sus páginas de ficheros.
func (s *GitHubService) commitDirsFromAPI(repo *domain.GitHubRepo, sha string) ([]string, error) {
	const perPage = 100
	var paths []string
	for page := 1; page <= commitFilesPages; page++ {
		var c struct {
			Files []struct {
				Filename         string `json:"filename"`
				PreviousFilename string `json:"previous_filename"`
			} `json:"files"`
		}
		err := s.asInstallation(repo.InstallationID, http.MethodGet,
			fmt.Sprintf("/repos/%s/commits/%s?per_page=%d&page=%d", repo.FullName, sha, perPage, page), nil, &c)
		var status *ghStatusError
		if errors.As(err, &status) && (status.Code == http.StatusNotFound || status.Code == http.StatusUnprocessableEntity) {
			return nil, errCommitGone
		}
		if err != nil {
			return nil, err
		}
		for _, f := range c.Files {
			// Un fichero movido toca también la carpeta de la que sale.
			paths = append(paths, f.Filename)
			if f.PreviousFilename != "" {
				paths = append(paths, f.PreviousFilename)
			}
		}
		if len(c.Files) < perPage {
			break
		}
	}
	return domain.CommitKeys(repo.RepoID, paths), nil
}

func goneKey(repoID int64, sha string) string { return fmt.Sprintf("%d:%s", repoID, sha) }

func (s *GitHubService) isGoneCommit(repoID int64, sha string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.gone[goneKey(repoID, sha)]
}

func (s *GitHubService) markGoneCommit(repoID int64, sha string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.gone == nil {
		s.gone = map[string]bool{}
	}
	s.gone[goneKey(repoID, sha)] = true
}

// goneCount: cuántos se saltan, para pedir a la base los suficientes.
func (s *GitHubService) goneCount() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.gone)
}
