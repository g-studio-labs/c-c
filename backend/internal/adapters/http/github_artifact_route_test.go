package http

import (
	"crypto/rand"
	"crypto/rsa"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

/*
Qué artefacto lleva cada commit del repo de cac, y con qué etiqueta sale
(command-and-control-526). Sobre el fixture de la R5, con el repo de cac
(domain.CacRepoID) en la misma instalación y atado a sp-1, y un GitHub falso
que sabe los ficheros de algunos commits.
*/

const cacFullName = "g-studio-labs/c-c"

type commitsGitHub struct {
	mu    sync.Mutex
	calls []string
	// files: los ficheros de cada commit; un sha que no está da 404.
	files map[string][]string
	// down: GitHub contesta 500 a todo lo de commits.
	down bool
}

func (g *commitsGitHub) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	g.mu.Lock()
	defer g.mu.Unlock()
	io.ReadAll(r.Body)
	prefix := "/repos/" + cacFullName + "/commits/"
	switch {
	case strings.HasSuffix(r.URL.Path, "/access_tokens"):
		w.WriteHeader(http.StatusCreated)
		fmt.Fprintf(w, `{"token":"ghs_prueba","expires_at":%q}`, time.Now().Add(time.Hour).Format(time.RFC3339))
	case strings.HasPrefix(r.URL.Path, prefix):
		sha := strings.TrimPrefix(r.URL.Path, prefix)
		g.calls = append(g.calls, sha)
		if g.down {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		files, ok := g.files[sha]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		var parts []string
		for _, f := range files {
			parts = append(parts, fmt.Sprintf(`{"filename":%q}`, f))
		}
		fmt.Fprintf(w, `{"sha":%q,"files":[%s]}`, sha, strings.Join(parts, ","))
	default:
		w.WriteHeader(http.StatusNotFound)
	}
}

func (g *commitsGitHub) asked() []string {
	g.mu.Lock()
	defer g.mu.Unlock()
	return append([]string{}, g.calls...)
}

type artifactFixture struct {
	*ghFixture
	gh  *commitsGitHub
	svc *service.GitHubService
}

func artifactSetup(t *testing.T) *artifactFixture {
	t.Helper()
	f, _ := githubSetup(t)
	if err := f.db.AutoMigrate(&domain.Release{}, &domain.ReleaseCommit{}, &domain.CommitDirs{},
		&domain.Incident{}, &domain.IncidentCommit{}); err != nil {
		t.Fatal(err)
	}
	f.hook(t, "installation_repositories", "d-cac", fmt.Sprintf(`{"action":"added","installation":{"id":5},
		"repositories_added":[{"id":%d,"full_name":%q}]}`, domain.CacRepoID, cacFullName))
	if err := repository.NewGitHubRepository(f.db).BindInstallation(5, "org-1"); err != nil {
		t.Fatal(err)
	}
	f.db.Model(&domain.GitHubRepo{}).Where("repo_id = ?", domain.CacRepoID).Update("space_id", "sp-1")

	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	fake := &commitsGitHub{files: map[string][]string{}}
	api := httptest.NewServer(fake)
	t.Cleanup(api.Close)
	svc := service.NewGitHubService(repository.NewGitHubRepository(f.db),
		service.GitHubConfig{AppSlug: "cac-test", WebhookSecret: ghSecret}, events.NewHub()).
		WithApp(service.GitHubAppKey{AppID: appID, Key: key, APIURL: api.URL}).Sync()
	r := chi.NewRouter()
	InitGitHubRoutesWith(r, svc)
	f.r = r
	return &artifactFixture{ghFixture: f, gh: fake, svc: svc}
}

// cacPush: un push al repo de cac con un commit y sus ficheros.
func cacPush(sha, msg string, files ...string) string {
	quoted := make([]string, len(files))
	for i, f := range files {
		quoted[i] = fmt.Sprintf("%q", f)
	}
	return fmt.Sprintf(`{"ref":"refs/heads/main","repository":{"id":%d,"full_name":%q},"commits":[{"id":%q,"message":%q,
		"url":"https://github.com/%s/commit/%s","author":{"name":"Ana","username":"ana"},
		"added":[],"modified":[%s],"removed":[]}]}`,
		domain.CacRepoID, cacFullName, sha, msg, cacFullName, sha, strings.Join(quoted, ","))
}

func (f *artifactFixture) release(t *testing.T, tag string, at time.Time, shas ...string) {
	t.Helper()
	var commits []domain.ReleaseCommit
	for _, s := range shas {
		commits = append(commits, domain.ReleaseCommit{SHA: s})
	}
	rel := &domain.Release{OrgID: "org-1", RepoID: domain.CacRepoID, RepoFullName: cacFullName, Tag: tag, PublishedAt: at}
	if _, err := repository.NewGitHubRepository(f.db).SaveRelease(rel, commits); err != nil {
		t.Fatal(err)
	}
}

func (f *artifactFixture) releasedIn(t *testing.T, itemID string) map[string]string {
	t.Helper()
	git, err := repository.GitLinksOf(f.db, itemID)
	if err != nil || git == nil {
		t.Fatalf("sin enlaces: %v", err)
	}
	out := map[string]string{}
	for _, c := range git.Commits {
		if c.ReleasedIn != nil {
			out[c.Key[:7]] = c.ReleasedIn.Tag
		}
	}
	return out
}

func (f *artifactFixture) dirsOf(sha string) (string, bool) {
	var row domain.CommitDirs
	if err := f.db.Where("repo_id = ? AND sha = ?", domain.CacRepoID, sha).First(&row).Error; err != nil {
		return "", false
	}
	return row.Dirs, true
}

func cacSha(c byte) string { return strings.Repeat(string(c), 40) }

// El caso de la tarea, de punta a punta: lo que llega por push se apunta, y
// un commit de la nativa no sale con la v1.35.0 de Tauri, sino con la
// desktop-v0.1.0. Uno de mcp/ sale con cualquiera de las dos líneas que lo
// llevan, y uno que toca core/ y desktop/ no sale con `v*`.
//
// Roturas que atrapa: no guardar las carpetas del push (nada saldría), la
// regla vieja (todo saldría con v1.35.0), y no aplicar la regla a `releasedIn`
// de cada commit o al `shipped` de la tarea.
func TestACommitOfTheNativeAppShipsWithItsOwnTag(t *testing.T) {
	f := artifactSetup(t)
	desk, mcp, mixed, back := cacSha('d'), cacSha('e'), cacSha('f'), cacSha('b')
	f.hook(t, "push", "p-1", cacPush(desk, "fix(desktop): x cac#12", "desktop/src/main.rs"))
	f.hook(t, "push", "p-2", cacPush(mcp, "fix(mcp): x cac#12", "mcp/src/server.rs"))
	f.hook(t, "push", "p-3", cacPush(mixed, "fix(core): x cac#12", "core/src/a.rs", "desktop/src/b.rs"))
	f.hook(t, "push", "p-4", cacPush(back, "fix(backend): x cac#12", "backend/x.go", "docs/y.md"))

	if dirs, ok := f.dirsOf(mixed); !ok || dirs != "core,desktop" {
		t.Fatalf("el push no apuntó las carpetas: %q %v", dirs, ok)
	}

	day := time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC)
	f.release(t, "v1.35.0", day, desk, mcp, mixed, back)
	got := f.releasedIn(t, "it-1")
	if fmt.Sprint(got) != fmt.Sprint(map[string]string{"eeeeeee": "v1.35.0"}) {
		t.Errorf("con v1.35.0 sólo sale el de mcp/: %v", got)
	}
	shipped, err := repository.NewGitHubRepository(f.db).ShippedOfTasks([]string{"it-1"})
	if err != nil || shipped["it-1"] != nil {
		t.Errorf("la tarea no salió con v1.35.0: %+v %v", shipped["it-1"], err)
	}

	f.release(t, "desktop-v0.1.0", day.Add(time.Hour))
	got = f.releasedIn(t, "it-1")
	want := map[string]string{"ddddddd": "desktop-v0.1.0", "eeeeeee": "v1.35.0", "fffffff": "desktop-v0.1.0"}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("con desktop-v0.1.0: %v, se esperaba %v", got, want)
	}
	if shipped, _ := repository.NewGitHubRepository(f.db).ShippedOfTasks([]string{"it-1"}); shipped["it-1"] != nil {
		t.Errorf("con un commit sólo de backend la tarea no sale por ninguna etiqueta: %+v", shipped["it-1"])
	}
}

// Un incidente cruza por sha corto: lo que decide es el commit entero que
// casó, con sus carpetas.
//
// Rotura que atrapa: buscar las carpetas por el sha corto del incidente (no
// casa nunca) o seguir con la regla vieja para los incidentes.
func TestAnIncidentOfTheNativeAppWaitsForItsTag(t *testing.T) {
	f := artifactSetup(t)
	desk := cacSha('d')
	f.hook(t, "push", "p-1", cacPush(desk, "fix(desktop): x", "desktop/src/main.rs"))
	day := time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC)
	f.release(t, "v1.35.0", day, desk)
	f.db.Create(&domain.Incident{BaseModel: domain.BaseModel{ID: "inc-1"}, OrgID: "org-1", DocID: "doc-1", Title: "x", Status: domain.IncidentFixed})
	f.db.Create(&domain.IncidentCommit{ID: "ic-1", IncidentID: "inc-1", SHA: desk[:7], Repo: ""})

	if got, _ := repository.ShippedOfIncidents(f.db, []string{"inc-1"}); got["inc-1"] != nil {
		t.Errorf("salió con v1.35.0: %+v", got["inc-1"])
	}
	f.release(t, "desktop-v0.1.0", day.Add(time.Hour))
	if got, _ := repository.ShippedOfIncidents(f.db, []string{"inc-1"}); got["inc-1"] == nil || got["inc-1"].Tag != "desktop-v0.1.0" {
		t.Errorf("no salió con desktop-v0.1.0: %+v", got["inc-1"])
	}
}

// El relleno: los commits enlazados y publicados de antes del push sin
// carpetas se preguntan a GitHub, de pocos en pocos, y sólo ésos.
//
// Roturas que atrapa: reescribir unas carpetas que ya estaban, preguntar por
// un commit que nadie enlaza o que no ha salido, pasarse del límite de la
// vuelta, volver a pedir uno que GitHub no conoce, y dar algo por sabido
// cuando GitHub falla (en vez de reintentarlo en la vuelta siguiente).
func TestTheBackfillOnlyFillsWhatIsMissing(t *testing.T) {
	f := artifactSetup(t)
	known, a, b, gone, loose, unreleased := cacSha('1'), cacSha('2'), cacSha('3'), cacSha('4'), cacSha('5'), cacSha('6')
	now := time.Now()
	for _, sha := range []string{known, a, b, gone, unreleased} {
		f.db.Create(&domain.TaskGitLink{OrgID: "org-1", ItemID: "it-1", RepoID: domain.CacRepoID, RepoFullName: cacFullName,
			Kind: domain.GitLinkCommit, Key: sha, OccurredAt: now})
	}
	repo := repository.NewGitHubRepository(f.db)
	if err := repo.SaveCommitDirs([]domain.CommitDirs{{RepoID: domain.CacRepoID, SHA: known, Dirs: "app"}}); err != nil {
		t.Fatal(err)
	}
	f.release(t, "v1.35.0", time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC), known, a, b, gone, loose)
	f.gh.files[known] = []string{"desktop/x.rs"}
	f.gh.files[a] = []string{"desktop/x.rs", "core/y.rs"}
	f.gh.files[b] = []string{"app/src/z.ts"}

	f.gh.down = true
	if n, err := f.svc.BackfillCommitDirs(2); err == nil || n != 0 {
		t.Errorf("con GitHub caído: %d rellenos, err %v", n, err)
	}
	for _, sha := range []string{a, b, gone} {
		if _, ok := f.dirsOf(sha); ok {
			t.Errorf("con GitHub caído se apuntó %s", sha[:7])
		}
	}

	f.gh.down = false
	f.gh.calls = nil
	n1, err := f.svc.BackfillCommitDirs(2)
	if err != nil || n1 > 2 {
		t.Fatalf("primera vuelta: %d, %v", n1, err)
	}
	if len(f.gh.asked()) != 2 {
		t.Errorf("la vuelta pidió %d commits, el límite es 2", len(f.gh.asked()))
	}
	if _, err := f.svc.BackfillCommitDirs(2); err != nil {
		t.Fatal(err)
	}
	if _, err := f.svc.BackfillCommitDirs(2); err != nil {
		t.Fatal(err)
	}

	for sha, want := range map[string]string{known: "app", a: "core,desktop", b: "app"} {
		if got, ok := f.dirsOf(sha); !ok || got != want {
			t.Errorf("%s: %q (%v), se esperaba %q", sha[:7], got, ok, want)
		}
	}
	if _, ok := f.dirsOf(gone); ok {
		t.Error("un commit que GitHub no conoce quedó apuntado")
	}
	counts := map[string]int{}
	for _, sha := range f.gh.asked() {
		counts[sha]++
	}
	for _, sha := range []string{known, loose, unreleased} {
		if counts[sha] != 0 {
			t.Errorf("se preguntó por %s, que no hacía falta", sha[:7])
		}
	}
	if counts[gone] != 1 {
		t.Errorf("el commit que GitHub no conoce se pidió %d veces", counts[gone])
	}

	// Lo que ya se sabe no se pisa, venga de donde venga.
	if err := repo.SaveCommitDirs([]domain.CommitDirs{{RepoID: domain.CacRepoID, SHA: known, Dirs: "desktop"}}); err != nil {
		t.Fatal(err)
	}
	if got, _ := f.dirsOf(known); got != "app" {
		t.Errorf("se reescribieron unas carpetas que ya estaban: %q", got)
	}

	// Con el que no existe recordado, la vuelta sigue sin pasarse del límite.
	c, d, e := cacSha('7'), cacSha('8'), cacSha('9')
	for _, sha := range []string{c, d, e} {
		f.db.Create(&domain.TaskGitLink{OrgID: "org-1", ItemID: "it-1", RepoID: domain.CacRepoID, RepoFullName: cacFullName,
			Kind: domain.GitLinkCommit, Key: sha, OccurredAt: now})
		f.gh.files[sha] = []string{"app/x.ts"}
	}
	f.release(t, "v1.36.0", time.Date(2026, 11, 26, 3, 0, 0, 0, time.UTC), c, d, e)
	f.gh.calls = nil
	if _, err := f.svc.BackfillCommitDirs(1); err != nil {
		t.Fatal(err)
	}
	if n := len(f.gh.asked()); n != 1 {
		t.Errorf("con límite 1 la vuelta pidió %d commits", n)
	}
}
