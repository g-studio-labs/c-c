package http

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/go-chi/chi/v5"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/adapters/handler"
	"github.com/guz-studio/cac/backend/internal/adapters/middleware"
	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

type stubShares struct{}

func (stubShares) Create(w http.ResponseWriter, r *http.Request)           { ok(w, r) }
func (stubShares) List(w http.ResponseWriter, r *http.Request)             { ok(w, r) }
func (stubShares) ListForOrg(w http.ResponseWriter, r *http.Request)       { ok(w, r) }
func (stubShares) Revoke(w http.ResponseWriter, r *http.Request)           { ok(w, r) }
func (stubShares) PublicDoc(w http.ResponseWriter, r *http.Request)        { ok(w, r) }
func (stubShares) PublicAttachment(w http.ResponseWriter, r *http.Request) { ok(w, r) }

// Sin JWT sólo entran las dos rutas públicas; revocar y la lista de la
// organización piden sesión.
func TestOnlyTheTwoPublicDocShareRoutesSkipAuth(t *testing.T) {
	r := chi.NewRouter()
	mountDocShareRoutes(r, stubShares{})
	public := map[string]bool{
		"GET /api/v1/shared-docs":                            true,
		"GET /api/v1/shared-docs/attachments/{attachmentId}": true,
	}
	seen, open := 0, 0
	err := chi.Walk(r, func(method, route string, _ http.Handler, _ ...func(http.Handler) http.Handler) error {
		seen++
		path := strings.NewReplacer("{id}", "x", "{attachmentId}", "a", "{orgId}", "o").Replace(route)
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, httptest.NewRequest(method, path, nil))
		key := method + " " + route
		if public[key] {
			open++
			if rec.Code == http.StatusUnauthorized {
				t.Errorf("%s asks for a JWT and is the door for people without an account", key)
			}
			return nil
		}
		if rec.Code != http.StatusUnauthorized {
			t.Errorf("%s without a JWT → %d, want 401", key, rec.Code)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if seen != 4 || open != len(public) {
		t.Fatalf("the walk saw %d routes and %d public ones", seen, open)
	}
}

func docShareDB(t *testing.T) *gorm.DB {
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
	const name = "cac_test_doc_share_route"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	adminSQL, _ := admin.DB()
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		adminSQL.Close()
	})
	if err := db.AutoMigrate(
		&domain.TaskSpace{}, &domain.TaskFolder{}, &domain.TaskList{}, &domain.User{},
		&domain.Doc{}, &domain.DocTab{}, &domain.DocVersion{}, &domain.DocAttachment{},
		&domain.DocPage{}, &domain.DocPageVersion{}, &domain.DocSection{}, &domain.Item{},
		&domain.DocShare{}, &domain.GraphLink{},
	); err != nil {
		t.Fatal(err)
	}
	if err := repository.EnsureSearchIndexes(db); err != nil {
		t.Fatal(err)
	}
	return db
}

// shareRouter: el grupo de docs con su JWT y las rutas de compartir, como en
// InitTaskRoutes. Los tokens personales de la prueba son `cac_pat_<org>_<scope>`.
func shareRouter(t *testing.T, db *gorm.DB) *chi.Mux {
	t.Helper()
	middleware.UsePATAuthenticator(func(tok string) (*domain.ClaimsJWT, error) {
		parts := strings.SplitN(strings.TrimPrefix(tok, repository.PATPrefix), "_", 2)
		c := &domain.ClaimsJWT{UserID: "u-" + parts[0],
			Orgs: []domain.OrgMembershipClaim{{OrgID: parts[0], Role: domain.OrgRoleMember}}}
		if len(parts) == 2 && parts[1] != "" {
			c.Scopes = []string{parts[1]}
		}
		return c, nil
	})
	t.Cleanup(func() { middleware.UsePATAuthenticator(nil) })
	docs := repository.NewDocRepository(db)
	h := handler.NewDocShareHandler(
		service.NewDocShareService(repository.NewDocShareRepository(db), docs),
		service.NewDocService(docs), nil)
	r := chi.NewRouter()
	r.Route("/api/v1/docs", func(r chi.Router) {
		r.Use(middleware.AuthMiddleware)
		mountDocShareMemberRoutes(r, h)
	})
	mountDocShareRoutes(r, h)
	return r
}

func call(r http.Handler, method, path, token, body string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(method, path, bytes.NewBufferString(body))
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+repository.PATPrefix+token)
	}
	rec := httptest.NewRecorder()
	r.ServeHTTP(rec, req)
	return rec
}

// Crear y revocar un enlace con un token personal pide `docs:share`; con
// `docs:write` o `docs:manage` no basta. Listar es leer y no lo pide. Y quien
// no es de la organización no crea, no lista ni revoca: 404, como al leer el doc.
func TestSharingAPageNeedsItsOwnScopeAndReadAccess(t *testing.T) {
	db := docShareDB(t)
	r := shareRouter(t, db)
	sp := &domain.TaskSpace{OrgID: "org1", Name: "Proteus"}
	db.Create(sp)
	l := &domain.TaskList{SpaceID: sp.ID, Name: "Apps"}
	db.Create(l)
	p, err := repository.NewDocRepository(db).CreatePage("org1", domain.DocOwnerList, l.ID,
		domain.CreateDocPageRequest{Title: "Root"}, "u-org1")
	if err != nil {
		t.Fatal(err)
	}
	shares := "/api/v1/docs/list/" + l.ID + "/pages/" + p.ID + "/shares"
	ttl := `{"ttlHours":168}`

	for _, tok := range []string{"org1_", "org1_docs:write", "org1_docs:manage"} {
		if rec := call(r, http.MethodPost, shares, tok, ttl); rec.Code != http.StatusForbidden {
			t.Errorf("a token %q creates a share: %d %s", tok, rec.Code, rec.Body)
		}
	}
	if rec := call(r, http.MethodPost, shares, "org2_docs:share", ttl); rec.Code != http.StatusNotFound {
		t.Errorf("an outsider creates a share: %d %s", rec.Code, rec.Body)
	}
	if rec := call(r, http.MethodPost, shares, "org1_docs:share", `{"ttlHours":48}`); rec.Code != http.StatusBadRequest {
		t.Errorf("48 hours: %d %s", rec.Code, rec.Body)
	}
	rec := call(r, http.MethodPost, shares, "org1_docs:share", ttl)
	if rec.Code != http.StatusCreated {
		t.Fatalf("a token with docs:share: %d %s", rec.Code, rec.Body)
	}
	var created struct {
		Data domain.DocShareResponse `json:"data"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if created.Data.Link == "" || created.Data.PageID != p.ID || created.Data.CreatedBy != "u-org1" {
		t.Fatalf("the created share: %+v", created.Data)
	}

	if rec := call(r, http.MethodGet, shares, "org1_", ""); rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), created.Data.ID) {
		t.Errorf("a read-only token does not list: %d %s", rec.Code, rec.Body)
	}
	if rec := call(r, http.MethodGet, shares, "org2_docs:share", ""); rec.Code != http.StatusNotFound {
		t.Errorf("an outsider lists: %d %s", rec.Code, rec.Body)
	}

	revoke := "/api/v1/doc-shares/" + created.Data.ID
	if rec := call(r, http.MethodDelete, revoke, "org1_docs:manage", ""); rec.Code != http.StatusForbidden {
		t.Errorf("docs:manage revokes: %d", rec.Code)
	}
	if rec := call(r, http.MethodDelete, revoke, "org2_docs:share", ""); rec.Code != http.StatusNotFound {
		t.Errorf("an outsider revokes: %d", rec.Code)
	}

	pub := httptest.NewRequest(http.MethodGet, "/api/v1/shared-docs", nil)
	pub.Header.Set(handler.DocShareHeader, created.Data.Link)
	out := httptest.NewRecorder()
	r.ServeHTTP(out, pub)
	if out.Code != http.StatusOK {
		t.Fatalf("the public doc with the header: %d %s", out.Code, out.Body)
	}

	if rec := call(r, http.MethodDelete, revoke, "org1_docs:share", ""); rec.Code != http.StatusNoContent {
		t.Fatalf("revoking with docs:share: %d %s", rec.Code, rec.Body)
	}
	out = httptest.NewRecorder()
	r.ServeHTTP(out, pub)
	if out.Code != http.StatusNotFound {
		t.Errorf("a revoked link: %d", out.Code)
	}
	var row domain.DocShare
	db.First(&row, "id = ?", created.Data.ID)
	if row.RevokedAt == nil || time.Since(*row.RevokedAt) > time.Minute {
		t.Errorf("the row was not revoked: %+v", row.RevokedAt)
	}
}

// La lista de toda la organización: un miembro la lee con un token sin
// permisos (es leer, como la lista por página), y sale el enlace que abrió;
// quien no es de la organización recibe 404, no 403 ni una lista vacía.
func TestANonMemberGets404ForTheOrgList(t *testing.T) {
	db := docShareDB(t)
	r := shareRouter(t, db)
	sp := &domain.TaskSpace{OrgID: "org1", Name: "Proteus"}
	db.Create(sp)
	l := &domain.TaskList{SpaceID: sp.ID, Name: "Apps"}
	db.Create(l)
	p, err := repository.NewDocRepository(db).CreatePage("org1", domain.DocOwnerList, l.ID,
		domain.CreateDocPageRequest{Title: "Guide"}, "u-org1")
	if err != nil {
		t.Fatal(err)
	}
	rec := call(r, http.MethodPost, "/api/v1/docs/list/"+l.ID+"/pages/"+p.ID+"/shares", "org1_docs:share", `{"ttlHours":720}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("creating the share: %d %s", rec.Code, rec.Body)
	}

	list := "/api/v1/organizations/org1/doc-shares"
	if rec := call(r, http.MethodGet, list, "org2_docs:share", ""); rec.Code != http.StatusNotFound {
		t.Errorf("an outsider gets the org list: %d %s", rec.Code, rec.Body)
	}
	rec = call(r, http.MethodGet, list+"?space="+sp.ID, "org1_", "")
	if rec.Code != http.StatusOK {
		t.Fatalf("a member with a read-only token: %d %s", rec.Code, rec.Body)
	}
	var got struct {
		Data []domain.OrgDocShareResponse `json:"data"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if len(got.Data) != 1 || got.Data[0].PageTitle != "Guide" || got.Data[0].SpaceID != sp.ID || got.Data[0].Link == "" {
		t.Fatalf("the org list: %+v", got.Data)
	}
}
