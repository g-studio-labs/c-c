package http

import (
	"crypto/rand"
	"crypto/rsa"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"slices"
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
	// compare: los commits de cada `base...head`; uno que no está da 404.
	compare  map[string][]string
	compared []string
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
	case r.URL.Path == "/repos/"+cacFullName+"/releases":
		io.WriteString(w, `[]`)
	case strings.HasPrefix(r.URL.Path, "/repos/"+cacFullName+"/compare/"):
		key := strings.TrimPrefix(r.URL.Path, "/repos/"+cacFullName+"/compare/")
		g.compared = append(g.compared, key)
		shas, ok := g.compare[key]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		var parts []string
		for _, s := range shas {
			parts = append(parts, fmt.Sprintf(`{"sha":%q,"html_url":"https://github.com/x/commit/%s","commit":{"message":"m"}}`, s, s))
		}
		fmt.Fprintf(w, `{"total_commits":%d,"commits":[%s]}`, len(shas), strings.Join(parts, ","))
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
			// «nuevo<-viejo»: un fichero movido, con su nombre de antes.
			name, prev, _ := strings.Cut(f, "<-")
			parts = append(parts, fmt.Sprintf(`{"filename":%q,"previous_filename":%q}`, name, prev))
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
	fake := &commitsGitHub{files: map[string][]string{}, compare: map[string][]string{}}
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

// cacPush: un push al repo de cac con un commit y sus ficheros modificados.
func cacPush(sha, msg string, files ...string) string {
	return cacPushFiles(sha, msg, nil, files, nil)
}

// cacPushFiles: lo mismo con los tres grupos de ficheros del payload.
func cacPushFiles(sha, msg string, added, modified, removed []string) string {
	list := func(files []string) string {
		quoted := make([]string, len(files))
		for i, f := range files {
			quoted[i] = fmt.Sprintf("%q", f)
		}
		return strings.Join(quoted, ",")
	}
	return fmt.Sprintf(`{"ref":"refs/heads/main","repository":{"id":%d,"full_name":%q},"commits":[{"id":%q,"message":%q,
		"url":"https://github.com/%s/commit/%s","author":{"name":"Ana","username":"ana"},
		"added":[%s],"modified":[%s],"removed":[%s]}]}`,
		domain.CacRepoID, cacFullName, sha, msg, cacFullName, sha, list(added), list(modified), list(removed))
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
// desktop-v0.1.0 que lo lleva. Uno de mcp/ sale con cualquiera de las líneas
// que lo llevan, uno que toca core/ y desktop/ no sale con `v*`, y uno de
// backend/ sale con `v*`, como el resto de lo que no es de un artefacto.
//
// Roturas que atrapa: no guardar las carpetas del push (nada saldría), la
// regla vieja (todo saldría con v1.35.0), dejar sin salir lo de backend/ (la
// tarea mixta no saldría nunca), no aplicar la regla a `releasedIn` de cada
// commit o al `shipped` de la tarea, y tomar la última release que lleva el
// commit en vez de la primera.
func TestACommitOfTheNativeAppShipsWithItsOwnTag(t *testing.T) {
	f := artifactSetup(t)
	desk, mcp, mixed, back := cacSha('d'), cacSha('e'), cacSha('f'), cacSha('b')
	f.hook(t, "push", "p-1", cacPush(desk, "fix(desktop): x cac#12", "desktop/src/main.rs"))
	f.hook(t, "push", "p-2", cacPush(mcp, "fix(mcp): x cac#12", "mcp/src/server.rs"))
	f.hook(t, "push", "p-3", cacPush(mixed, "fix(core): x cac#12", "core/src/a.rs", "desktop/src/b.rs"))
	f.hook(t, "push", "p-4", cacPush(back, "fix(backend): x cac#12", "backend/x.go", "STATUS.md"))

	if dirs, ok := f.dirsOf(mixed); !ok || dirs != "core,desktop" {
		t.Fatalf("el push no apuntó las carpetas: %q %v", dirs, ok)
	}

	day := time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC)
	f.release(t, "v1.35.0", day, desk, mcp, mixed, back)
	got := f.releasedIn(t, "it-1")
	want := map[string]string{"bbbbbbb": "v1.35.0", "eeeeeee": "v1.35.0"}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("con v1.35.0: %v, se esperaba %v", got, want)
	}
	shipped, err := repository.NewGitHubRepository(f.db).ShippedOfTasks([]string{"it-1"})
	if err != nil || shipped["it-1"] != nil {
		t.Errorf("la tarea no salió con v1.35.0: %+v %v", shipped["it-1"], err)
	}

	f.release(t, "desktop-v0.1.0", day.Add(time.Hour), desk, mcp, mixed, back)
	// Las siguientes de cada línea también los llevan: no cambian nada.
	f.release(t, "v1.36.0", day.Add(2*time.Hour), desk, mcp, mixed, back)
	f.release(t, "desktop-v0.2.0", day.Add(3*time.Hour), desk, mcp, mixed, back)
	got = f.releasedIn(t, "it-1")
	want = map[string]string{"bbbbbbb": "v1.35.0", "ddddddd": "desktop-v0.1.0", "eeeeeee": "v1.35.0", "fffffff": "desktop-v0.1.0"}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("con desktop-v0.1.0: %v, se esperaba %v", got, want)
	}
	shipped, _ = repository.NewGitHubRepository(f.db).ShippedOfTasks([]string{"it-1"})
	if shipped["it-1"] == nil || shipped["it-1"].Tag != "desktop-v0.1.0" {
		t.Errorf("la tarea salió entera con desktop-v0.1.0: %+v", shipped["it-1"])
	}
}

// Lo que toca un commit es lo que añade, lo que cambia y lo que borra: un
// commit que sólo borra ficheros de la nativa es de la nativa.
//
// Roturas que atrapa: no mirar `removed` o `added` del push (ese commit no
// tendría carpetas, o tendría sólo las de lo modificado), y no apuntar el
// fichero suelto de la tabla (`app/src/index.css` saldría sin la nativa).
func TestAPushCountsAddedAndRemovedFiles(t *testing.T) {
	f := artifactSetup(t)
	gone, added := cacSha('a'), cacSha('c')
	f.hook(t, "push", "p-1", cacPushFiles(gone, "chore(desktop): x cac#12", nil, []string{"app/src/a.ts"}, []string{"desktop/src/old.rs"}))
	f.hook(t, "push", "p-2", cacPushFiles(added, "feat(voice): x cac#12", []string{"voice/src/new.rs"}, nil, nil))
	if dirs, _ := f.dirsOf(gone); dirs != "app,desktop" {
		t.Errorf("borrar desktop/: %q", dirs)
	}
	if dirs, _ := f.dirsOf(added); dirs != "voice" {
		t.Errorf("añadir en voice/: %q", dirs)
	}
	css := cacSha('5')
	f.hook(t, "push", "p-3", cacPush(css, "fix(app): x cac#12", "app/src/index.css"))
	if dirs, _ := f.dirsOf(css); dirs != "app,app/src/index.css" {
		t.Errorf("el CSS que se lleva la nativa: %q", dirs)
	}
}

// Si las carpetas no se pueden guardar, el push sigue: el commit se enlaza a
// su tarea igual, y el relleno las preguntará después.
//
// Rotura que atrapa: devolver el error de SaveCommitDirs y cortar el push
// (el commit no se enlazaría, ni los docs ni los despliegues verían nada).
func TestAPushThatCannotSaveItsDirsStillLinksTheCommit(t *testing.T) {
	f := artifactSetup(t)
	if err := f.db.Migrator().DropTable(&domain.CommitDirs{}); err != nil {
		t.Fatal(err)
	}
	sha := cacSha('d')
	f.hook(t, "push", "p-1", cacPush(sha, "fix(desktop): x cac#12", "desktop/src/main.rs"))
	var n int64
	f.db.Model(&domain.TaskGitLink{}).Where("item_id = ? AND kind = ? AND key = ?", "it-1", domain.GitLinkCommit, sha).Count(&n)
	if n != 1 {
		t.Errorf("el commit no se enlazó a su tarea (%d)", n)
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
	f.release(t, "desktop-v0.1.0", day.Add(time.Hour), desk)
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

// El relleno también pregunta por los commits que sólo enlaza un incidente
// (con su sha corto), y un fichero movido toca la carpeta de la que sale.
//
// Roturas que atrapa: rellenar sólo los commits de las tareas (el incidente
// no saldría nunca), no mirar `previous_filename` (mover un fichero de
// desktop/ a app/ saldría con `v*` sin esperar a la nativa), y no apuntar el
// fichero suelto de la tabla.
func TestTheBackfillFollowsRenamesAndIncidents(t *testing.T) {
	f := artifactSetup(t)
	moved, fixed := cacSha('1'), cacSha('2')
	f.db.Create(&domain.TaskGitLink{OrgID: "org-1", ItemID: "it-1", RepoID: domain.CacRepoID, RepoFullName: cacFullName,
		Kind: domain.GitLinkCommit, Key: moved, OccurredAt: time.Now()})
	f.db.Create(&domain.Incident{BaseModel: domain.BaseModel{ID: "inc-1"}, OrgID: "org-1", DocID: "doc-1", Title: "x", Status: domain.IncidentFixed})
	f.db.Create(&domain.IncidentCommit{ID: "ic-1", IncidentID: "inc-1", SHA: fixed[:7], Repo: ""})
	f.release(t, "v1.35.0", time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC), moved, fixed)
	f.gh.files[moved] = []string{"app/src/x.ts<-desktop/src/x.rs"}
	f.gh.files[fixed] = []string{"desktop/src/y.rs", "app/src/index.css"}

	if _, err := f.svc.BackfillCommitDirs(10); err != nil {
		t.Fatal(err)
	}
	if dirs, ok := f.dirsOf(moved); !ok || dirs != "app,desktop" {
		t.Errorf("el fichero movido: %q (%v)", dirs, ok)
	}
	if dirs, ok := f.dirsOf(fixed); !ok || dirs != "app,app/src/index.css,desktop" {
		t.Errorf("el commit del incidente: %q (%v)", dirs, ok)
	}
}

// Las releases guardadas antes de comparar por línea se ponen al día solas:
// la que se comparó contra otra línea pide su `compare` contra la anterior de
// la suya y añade lo que falta, y la primera de una línea hereda lo de antes.
// Una segunda vuelta no hace nada.
//
// Roturas que atrapa: no relistar (un commit de app/ que la v1.36.0 lleva
// pero que su `compare` viejo contra mcp-v1.33.0 no listó no saldría),
// comparar contra la anterior de cualquier línea, no heredar en la primera
// de una línea, volver a preguntar a GitHub por una ya puesta al día, volver
// a comparar una cuya anterior ya era de su línea, y relistar de la más nueva
// a la más vieja: la desktop-v0.1.0 heredaría de la v1.36.0 antes de que ésta
// ganara lo que su `compare` nuevo trae y el viejo no (el de nativa `n`, que
// la lista vieja, cortada, no tenía).
func TestOldReleasesAreRelistedByLine(t *testing.T) {
	f := artifactSetup(t)
	app, mcp, desk, late := cacSha('a'), cacSha('e'), cacSha('d'), cacSha('f')
	f.hook(t, "push", "p-1", cacPush(app, "fix(app): x cac#12", "app/src/x.ts"))
	f.hook(t, "push", "p-2", cacPush(desk, "fix(desktop): x cac#12", "desktop/src/x.rs"))
	f.hook(t, "push", "p-3", cacPush(late, "fix(desktop): y cac#12", "desktop/src/y.rs"))
	day := time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC)
	old := func(tag, prev string, at time.Time, shas ...string) {
		var commits []domain.ReleaseCommit
		for _, s := range shas {
			commits = append(commits, domain.ReleaseCommit{SHA: s})
		}
		rel := &domain.Release{OrgID: "org-1", RepoID: domain.CacRepoID, RepoFullName: cacFullName, Tag: tag, PrevTag: prev, PublishedAt: at}
		if _, err := repository.NewGitHubRepository(f.db).SaveRelease(rel, commits); err != nil {
			t.Fatal(err)
		}
	}
	// Como las guardaba cac antes: cada una contra la anterior de cualquier
	// línea. El commit de app/ quedó en la mcp-v1.33.0 y no en la v1.36.0.
	old("v1.34.0", "v1.33.0", day.Add(-time.Hour))
	old("v1.35.0", "v1.34.0", day)
	old("mcp-v1.33.0", "v1.35.0", day.Add(time.Hour), app, mcp)
	old("v1.36.0", "mcp-v1.33.0", day.Add(2*time.Hour), desk)
	old("desktop-v0.1.0", "v1.36.0", day.Add(3*time.Hour))
	f.gh.compare["v1.35.0...v1.36.0"] = []string{app, mcp, desk, late}
	f.gh.compare["desktop-v0.1.0...v1.36.0"] = []string{}
	f.gh.compare["mcp-v1.33.0...v1.35.0"] = []string{}

	if got := f.releasedIn(t, "it-1"); got["aaaaaaa"] != "" {
		t.Fatalf("antes de relistar el de app/ no salió con ninguna v*: %v", got)
	}
	if _, err := f.svc.RelistReleasesByLine(10); err != nil {
		t.Fatal(err)
	}
	compared := append([]string{}, f.gh.compared...)
	slices.Sort(compared)
	if fmt.Sprint(compared) != "[desktop-v0.1.0...v1.36.0 mcp-v1.33.0...v1.35.0 v1.35.0...v1.36.0]" {
		t.Errorf("se compararon %v, se esperaba v1.35.0...v1.36.0 y lo que a las primeras de su línea les falta de su base", compared)
	}
	got := f.releasedIn(t, "it-1")
	want := map[string]string{"aaaaaaa": "v1.36.0", "ddddddd": "desktop-v0.1.0", "fffffff": "desktop-v0.1.0"}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("relistadas: %v, se esperaba %v", got, want)
	}
	var v136 domain.Release
	f.db.Where("tag = ?", "v1.36.0").First(&v136)
	if v136.PrevTag != "v1.35.0" || !v136.ListedByLine {
		t.Errorf("v1.36.0 quedó con prev %q, por línea %v", v136.PrevTag, v136.ListedByLine)
	}

	f.gh.compared = nil
	if n, err := f.svc.RelistReleasesByLine(10); err != nil || n != 0 || len(f.gh.compared) != 0 {
		t.Errorf("la segunda vuelta hizo algo: %d, %v, %v", n, f.gh.compared, err)
	}
}

// oldRelease: una release como la guardaba cac antes del 25-nov-2026, sin
// relistar, comparada contra `prev`.
func (f *artifactFixture) oldRelease(t *testing.T, tag, prev string, at time.Time, shas ...string) {
	t.Helper()
	var commits []domain.ReleaseCommit
	for _, s := range shas {
		commits = append(commits, domain.ReleaseCommit{SHA: s})
	}
	rel := &domain.Release{OrgID: "org-1", RepoID: domain.CacRepoID, RepoFullName: cacFullName, Tag: tag, PrevTag: prev, PublishedAt: at}
	if _, err := repository.NewGitHubRepository(f.db).SaveRelease(rel, commits); err != nil {
		t.Fatal(err)
	}
}

func (f *artifactFixture) relisted(tag string) bool {
	var rel domain.Release
	f.db.Where("repo_id = ? AND tag = ?", domain.CacRepoID, tag).First(&rel)
	return rel.ListedByLine
}

// Una etiqueta que GitHub ya no sabe comparar (404) no atasca el relistado:
// esa release se marca sin añadir nada y la vuelta sigue con las de después.
// Pasa con el `compare` contra la anterior de su línea y con el que pide la
// primera de una línea para saber qué no heredar.
//
// Roturas que atrapa: devolver el error del 404 en cualquiera de los dos
// sitios (la vuelta fallaría, la v1.35.0 se quedaría la primera para
// siempre y la v1.36.0 no se relistaría nunca), y heredar sin saber qué le
// falta a la nueva.
func TestARelistThatGitHubCannotCompareDoesNotBlockTheRest(t *testing.T) {
	f := artifactSetup(t)
	app, desk := cacSha('a'), cacSha('d')
	f.hook(t, "push", "p-1", cacPush(app, "fix(app): x cac#12", "app/src/x.ts"))
	f.hook(t, "push", "p-2", cacPush(desk, "fix(desktop): x cac#12", "desktop/src/x.rs"))
	day := time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC)
	f.oldRelease(t, "v1.34.0", "v1.33.0", day, desk)
	f.oldRelease(t, "v1.35.0", "mcp-v0.9.0", day.Add(time.Hour))
	f.oldRelease(t, "v1.36.0", "mcp-v0.9.1", day.Add(2*time.Hour))
	f.oldRelease(t, "desktop-v0.1.0", "v1.36.0", day.Add(3*time.Hour))
	// Ni v1.34.0...v1.35.0 ni desktop-v0.1.0...v1.36.0: 404.
	f.gh.compare["v1.35.0...v1.36.0"] = []string{app}

	if _, err := f.svc.RelistReleasesByLine(10); err != nil {
		t.Fatalf("un 404 del compare cortó la vuelta: %v", err)
	}
	for _, tag := range []string{"v1.34.0", "v1.35.0", "v1.36.0", "desktop-v0.1.0"} {
		if !f.relisted(tag) {
			t.Errorf("%s no quedó relistada", tag)
		}
	}
	got := f.releasedIn(t, "it-1")
	if want := map[string]string{"aaaaaaa": "v1.36.0"}; fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("tras el 404: %v, se esperaba %v (la desktop-v0.1.0 sin heredar)", got, want)
	}
}

func cacReleaseEvent(tag string, published time.Time) string {
	return fmt.Sprintf(`{"action":"published","repository":{"id":%d,"full_name":%q},
		"release":{"tag_name":%q,"name":%q,"html_url":"https://github.com/%s/releases/tag/%s",
		"published_at":%q,"author":{"login":"jose"}}}`,
		domain.CacRepoID, cacFullName, tag, tag, cacFullName, tag, published.UTC().Format(time.RFC3339))
}

// La carrera de la primera release de una línea: desktop-release se lanza en
// A; mientras compila entra C (de desktop/) y sale la v1.36.0 con C; luego se
// publica la desktop-v0.1.0, con target A. Su base es la v1.36.0, y heredar
// por fecha le metería C sin llevarlo. Lo que la base lleva y ella no
// (`compare desktop-v0.1.0...v1.36.0`) no se hereda: C no ha salido, y A sí.
//
// Roturas que atrapa: heredar todo lo publicado hasta la base sin quitar lo
// que la nueva no lleva, y no heredar nada (A, de la nativa, no saldría).
func TestTheFirstReleaseOfALineOnlyInheritsWhatItCarries(t *testing.T) {
	f := artifactSetup(t)
	a, c := cacSha('a'), cacSha('c')
	f.hook(t, "push", "p-1", cacPush(a, "fix(desktop): a cac#12", "desktop/src/a.rs"))
	f.hook(t, "push", "p-2", cacPush(c, "fix(desktop): c cac#12", "desktop/src/c.rs"))
	day := time.Date(2026, 11, 25, 3, 0, 0, 0, time.UTC)
	f.release(t, "v1.35.0", day, a)
	f.gh.compare["v1.35.0...v1.36.0"] = []string{c}
	f.gh.compare["v1.36.0...desktop-v0.1.0"] = []string{}
	f.gh.compare["desktop-v0.1.0...v1.36.0"] = []string{c}

	f.hook(t, "release", "r-1", cacReleaseEvent("v1.36.0", day.Add(time.Hour)))
	f.hook(t, "release", "r-2", cacReleaseEvent("desktop-v0.1.0", day.Add(2*time.Hour)))
	var desk domain.Release
	if err := f.db.Where("tag = ?", "desktop-v0.1.0").First(&desk).Error; err != nil {
		t.Fatalf("la desktop-v0.1.0 no se guardó: %v", err)
	}
	got := f.releasedIn(t, "it-1")
	if want := map[string]string{"aaaaaaa": "desktop-v0.1.0"}; fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("publicados: %v, se esperaba %v (C no va en la desktop-v0.1.0)", got, want)
	}
}
