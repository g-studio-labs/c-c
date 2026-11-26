package service

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// La integración con Basecamp (command-and-control-553), contra un Basecamp
// de mentira: nada de estas pruebas sale a internet ni usa credenciales de
// verdad.

const (
	bcClientID     = "client-id-de-prueba"
	bcClientSecret = "client-secret-de-prueba"
	bcAccount      = int64(999)
	bcProject      = int64(42)
	bcTodo         = int64(5001)
)

// fakeBasecamp: Launchpad y la API de una cuenta, con lo que cada prueba mira.
type fakeBasecamp struct {
	srv *httptest.Server
	mu  sync.Mutex
	// Lo que recibió.
	hookPayloadURL string
	hookTypes      []string
	hooksDeleted   []string
	refreshes      int
	auths          []string
	// Cómo contesta.
	failRefresh bool
	access      string
	nextHook    int
}

// bcOtherAccount: la cuenta que autoriza el code `other-code`, con su propio
// token: conectar otra cuenta a la misma org.
const bcOtherAccount = int64(1000)

func newFakeBasecamp(t *testing.T) *fakeBasecamp {
	f := &fakeBasecamp{access: "at-1", nextHook: 7}
	mux := http.NewServeMux()
	mux.HandleFunc("POST /authorization/token", func(w http.ResponseWriter, r *http.Request) {
		_ = r.ParseForm()
		if r.URL.RawQuery != "" {
			t.Errorf("the token call carried a query string (secrets end up in logs): %q", r.URL.RawQuery)
		}
		if r.PostForm.Get("client_id") != bcClientID || r.PostForm.Get("client_secret") != bcClientSecret {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		f.mu.Lock()
		defer f.mu.Unlock()
		switch r.PostForm.Get("type") {
		case "web_server":
			switch r.PostForm.Get("code") {
			case "good-code":
				fmt.Fprint(w, `{"access_token":"at-1","refresh_token":"rt-1","expires_in":1209600}`)
			case "other-code":
				fmt.Fprint(w, `{"access_token":"at-other","refresh_token":"rt-other","expires_in":1209600}`)
			default:
				w.WriteHeader(http.StatusBadRequest)
			}
		case "refresh":
			f.refreshes++
			if f.failRefresh || r.PostForm.Get("refresh_token") != "rt-1" {
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			f.access = fmt.Sprintf("at-%d", f.refreshes+1)
			fmt.Fprintf(w, `{"access_token":%q,"expires_in":1209600}`, f.access)
		default:
			w.WriteHeader(http.StatusBadRequest)
		}
	})
	authed := func(h http.HandlerFunc) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			f.mu.Lock()
			want := "Bearer " + f.access
			f.auths = append(f.auths, r.Header.Get("Authorization"))
			f.mu.Unlock()
			if r.Header.Get("Authorization") != want {
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			h(w, r)
		}
	}
	mux.HandleFunc("GET /authorization.json", func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") == "Bearer at-other" {
			fmt.Fprintf(w, `{"identity":{"first_name":"Otra","last_name":"Cuenta"},"accounts":[
				{"product":"bc3","id":%d,"name":"Otra"}]}`, bcOtherAccount)
			return
		}
		authed(func(w http.ResponseWriter, r *http.Request) {
			fmt.Fprintf(w, `{"identity":{"first_name":"Jose","last_name":"G"},"accounts":[
				{"product":"bcx","id":1,"name":"Viejo"},{"product":"bc3","id":%d,"name":"Dwit"}]}`, bcAccount)
		})(w, r)
	})
	mux.HandleFunc(fmt.Sprintf("GET /%d/projects/%d.json", bcAccount, bcProject), authed(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintf(w, `{"id":%d,"name":"Proteus","app_url":"https://3.basecamp.com/%d/projects/%d"}`, bcProject, bcAccount, bcProject)
	}))
	mux.HandleFunc(fmt.Sprintf("POST /%d/buckets/%d/webhooks.json", bcAccount, bcProject), authed(func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			PayloadURL string   `json:"payload_url"`
			Types      []string `json:"types"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		f.mu.Lock()
		f.hookPayloadURL, f.hookTypes = body.PayloadURL, body.Types
		id := f.nextHook
		f.nextHook++
		f.mu.Unlock()
		w.WriteHeader(http.StatusCreated)
		fmt.Fprintf(w, `{"id":%d,"active":true}`, id)
	}))
	mux.HandleFunc(fmt.Sprintf("DELETE /%d/buckets/%d/webhooks/{hook}", bcAccount, bcProject), authed(func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		f.hooksDeleted = append(f.hooksDeleted, r.PathValue("hook"))
		f.mu.Unlock()
		w.WriteHeader(http.StatusNoContent)
	}))
	mux.HandleFunc(fmt.Sprintf("GET /%d/projects/%d/people.json", bcAccount, bcProject), authed(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `[
			{"id":101,"name":"Ana Bc","email_address":"ANA@dwit.mx","attachable_sgid":"sgid-ana"},
			{"id":102,"name":"Bob Bc","email_address":"bob@dwit.mx","attachable_sgid":"sgid-bob"},
			{"id":103,"name":"Carla Bc","email_address":"carla@dwit.mx","attachable_sgid":"sgid-carla"},
			{"id":200,"name":"Cliente","email_address":"cliente@proteus.com","attachable_sgid":"sgid-cliente","client":true}]`)
	}))
	mux.HandleFunc(fmt.Sprintf("GET /%d/projects.json", bcAccount), authed(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("page") == "2" {
			fmt.Fprint(w, `[{"id":43,"name":"Otro","app_url":"javascript:alert(1)"}]`)
			return
		}
		w.Header().Set("Link", fmt.Sprintf(`<%s/%d/projects.json?page=2>; rel="next"`, f.srv.URL, bcAccount))
		fmt.Fprintf(w, `[{"id":%d,"name":"Proteus","app_url":"https://3.basecamp.com/%d/projects/%d"}]`, bcProject, bcAccount, bcProject)
	}))
	f.srv = httptest.NewServer(mux)
	t.Cleanup(f.srv.Close)
	return f
}

// capturedNotices: el Notifier de las pruebas.
type capturedNotices struct {
	mu   sync.Mutex
	list []domain.Aviso
}

func (a *capturedNotices) Notify(x domain.Aviso) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.list = append(a.list, x)
}

func (a *capturedNotices) of(userID string) []domain.Aviso {
	a.mu.Lock()
	defer a.mu.Unlock()
	var out []domain.Aviso
	for _, x := range a.list {
		if x.UserID == userID {
			out = append(out, x)
		}
	}
	return out
}

func (a *capturedNotices) reset() {
	a.mu.Lock()
	a.list = nil
	a.mu.Unlock()
}

func basecampDB(t *testing.T) *gorm.DB {
	t.Helper()
	if repository.GetEnv("DB_HOST", "") == "" {
		t.Skip("no database configured")
	}
	dsn := func(name string) string {
		return fmt.Sprintf("host=%s port=%s user=%s password=%s dbname=%s sslmode=%s",
			repository.GetEnv("DB_HOST", "localhost"), repository.GetEnv("DB_PORT", "5432"),
			repository.GetEnv("DB_USER", "postgres"), repository.GetEnv("DB_PASSWORD", ""),
			name, repository.GetEnv("DB_SSLMODE", "disable"))
	}
	admin, err := gorm.Open(postgres.Open(dsn(repository.GetEnv("DB_NAME", "cac"))), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatalf("DB_HOST is set but the database does not answer (a skipped test is not a green one): %v", err)
	}
	const name = "cac_test_basecamp"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		if a, _ := admin.DB(); a != nil {
			a.Close()
		}
	})
	if err := db.AutoMigrate(&domain.Organization{}, &domain.User{}, &domain.OrgMembership{}, &domain.TaskSpace{},
		&domain.TaskList{}, &domain.Item{}, &domain.ItemAssignee{}, &domain.ItemComment{}, &domain.SpaceSource{},
		&domain.Request{}, &domain.RequestVersion{},
		&domain.BasecampConnection{}, &domain.BasecampProject{}, &domain.BasecampPerson{},
		&domain.BasecampLink{}, &domain.BasecampEvent{}, &domain.BasecampOAuthState{}); err != nil {
		t.Fatal(err)
	}
	if err := repository.EnsureGitHubIndexes(db); err != nil {
		t.Fatal(err)
	}
	return db
}

type bcFixture struct {
	db     *gorm.DB
	fake   *fakeBasecamp
	svc    *BasecampService
	inbox  *capturedNotices
	now    time.Time
	secret string
}

func (f *bcFixture) clock() time.Time { return f.now }

// newBasecampFixture: org-1 con Ana (admin), Bob y Carla; el espacio sp-1 con
// la tarea t-1 (de Ana y Bob); Basecamp conectado y el proyecto 42 enlazado.
func newBasecampFixture(t *testing.T) *bcFixture {
	t.Helper()
	t.Setenv("REPORTS_KEK", base64.StdEncoding.EncodeToString([]byte("0123456789abcdef0123456789abcdef")))
	db := basecampDB(t)
	f := &bcFixture{db: db, fake: newFakeBasecamp(t), inbox: &capturedNotices{}, now: time.Date(2026, 11, 26, 12, 0, 0, 0, time.UTC)}
	org := &domain.Organization{Name: "org-1", Slug: "org-1"}
	org.ID = "org-1"
	mustCreate(t, db, org)
	for _, u := range []struct{ id, email string }{{"u-ana", "ana@dwit.mx"}, {"u-bob", "bob@dwit.mx"}, {"u-carla", "carla@dwit.mx"}, {"u-dan", "dan@dwit.mx"}} {
		user := &domain.User{Username: strings.TrimPrefix(u.id, "u-"), Email: u.email}
		user.ID = u.id
		mustCreate(t, db, user)
	}
	// Dan existe pero no es de la org.
	for _, m := range []struct {
		id   string
		role domain.OrgRole
	}{{"u-ana", domain.OrgRoleAdmin}, {"u-bob", domain.OrgRoleMember}, {"u-carla", domain.OrgRoleMember}} {
		mustCreate(t, db, &domain.OrgMembership{OrgID: "org-1", UserID: m.id, Role: m.role})
	}
	sp := &domain.TaskSpace{OrgID: "org-1", Name: "Proteus", Key: "pr", Rank: "0.5"}
	sp.ID = "sp-1"
	mustCreate(t, db, sp)
	l := &domain.TaskList{SpaceID: "sp-1", Name: "Backend", Rank: "0.5"}
	l.ID = "l-1"
	mustCreate(t, db, l)
	it := &domain.Item{OrgID: "org-1", SpaceID: "sp-1", ListID: "l-1", Title: "El login acepta mayúsculas", Status: domain.ReportPending, Seq: 1}
	it.ID = "t-1"
	mustCreate(t, db, it)
	for _, uid := range []string{"u-ana", "u-bob"} {
		mustCreate(t, db, &domain.ItemAssignee{ItemID: "t-1", UserID: uid})
	}

	f.svc = NewBasecampService(repository.NewBasecampRepository(db), BasecampConfig{
		ClientID: bcClientID, ClientSecret: bcClientSecret,
		RedirectURI:  "https://cac.example/webhooks/basecamp/setup",
		LaunchpadURL: f.fake.srv.URL, APIURL: f.fake.srv.URL,
	}, nil, f.inbox, NewRequestService(repository.NewRequestRepository(db), repository.NewTaskRepository(db),
		repository.NewDocRepository(db), nil, nil)).WithClock(f.clock)

	f.connect(t)
	if _, err := f.svc.LinkProject("org-1", "u-ana", bcProject, "sp-1"); err != nil {
		t.Fatalf("link project: %v", err)
	}
	f.secret = strings.TrimPrefix(f.fake.hookPayloadURL, "https://cac.example/webhooks/basecamp/")
	return f
}

func (f *bcFixture) connect(t *testing.T) {
	t.Helper()
	u, err := f.svc.ConnectURL("org-1", "u-ana")
	if err != nil {
		t.Fatal(err)
	}
	parsed, _ := url.Parse(u)
	if err := f.svc.Setup("good-code", parsed.Query().Get("state")); err != nil {
		t.Fatalf("setup: %v", err)
	}
}

func (f *bcFixture) linkTodo(t *testing.T) {
	t.Helper()
	item, err := f.svc.Task("t-1")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.svc.LinkTask(item, fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d", bcAccount, bcProject, bcTodo)); err != nil {
		t.Fatalf("link task: %v", err)
	}
}

func mentionHTML(sgid, name string) string {
	return `<bc-attachment sgid="` + sgid + `" content-type="application/vnd.basecamp.mention"><figure><figcaption>` + name + `</figcaption></figure></bc-attachment>`
}

// commentEvent: un comentario nuevo en el to-do enlazable, escrito por `who`.
func commentEvent(eventID, commentID int64, who domain.BasecampPersonWire, content string) []byte {
	ev := map[string]any{
		"id": eventID, "kind": "comment_created", "created_at": "2026-11-26T12:00:00Z",
		"recording": map[string]any{
			"id": commentID, "type": "Comment", "title": "Re: Arreglar el login",
			"app_url": fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d#__recording_%d", bcAccount, bcProject, bcTodo, commentID),
			"content": content,
			"parent": map[string]any{"id": bcTodo, "title": "Arreglar el login", "type": "Todo",
				"app_url": fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d", bcAccount, bcProject, bcTodo)},
			"bucket": map[string]any{"id": bcProject, "name": "Proteus", "type": "Project"},
		},
		"creator": who,
	}
	raw, _ := json.Marshal(ev)
	return raw
}

var (
	bcAna      = domain.BasecampPersonWire{ID: 101, Name: "Ana Bc", EmailAddress: "ana@dwit.mx", AttachableSGID: "sgid-ana"}
	bcClient   = domain.BasecampPersonWire{ID: 200, Name: "Cliente", EmailAddress: "cliente@proteus.com", AttachableSGID: "sgid-cliente", Client: true}
	bcNewcomer = domain.BasecampPersonWire{ID: 300, Name: "Nuevo", EmailAddress: "nuevo@otra.com", AttachableSGID: "sgid-nuevo"}
)

func (f *bcFixture) comments(t *testing.T, itemID string) []domain.ItemComment {
	t.Helper()
	var out []domain.ItemComment
	if err := f.db.Where("item_id = ?", itemID).Order("created_at").Find(&out).Error; err != nil {
		t.Fatal(err)
	}
	return out
}

func kinds(notices []domain.Aviso) []string {
	var out []string
	for _, a := range notices {
		out = append(out, a.Kind)
	}
	return out
}

// Conectar: el `state` firmado lleva la org y quién conecta; el callback
// cambia el code por tokens que se guardan cifrados, con la cuenta bc3.
//
// Roturas que atrapa: guardar los tokens en claro; quedarse con la primera
// cuenta aunque no sea bc3; aceptar un state cambiado de org, caducado o ya
// usado; conectar sin la app configurada.
func TestConnectingBasecampStoresTheAccountWithEncryptedTokens(t *testing.T) {
	f := newBasecampFixture(t)
	var conn domain.BasecampConnection
	if err := f.db.First(&conn, "org_id = ?", "org-1").Error; err != nil {
		t.Fatal(err)
	}
	if conn.AccountID != bcAccount || conn.AccountName != "Dwit" || conn.ConnectedByID != "u-ana" {
		t.Fatalf("the connection is %+v", conn)
	}
	if strings.Contains(string(conn.AccessToken), "at-1") || strings.Contains(string(conn.RefreshToken), "rt-1") {
		t.Fatal("the tokens are stored in plain text")
	}
	plain, err := repository.DecryptTelemetry(conn.RefreshToken)
	if err != nil || string(plain) != "rt-1" {
		t.Fatalf("the refresh token does not decrypt to what Launchpad gave: %q %v", plain, err)
	}
	if want := f.now.Add(1209600 * time.Second); !conn.ExpiresAt.Equal(want) {
		t.Fatalf("expires at %v, want %v", conn.ExpiresAt, want)
	}

	// Un state de otra org, con la firma de ésta, no sirve.
	u, _ := f.svc.ConnectURL("org-1", "u-ana")
	parsed, _ := url.Parse(u)
	state := parsed.Query().Get("state")
	parts := strings.Split(state, ".")
	parts[0] = base64.RawURLEncoding.EncodeToString([]byte("org-2"))
	if err := f.svc.Setup("good-code", strings.Join(parts, ".")); !errors.Is(err, domain.ErrBasecampBadState) {
		t.Fatalf("a state moved to another org gave %v", err)
	}
	// Un state vale una vez: quien vio la URL de conectar no vuelve con ella.
	u, _ = f.svc.ConnectURL("org-1", "u-ana")
	parsed, _ = url.Parse(u)
	once := parsed.Query().Get("state")
	if err := f.svc.Setup("good-code", once); err != nil {
		t.Fatalf("a fresh state gave %v", err)
	}
	if err := f.svc.Setup("good-code", once); !errors.Is(err, domain.ErrBasecampBadState) {
		t.Fatalf("a state used twice gave %v", err)
	}
	// El nonce va firmado: no se le pone a un state el de otro enlace vivo.
	u, _ = f.svc.ConnectURL("org-1", "u-ana")
	parsed, _ = url.Parse(u)
	live := strings.Split(parsed.Query().Get("state"), ".")
	swapped := strings.Split(state, ".")
	swapped[2] = live[2]
	if err := f.svc.Setup("good-code", strings.Join(swapped, ".")); !errors.Is(err, domain.ErrBasecampBadState) {
		t.Fatalf("a state with another link's nonce gave %v", err)
	}
	// Caducado.
	f.now = f.now.Add(11 * time.Minute)
	if err := f.svc.Setup("good-code", state); !errors.Is(err, domain.ErrBasecampBadState) {
		t.Fatalf("an expired state gave %v", err)
	}

	// Sin la app de 37signals: «no configurado», y el estado lo dice.
	off := NewBasecampService(repository.NewBasecampRepository(f.db), BasecampConfig{}, nil, nil, nil)
	if _, err := off.ConnectURL("org-1", "u-ana"); !errors.Is(err, domain.ErrBasecampOff) {
		t.Fatalf("connecting without a configured app gave %v", err)
	}
	st, err := off.Status("org-1")
	if err != nil || st.Configured {
		t.Fatalf("status without a configured app: %+v %v", st, err)
	}
}

// Elegir un proyecto da de alta su webhook, con un secreto propio en la
// dirección, del que sólo se guarda el hash; apunta Basecamp como fuente del
// espacio y trae a su gente, casada por correo.
//
// Roturas que atrapa: guardar el secreto en claro; no suscribir los cuatro
// tipos; no apuntar la fuente; casar por correo distinguiendo mayúsculas;
// aceptar un secreto que no es el del proyecto; dejar la fuente o el webhook
// al soltar el proyecto.
func TestLinkingAProjectRegistersItsWebhookWithItsOwnSecret(t *testing.T) {
	f := newBasecampFixture(t)
	if !strings.HasPrefix(f.fake.hookPayloadURL, "https://cac.example/webhooks/basecamp/") || len(f.secret) < 40 {
		t.Fatalf("the webhook address is %q", f.fake.hookPayloadURL)
	}
	if strings.Join(f.fake.hookTypes, ",") != "Comment,Message,Todo,Kanban::Card" {
		t.Fatalf("the webhook listens to %v", f.fake.hookTypes)
	}
	var p domain.BasecampProject
	if err := f.db.First(&p, "org_id = ? AND project_id = ?", "org-1", bcProject).Error; err != nil {
		t.Fatal(err)
	}
	if p.SecretHash != domain.BasecampSecretHash(f.secret) || p.WebhookID != 7 || p.SpaceID != "sp-1" {
		t.Fatalf("the project row is %+v", p)
	}
	var leaked int64
	f.db.Raw(`SELECT COUNT(*) FROM basecamp_projects WHERE row_to_json(basecamp_projects)::text LIKE ?`, "%"+f.secret+"%").Scan(&leaked)
	if leaked != 0 {
		t.Fatal("the webhook secret is stored in plain text")
	}
	var src domain.SpaceSource
	if err := f.db.First(&src, "space_id = ? AND system = ?", "sp-1", domain.SystemBasecamp).Error; err != nil {
		t.Fatalf("Basecamp is not a source of the space: %v", err)
	}
	if src.Role != domain.SourceTracker || src.Ref != fmt.Sprintf("https://3.basecamp.com/%d/projects/%d", bcAccount, bcProject) {
		t.Fatalf("the source is %+v", src)
	}
	people, _ := f.svc.People("org-1")
	byID := map[int64]domain.BasecampPerson{}
	for _, x := range people {
		byID[x.PersonID] = x
	}
	if byID[101].UserID != "u-ana" || byID[102].UserID != "u-bob" || byID[200].UserID != "" {
		t.Fatalf("people after linking: %+v", people)
	}

	if err := f.svc.HandleWebhook(f.secret+"x", commentEvent(1, 9001, bcAna, "hola")); !errors.Is(err, ErrBasecampUnknownHook) {
		t.Fatalf("a wrong secret gave %v", err)
	}
	if err := f.svc.HandleWebhook(f.secret, commentEvent(1, 9001, bcAna, "hola")); err != nil {
		t.Fatalf("the right secret gave %v", err)
	}

	// Soltar el proyecto quita su webhook y su fuente.
	if err := f.svc.UnlinkProject("org-1", bcProject); err != nil {
		t.Fatal(err)
	}
	if strings.Join(f.fake.hooksDeleted, ",") != "7.json" {
		t.Fatalf("hooks removed in Basecamp: %v", f.fake.hooksDeleted)
	}
	var n int64
	f.db.Model(&domain.SpaceSource{}).Where("space_id = ?", "sp-1").Count(&n)
	if n != 0 {
		t.Fatal("the source stayed after unlinking the project")
	}
	if err := f.svc.HandleWebhook(f.secret, commentEvent(2, 9002, bcAna, "hola")); !errors.Is(err, ErrBasecampUnknownHook) {
		t.Fatalf("the secret of an unlinked project still works: %v", err)
	}
}

// Un comentario nuevo en un to-do enlazado queda en la tarea (interno, con
// quién y qué) y avisa a sus responsables, menos a quien lo escribió.
//
// Roturas que atrapa: no comentar la tarea; comentarla en público; avisar a
// quien escribió; no avisar a los responsables; perder el enlace a Basecamp.
func TestACommentOnALinkedTodoLandsInTheTaskAndTellsItsAssignees(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(10, 9010, bcAna, "<div>Ya quedó en <strong>staging</strong></div>")); err != nil {
		t.Fatal(err)
	}
	cs := f.comments(t, "t-1")
	if len(cs) != 1 {
		t.Fatalf("the task has %d comments", len(cs))
	}
	c := cs[0]
	if c.Visibility != domain.VisibilityInternal || c.Kind != domain.CommentKindSystem {
		t.Fatalf("the comment is %s/%s", c.Visibility, c.Kind)
	}
	for _, want := range []string{"**Ana Bc**", "Arreglar el login", "> Ya quedó en staging",
		fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d", bcAccount, bcProject, bcTodo)} {
		if !strings.Contains(c.Body, want) {
			t.Fatalf("the comment does not say %q:\n%s", want, c.Body)
		}
	}
	if got := f.inbox.of("u-ana"); len(got) != 0 {
		t.Fatalf("the author was told about her own comment: %+v", got)
	}
	bob := f.inbox.of("u-bob")
	if len(bob) != 1 || bob[0].Kind != "task:comment" || bob[0].Link != "/tasks?task=t-1" ||
		bob[0].TitleKey != "notify.basecamp.comment" || bob[0].TitleArgs["who"] != "Ana Bc" || bob[0].Body != "Ya quedó en staging" {
		t.Fatalf("Bob got %+v", bob)
	}
	if got := f.inbox.of("u-carla"); len(got) != 0 {
		t.Fatalf("Carla is not on the task and was told: %+v", got)
	}
}

// Una mención en Basecamp avisa a esa persona, aunque no esté en la tarea; y
// quien está en las dos (mencionado y responsable) recibe un solo aviso.
//
// Roturas que atrapa: no leer las menciones; avisar dos veces a quien es
// responsable y además mencionado; avisar a quien se nombra a sí mismo;
// resolver la mención por un sgid que no es el suyo.
func TestAMentionInBasecampTellsThatPerson(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	content := "<div>" + mentionHTML("sgid-carla", "Carla Bc") + " y " + mentionHTML("sgid-bob", "Bob Bc") +
		" y yo " + mentionHTML("sgid-ana", "Ana Bc") + ", miren esto</div>"
	if err := f.svc.HandleWebhook(f.secret, commentEvent(20, 9020, bcAna, content)); err != nil {
		t.Fatal(err)
	}
	carla := f.inbox.of("u-carla")
	if len(carla) != 1 || carla[0].Kind != domain.BasecampMentionKind || carla[0].TitleKey != "notify.basecamp.mention" ||
		carla[0].Link != "/tasks?task=t-1" {
		t.Fatalf("Carla got %+v", carla)
	}
	if bob := f.inbox.of("u-bob"); len(bob) != 1 || bob[0].Kind != domain.BasecampMentionKind {
		t.Fatalf("Bob (assignee and mentioned) got %v", kinds(bob))
	}
	if ana := f.inbox.of("u-ana"); len(ana) != 0 {
		t.Fatalf("Ana mentioned herself and was told: %v", kinds(ana))
	}

	// El sgid de verdad es un id global de Rails: la persona se lee de él
	// aunque su sgid no esté guardado.
	gid := base64.RawURLEncoding.EncodeToString([]byte(`{"_rails":{"data":"gid://bc3/Person/103","pur":"attachable"}}`)) + "--firma"
	f.inbox.reset()
	if err := f.svc.HandleWebhook(f.secret, commentEvent(21, 9021, bcAna, mentionHTML(gid, "Carla"))); err != nil {
		t.Fatal(err)
	}
	if carla := f.inbox.of("u-carla"); len(carla) != 1 || carla[0].Kind != domain.BasecampMentionKind {
		t.Fatalf("a mention by global id did not reach Carla: %v", kinds(carla))
	}
}

// Sin tarea enlazada no se comenta nada: sólo se avisa, con el enlace a
// Basecamp. A quien se nombra; si no se nombra a nadie, a quien enlazó el
// proyecto, salvo que sea quien escribió.
//
// Roturas que atrapa: comentar en alguna tarea; enlazar a cac en vez de a
// Basecamp; dejar el evento sin nadie a quien avisar; avisar a quien enlazó
// el proyecto de lo que escribió él mismo; aceptar por la dirección de un
// proyecto un evento de otro.
func TestWithoutALinkedTaskOnlyANoticeWithTheBasecampLink(t *testing.T) {
	f := newBasecampFixture(t)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(30, 9030, bcClient, "<div>Hola "+mentionHTML("sgid-carla", "Carla")+"</div>")); err != nil {
		t.Fatal(err)
	}
	var n int64
	f.db.Model(&domain.ItemComment{}).Count(&n)
	if n != 0 {
		t.Fatalf("an event without a linked task left %d comments", n)
	}
	carla := f.inbox.of("u-carla")
	if len(carla) != 1 || !strings.HasPrefix(carla[0].Link, fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d", bcAccount, bcProject, bcTodo)) ||
		carla[0].Group != domain.BasecampProjectGroup(bcProject) || carla[0].Label != "Proteus" {
		t.Fatalf("Carla got %+v", carla)
	}
	if ana := f.inbox.of("u-ana"); len(ana) != 0 {
		t.Fatalf("with a mention, whoever linked the project is not told too: %v", kinds(ana))
	}

	f.inbox.reset()
	if err := f.svc.HandleWebhook(f.secret, commentEvent(31, 9031, bcClient, "<div>¿Alguien?</div>")); err != nil {
		t.Fatal(err)
	}
	ana := f.inbox.of("u-ana")
	if len(ana) != 1 || ana[0].Kind != domain.BasecampActivityKind || !strings.HasPrefix(ana[0].Link, "https://3.basecamp.com/") {
		t.Fatalf("whoever linked the project got %+v", ana)
	}

	// El secreto de un proyecto no habla por otro.
	f.inbox.reset()
	other := []byte(strings.ReplaceAll(string(commentEvent(33, 9033, bcClient, "<div>otro</div>")),
		fmt.Sprintf(`"bucket":{"id":%d`, bcProject), `"bucket":{"id":43`))
	if err := f.svc.HandleWebhook(f.secret, other); err != nil {
		t.Fatal(err)
	}
	if got := f.inbox.list; len(got) != 0 {
		t.Fatalf("an event of another project, sent to this project's address, told %+v", got)
	}

	f.inbox.reset()
	if err := f.svc.HandleWebhook(f.secret, commentEvent(32, 9032, bcAna, "<div>nota mía</div>")); err != nil {
		t.Fatal(err)
	}
	if got := f.inbox.list; len(got) != 0 {
		t.Fatalf("Ana wrote it herself and someone was told: %+v", got)
	}
}

// Quien escribe en Basecamp y no casa con nadie de cac queda apuntado para
// que un admin lo asigne; asignado, ya no le avisa lo que escribe él.
//
// Roturas que atrapa: no apuntar a la persona nueva; casarla con alguien de
// fuera de la org; no respetar la asignación a mano en el siguiente evento.
func TestAnUnmatchedPersonIsLeftForAnAdminToAssign(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(40, 9040, bcNewcomer, "<div>Soy nuevo</div>")); err != nil {
		t.Fatal(err)
	}
	st, err := f.svc.Status("org-1")
	if err != nil {
		t.Fatal(err)
	}
	// El nuevo y el cliente.
	if st.Unmatched != 2 {
		t.Fatalf("unmatched people: %d", st.Unmatched)
	}
	if _, err := f.svc.MatchPerson("org-1", 300, "u-dan"); !errors.Is(err, domain.ErrBasecampUserOutside) {
		t.Fatalf("matching someone outside the org gave %v", err)
	}
	p, err := f.svc.MatchPerson("org-1", 300, "u-bob")
	if err != nil || p.UserID != "u-bob" || p.MatchedBy != "manual" {
		t.Fatalf("manual match: %+v %v", p, err)
	}
	// Un evento después no lo despega, y lo que escribe ya no le avisa a él.
	f.inbox.reset()
	if err := f.svc.HandleWebhook(f.secret, commentEvent(41, 9041, bcNewcomer, "<div>Otra vez</div>")); err != nil {
		t.Fatal(err)
	}
	if bob := f.inbox.of("u-bob"); len(bob) != 0 {
		t.Fatalf("Bob (now that person) was told about his own comment: %v", kinds(bob))
	}
	if ana := f.inbox.of("u-ana"); len(ana) != 1 {
		t.Fatalf("Ana (assignee) got %v", kinds(ana))
	}
}

// Basecamp reintenta: el mismo evento no avisa ni comenta dos veces, y
// tampoco otra entrega del mismo comentario.
//
// Roturas que atrapa: no apuntar el evento, o no mirar si ya estaba (sin
// tarea enlazada es lo único que lo frena); comentar dos veces el mismo
// recording.
func TestARepeatedEventTellsOnlyOnce(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	body := commentEvent(50, 9050, bcClient, "<div>Hola "+mentionHTML("sgid-carla", "Carla")+"</div>")
	for range 3 {
		if err := f.svc.HandleWebhook(f.secret, body); err != nil {
			t.Fatal(err)
		}
	}
	// Otro id de evento, el mismo comentario.
	if err := f.svc.HandleWebhook(f.secret, commentEvent(51, 9050, bcClient, "<div>Hola</div>")); err != nil {
		t.Fatal(err)
	}
	if cs := f.comments(t, "t-1"); len(cs) != 1 {
		t.Fatalf("the task has %d comments", len(cs))
	}
	if carla := f.inbox.of("u-carla"); len(carla) != 1 {
		t.Fatalf("Carla was told %d times", len(carla))
	}
	if bob := f.inbox.of("u-bob"); len(bob) != 1 {
		t.Fatalf("Bob was told %d times", len(bob))
	}

	// Sin tarea enlazada no hay comentario que lo frene: sólo el id del evento.
	f.inbox.reset()
	unlinked := commentEvent(52, 9052, bcClient, "<div>"+mentionHTML("sgid-carla", "Carla")+"</div>")
	unlinked = []byte(strings.ReplaceAll(string(unlinked), fmt.Sprintf(`"id":%d`, bcTodo), `"id":7777`))
	for range 2 {
		if err := f.svc.HandleWebhook(f.secret, unlinked); err != nil {
			t.Fatal(err)
		}
	}
	if carla := f.inbox.of("u-carla"); len(carla) != 1 || !strings.HasPrefix(carla[0].Link, "https://") {
		t.Fatalf("a repeated event without a linked task told Carla %d times (%+v)", len(carla), carla)
	}
}

// El token caduca a las dos semanas: se renueva solo con el refresh, y si no
// se puede, se avisa a los admins (una vez por racha), no se queda mudo.
//
// Roturas que atrapa: no renovar; renovar y seguir mandando el token viejo;
// no avisar del fallo; avisar a quien no es admin; avisar cada vez.
func TestTheTokenRenewsItselfAndAFailureTellsTheAdmins(t *testing.T) {
	f := newBasecampFixture(t)
	f.now = f.now.Add(13*24*time.Hour + time.Hour)
	if _, err := f.svc.AvailableProjects("org-1"); err != nil {
		t.Fatalf("listing projects after the token aged: %v", err)
	}
	if f.fake.refreshes != 1 || f.fake.auths[len(f.fake.auths)-1] != "Bearer at-2" {
		t.Fatalf("refreshes %d, last auth %q", f.fake.refreshes, f.fake.auths[len(f.fake.auths)-1])
	}

	// Ahora Launchpad dice que no. El reloj de fondo lo intenta, falla, y
	// avisa a la admin una sola vez aunque lo intente de nuevo.
	f.fake.failRefresh = true
	f.now = f.now.Add(12 * 24 * time.Hour)
	f.svc.RefreshExpiring()
	f.svc.RefreshExpiring()
	ana := f.inbox.of("u-ana")
	if len(ana) != 1 || ana[0].Kind != domain.BasecampAuthKind || ana[0].TitleKey != "notify.basecamp.auth_failed" {
		t.Fatalf("the admin got %+v", ana)
	}
	if bob := f.inbox.of("u-bob"); len(bob) != 0 {
		t.Fatalf("a member who is not an admin was told: %v", kinds(bob))
	}
	st, _ := f.svc.Status("org-1")
	if st.RefreshFailedAt == nil {
		t.Fatal("the status does not say the renewal failed")
	}

	// Vuelve a funcionar: la marca se limpia, y el siguiente fallo avisa otra vez.
	f.fake.failRefresh = false
	f.svc.RefreshExpiring()
	if st, _ := f.svc.Status("org-1"); st.RefreshFailedAt != nil {
		t.Fatal("a renewal that worked did not clear the failure")
	}
}

// Un comentario en el hilo de Basecamp de la tarea contesta el pedido abierto
// a esa persona (los que se contestan con texto), por la vía de Basecamp.
//
// Roturas que atrapa: no contestarlo; contestar un pedido hecho a otra
// persona. (Que un «decide» no se conteste con una frase lo guarda también
// `RequestAnswer.Normalize`.)
func TestABasecampCommentAnswersTheRequestWaitingOnThatPerson(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	mk := func(code, of string, kind domain.RequestKind) {
		q := &domain.Request{Code: code, OrgID: "org-1", TaskID: "t-1", Kind: kind, Summary: "falta " + code,
			Body: []byte(`{"what":"el acceso","from":"el cliente"}`), RequestedOfID: of, RequestedByID: "u-bob",
			Status: domain.RequestOpen}
		mustCreate(t, f.db, q)
	}
	mk("req-ana001", "u-ana", domain.RequestObtain)
	mk("req-ana002", "u-ana", domain.RequestDecide)
	mk("req-bob001", "u-bob", domain.RequestObtain)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(60, 9060, bcAna, "<div>El acceso es por VPN</div>")); err != nil {
		t.Fatal(err)
	}
	get := func(code string) domain.Request {
		var q domain.Request
		if err := f.db.First(&q, "code = ?", code).Error; err != nil {
			t.Fatal(err)
		}
		return q
	}
	q := get("req-ana001")
	var a domain.RequestAnswer
	_ = json.Unmarshal(q.Answer, &a)
	if q.Status != domain.RequestAnswered || q.AnsweredVia != domain.AnsweredViaBasecamp || q.AnsweredByID != "u-ana" ||
		!strings.Contains(a.Note, "El acceso es por VPN") {
		t.Fatalf("Ana's request: %s via %q by %q, note %q", q.Status, q.AnsweredVia, q.AnsweredByID, a.Note)
	}
	if q := get("req-ana002"); q.Status != domain.RequestOpen {
		t.Fatal("a decide request was answered with a sentence")
	}
	if q := get("req-bob001"); q.Status != domain.RequestOpen {
		t.Fatal("Ana's comment answered Bob's request")
	}
}

// Las menciones y las direcciones, sin base.
//
// Roturas que atrapa: tomar por mención un adjunto que no lo es; repetir una
// mención; aceptar una URL que no es de Basecamp; leer mal una URL de
// recording.
func TestBasecampMentionsAndURLs(t *testing.T) {
	html := `<div>` + mentionHTML("a", "A") + mentionHTML("a", "A") +
		`<bc-attachment sgid="img" content-type="image/png"></bc-attachment>` + mentionHTML("b", "B") + `</div>`
	got := domain.ExtractBasecampMentions(html)
	if len(got) != 2 || got[0].SGID != "a" || got[1].SGID != "b" {
		t.Fatalf("mentions: %+v", got)
	}
	for _, bad := range []string{"javascript:alert(1)", "http://3.basecamp.com/1/buckets/2/todos/3",
		"https://basecamp.com.evil.io/1/buckets/2/todos/3", "https://3.basecamp.com/1/projects/2"} {
		if _, _, _, err := domain.ParseBasecampRecordingURL(bad); err == nil {
			t.Fatalf("%q was taken as a recording", bad)
		}
	}
	for _, good := range []string{"https://3.basecamp.com/999/buckets/42/messages/77",
		"https://3.basecamp.com/999/buckets/42/card_tables/cards/77#__recording_80"} {
		acc, proj, rec, err := domain.ParseBasecampRecordingURL(good)
		if err != nil || acc != 999 || proj != 42 || rec != 77 {
			t.Fatalf("%q gave %d %d %d %v", good, acc, proj, rec, err)
		}
	}
}

// addSecondOrg: org-2, de la que sólo es Dan, con una persona de Basecamp
// propia (Eve, sgid-eve) casada con él.
func (f *bcFixture) addSecondOrg(t *testing.T) {
	t.Helper()
	org := &domain.Organization{Name: "org-2", Slug: "org-2"}
	org.ID = "org-2"
	mustCreate(t, f.db, org)
	mustCreate(t, f.db, &domain.OrgMembership{OrgID: "org-2", UserID: "u-dan", Role: domain.OrgRoleMember})
	mustCreate(t, f.db, &domain.BasecampPerson{OrgID: "org-2", PersonID: 400, SGID: "sgid-eve", Name: "Eve",
		Email: "eve@otra.com", UserID: "u-dan", MatchedBy: "manual"})
}

var bcDan = domain.BasecampPersonWire{ID: 401, Name: "Dan Bc", EmailAddress: "dan@dwit.mx", AttachableSGID: "sgid-dan"}

// Una persona de Basecamp con el correo de alguien que sólo es de otra org no
// se casa con él en ésta: le llegarían las menciones de una org que no es la
// suya y su comentario contestaría pedidos.
//
// Rotura que atrapa: casar por correo sin mirar la org del miembro.
func TestAnEmailOfAnotherOrgsMemberDoesNotMatch(t *testing.T) {
	f := newBasecampFixture(t)
	f.addSecondOrg(t)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(80, 9080, bcDan, "<div>hola</div>")); err != nil {
		t.Fatal(err)
	}
	var p domain.BasecampPerson
	if err := f.db.First(&p, "org_id = ? AND person_id = ?", "org-1", bcDan.ID).Error; err != nil {
		t.Fatal(err)
	}
	if p.UserID != "" {
		t.Fatalf("a Basecamp person of org-1 was matched with %q, who is only in org-2", p.UserID)
	}
}

// Una mención en un proyecto de org-1 sólo busca a la persona entre las de
// org-1, aunque otra org tenga guardada a alguien con ese sgid.
//
// Rotura que atrapa: buscar la persona mencionada sin la org.
func TestAMentionLooksOnlyAtThePeopleOfItsOrg(t *testing.T) {
	f := newBasecampFixture(t)
	f.addSecondOrg(t)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(81, 9081, bcAna, "<div>"+mentionHTML("sgid-eve", "Eve")+"</div>")); err != nil {
		t.Fatal(err)
	}
	if dan := f.inbox.of("u-dan"); len(dan) != 0 {
		t.Fatalf("a mention in org-1 reached a user of org-2: %+v", dan)
	}
}

// Enlazar una tarea a un recording de otra cuenta de Basecamp no se puede: no
// llegaría nada, y el enlace taparía el de la cuenta de verdad.
//
// Rotura que atrapa: no comparar la cuenta de la URL con la conectada.
func TestLinkingATaskToAnotherAccountIsRefused(t *testing.T) {
	f := newBasecampFixture(t)
	item, err := f.svc.Task("t-1")
	if err != nil {
		t.Fatal(err)
	}
	_, err = f.svc.LinkTask(item, fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d", bcOtherAccount, bcProject, bcTodo))
	if !errors.Is(err, domain.ErrBasecampBadURL) {
		t.Fatalf("a URL of another account gave %v", err)
	}
}

// El secreto de un proyecto sólo habla por ese proyecto: un cuerpo que no dice
// de cuál es no se cree, y un recording enlazado que es de otro proyecto no se
// escribe desde éste.
//
// Roturas que atrapa: aceptar un cuerpo sin `bucket.id`; escribir en la tarea
// enlazada a un recording de otro proyecto.
func TestOneProjectsSecretSpeaksOnlyForItsProject(t *testing.T) {
	f := newBasecampFixture(t)
	noBucket := []byte(strings.ReplaceAll(string(commentEvent(90, 9090, bcClient, "<div>"+mentionHTML("sgid-carla", "Carla")+"</div>")),
		fmt.Sprintf(`"bucket":{"id":%d,`, bcProject), `"bucket":{`))
	if err := f.svc.HandleWebhook(f.secret, noBucket); err != nil {
		t.Fatal(err)
	}
	if got := f.inbox.list; len(got) != 0 {
		t.Fatalf("a body without its project told %+v", got)
	}

	// t-1 enlazada a un to-do del proyecto 43; el evento llega por el secreto
	// del 42 diciendo que es del 42.
	item, _ := f.svc.Task("t-1")
	if _, err := f.svc.LinkTask(item, fmt.Sprintf("https://3.basecamp.com/%d/buckets/43/todos/6001", bcAccount)); err != nil {
		t.Fatal(err)
	}
	other := []byte(strings.ReplaceAll(string(commentEvent(91, 9091, bcClient, "<div>de otro</div>")),
		fmt.Sprintf(`"id":%d`, bcTodo), `"id":6001`))
	if err := f.svc.HandleWebhook(f.secret, other); err != nil {
		t.Fatal(err)
	}
	if cs := f.comments(t, "t-1"); len(cs) != 0 {
		t.Fatalf("project 42's secret wrote in a task linked to project 43: %+v", cs)
	}
}

// Conectar otra cuenta a la org quita los webhooks de la vieja con el token de
// la vieja, y sus proyectos: su secreto deja de valer.
//
// Roturas que atrapa: dejar los proyectos de la cuenta vieja; quitar sus
// webhooks después de guardar la nueva (con su token y contra su cuenta).
func TestConnectingAnotherAccountDropsTheOldProjects(t *testing.T) {
	f := newBasecampFixture(t)
	u, _ := f.svc.ConnectURL("org-1", "u-ana")
	parsed, _ := url.Parse(u)
	if err := f.svc.Setup("other-code", parsed.Query().Get("state")); err != nil {
		t.Fatalf("connecting another account: %v", err)
	}
	if strings.Join(f.fake.hooksDeleted, ",") != "7.json" {
		t.Fatalf("hooks removed from the old account: %v", f.fake.hooksDeleted)
	}
	st, err := f.svc.Status("org-1")
	if err != nil || st.AccountID != bcOtherAccount || len(st.Projects) != 0 {
		t.Fatalf("after connecting another account: %+v %v", st, err)
	}
	var n int64
	f.db.Model(&domain.SpaceSource{}).Where("space_id = ?", "sp-1").Count(&n)
	if n != 0 {
		t.Fatal("the old account's project is still a source of the space")
	}
	if err := f.svc.HandleWebhook(f.secret, commentEvent(95, 9095, bcAna, "hola")); !errors.Is(err, ErrBasecampUnknownHook) {
		t.Fatalf("the old account's secret still works: %v", err)
	}
}

// Si guardar el proyecto falla, el webhook recién dado de alta se quita (no
// se queda uno huérfano), y el que ya había sigue.
//
// Roturas que atrapa: dejar el webhook nuevo; quitar el viejo antes de saber
// que el nuevo quedó guardado.
func TestARelinkThatFailsToSaveRemovesTheNewWebhook(t *testing.T) {
	f := newBasecampFixture(t)
	if err := f.db.Migrator().DropTable(&domain.SpaceSource{}); err != nil {
		t.Fatal(err)
	}
	if _, err := f.svc.LinkProject("org-1", "u-ana", bcProject, "sp-1"); err == nil {
		t.Fatal("saving the project could not work and linking said yes")
	}
	if strings.Join(f.fake.hooksDeleted, ",") != "8.json" {
		t.Fatalf("hooks removed: %v (want only the new one, 8)", f.fake.hooksDeleted)
	}
	var p domain.BasecampProject
	if err := f.db.First(&p, "org_id = ? AND project_id = ?", "org-1", bcProject).Error; err != nil || p.WebhookID != 7 {
		t.Fatalf("the old project row: %+v %v", p, err)
	}
}

// todoCompleted: el to-do enlazable, completado a la hora `at`.
func todoCompleted(eventID int64, at string) []byte {
	ev := map[string]any{
		"id": eventID, "kind": "todo_completed", "created_at": at,
		"recording": map[string]any{
			"id": bcTodo, "type": "Todo", "title": "Arreglar el login",
			"app_url": fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d", bcAccount, bcProject, bcTodo),
			"bucket":  map[string]any{"id": bcProject, "name": "Proteus", "type": "Project"},
		},
		"creator": bcClient,
	}
	raw, _ := json.Marshal(ev)
	return raw
}

// Un to-do reabierto y completado otra vez es otra cosa que pasó: otra línea
// y otro aviso. La misma completada entregada dos veces, no.
//
// Rotura que atrapa: la clave de la línea sólo por el to-do (la segunda
// completada no se apunta ni avisa).
func TestATodoCompletedAgainIsAnotherLine(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	for _, b := range [][]byte{todoCompleted(70, "2026-11-26T12:00:00Z"), todoCompleted(71, "2026-11-26T12:00:00Z"),
		todoCompleted(72, "2026-11-26T15:00:00Z")} {
		if err := f.svc.HandleWebhook(f.secret, b); err != nil {
			t.Fatal(err)
		}
	}
	if cs := f.comments(t, "t-1"); len(cs) != 2 {
		t.Fatalf("two completions left %d lines", len(cs))
	}
	if bob := f.inbox.of("u-bob"); len(bob) != 2 {
		t.Fatalf("Bob was told %d times of two completions", len(bob))
	}
}

// Un comentario contesta un pedido sólo si esa persona tiene uno abierto que
// se contesta con texto; con dos, no adivina: no contesta ninguno y le avisa.
// En un «correr», lo que escribe es la salida.
//
// Roturas que atrapa: contestar todos los abiertos; contestar uno de dos;
// no avisar a quien se quedó sin contestar; poner el texto de un «correr» en
// la nota y no en la salida.
func TestABasecampCommentAnswersOnlyAnUnambiguousRequest(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	mk := func(code string, kind domain.RequestKind) {
		mustCreate(t, f.db, &domain.Request{Code: code, OrgID: "org-1", TaskID: "t-1", Kind: kind, Summary: "falta " + code,
			Body: []byte(`{"what":"el acceso","from":"el cliente","command":"make","where":"gstd"}`), RequestedOfID: "u-ana",
			RequestedByID: "u-bob", Status: domain.RequestOpen})
	}
	get := func(code string) domain.Request {
		var q domain.Request
		if err := f.db.First(&q, "code = ?", code).Error; err != nil {
			t.Fatal(err)
		}
		return q
	}
	mk("req-two001", domain.RequestObtain)
	mk("req-two002", domain.RequestRun)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(100, 9100, bcAna, "<div>gracias</div>")); err != nil {
		t.Fatal(err)
	}
	for _, c := range []string{"req-two001", "req-two002"} {
		if q := get(c); q.Status != domain.RequestOpen {
			t.Fatalf("with two open requests, %s was answered", c)
		}
	}
	ana := f.inbox.of("u-ana")
	if len(ana) != 1 || ana[0].TitleKey != "notify.basecamp.request_ambiguous" || ana[0].TitleArgs["count"] != "2" {
		t.Fatalf("Ana got %+v", ana)
	}

	// Con uno solo, un «correr»: el texto es la salida.
	f.db.Model(&domain.Request{}).Where("code = ?", "req-two001").Update("status", domain.RequestWithdrawn)
	if err := f.svc.HandleWebhook(f.secret, commentEvent(101, 9101, bcAna, "<div>ok: 3 passed</div>")); err != nil {
		t.Fatal(err)
	}
	q := get("req-two002")
	var a domain.RequestAnswer
	_ = json.Unmarshal(q.Answer, &a)
	if q.Status != domain.RequestAnswered || !strings.Contains(a.Output, "ok: 3 passed") {
		t.Fatalf("the run request: %s, answer %+v", q.Status, a)
	}
}

// Si atender un evento falla, el evento se suelta: el reintento de Basecamp
// entra y se apunta.
//
// Rotura que atrapa: no soltar el evento (el reintento se toma por repetido y
// se pierde).
func TestAFailedEventLetsTheRetryIn(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	if err := f.db.Migrator().DropTable(&domain.ItemComment{}); err != nil {
		t.Fatal(err)
	}
	body := commentEvent(110, 9110, bcClient, "<div>hola</div>")
	if err := f.svc.HandleWebhook(f.secret, body); err == nil {
		t.Fatal("handling the event could not work and it said yes")
	}
	if err := f.db.AutoMigrate(&domain.ItemComment{}); err != nil {
		t.Fatal(err)
	}
	if err := repository.EnsureGitHubIndexes(f.db); err != nil {
		t.Fatal(err)
	}
	if err := f.svc.HandleWebhook(f.secret, body); err != nil {
		t.Fatal(err)
	}
	if cs := f.comments(t, "t-1"); len(cs) != 1 {
		t.Fatalf("the retry left %d lines", len(cs))
	}
}
