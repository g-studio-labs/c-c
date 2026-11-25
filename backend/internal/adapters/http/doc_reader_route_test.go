package http

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/adapters/middleware"
	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Un lector lee la documentación y no la cambia, por ninguna ruta
// (command-and-control-392). Hasta el 25-nov-2026 las rutas de docs sólo
// miraban que quien llama fuese de la organización, y la app de Tauri no le
// esconde los botones a un lector: con ella, o con un token, escribía.
//
// La prueba recorre el enrutado de verdad (`InitTaskRoutes`), así que una ruta
// de escritura nueva bajo `/api/v1/docs`, `/api/v1/incidents` o
// `/api/v1/doc-shares` que no pase por `refuseReader` la pone en rojo.
//
// Roturas que atrapa: quitar la comprobación de una ruta cualquiera; colgar una
// ruta nueva que escriba sin ella; que un token personal de un lector con
// `docs:write` escriba; y, con el control, que se le niegue también a un
// miembro, o que el lector deje de poder leer.
func TestAReaderCannotChangeDocumentation(t *testing.T) {
	db := docReaderDB(t)
	r := chi.NewRouter()
	InitTaskRoutes(db, r, events.NewHub(), nil)

	// Los tokens personales de la prueba: `cac_pat_<rol>_<scope>`, de org-1.
	middleware.UsePATAuthenticator(func(tok string) (*domain.ClaimsJWT, error) {
		parts := strings.SplitN(strings.TrimPrefix(tok, repository.PATPrefix), "_", 2)
		c := &domain.ClaimsJWT{UserID: "u-" + parts[0],
			Orgs: []domain.OrgMembershipClaim{{OrgID: "org-1", Role: domain.OrgRole(parts[0])}}}
		if len(parts) == 2 && parts[1] != "" {
			c.Scopes = []string{parts[1]}
		}
		return c, nil
	})
	t.Cleanup(func() { middleware.UsePATAuthenticator(nil) })

	jwtFor := func(user string, role domain.OrgRole) string {
		pair, err := repository.GenerateTokens(user, user, false,
			[]domain.OrgMembershipClaim{{OrgID: "org-1", Role: role}})
		if err != nil {
			t.Fatal(err)
		}
		return pair.AccessToken
	}
	reader, member := jwtFor("u-reader", domain.OrgRoleViewer), jwtFor("u-member", domain.OrgRoleMember)
	do := func(token, method, path, body string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(method, path, strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer "+token)
		req.Header.Set("Content-Type", "application/json")
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, req)
		return rec
	}
	dataID := func(rec *httptest.ResponseRecorder, what string) string {
		t.Helper()
		var out struct {
			Data struct {
				ID       string `json:"id"`
				Recorded struct {
					ID string `json:"id"`
				} `json:"recorded"`
			} `json:"data"`
		}
		if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
			t.Fatalf("%s: %v %s", what, err, rec.Body.String())
		}
		if out.Data.Recorded.ID != "" {
			return out.Data.Recorded.ID
		}
		if out.Data.ID == "" {
			t.Fatalf("%s sin id: %d %s", what, rec.Code, rec.Body.String())
		}
		return out.Data.ID
	}

	sp := &domain.TaskSpace{OrgID: "org-1", Name: "Proteus"}
	if err := db.Create(sp).Error; err != nil {
		t.Fatal(err)
	}
	l := &domain.TaskList{SpaceID: sp.ID, Name: "Apps"}
	if err := db.Create(l).Error; err != nil {
		t.Fatal(err)
	}
	node := "/api/v1/docs/list/" + l.ID

	// Lo que hay que tener para que cada ruta encuentre lo suyo, escrito por un
	// miembro por las mismas rutas.
	rec := do(member, http.MethodPost, node+"/pages", `{"title":"Root"}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("un miembro crea una página → %d %s", rec.Code, rec.Body.String())
	}
	pageID := dataID(rec, "la página")
	rec = do(member, http.MethodPost, node+"/decisions", `{"title":"Postgres","origin":"doc"}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("un miembro apunta una decisión → %d %s", rec.Code, rec.Body.String())
	}
	decisionID := dataID(rec, "la decisión")
	rec = do(member, http.MethodPost, node+"/incidents", `{"title":"Se cayó"}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("un miembro apunta un incidente → %d %s", rec.Code, rec.Body.String())
	}
	incidentID := dataID(rec, "el incidente")
	rec = do(member, http.MethodPost, node+"/pages/"+pageID+"/shares", `{"ttlHours":24}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("un miembro comparte la página → %d %s", rec.Code, rec.Body.String())
	}
	shareID := dataID(rec, "el enlace")
	var doc domain.Doc
	if err := db.First(&doc, "owner_id = ?", l.ID).Error; err != nil {
		t.Fatal(err)
	}

	fill := func(route string) string {
		id := doc.ID
		switch {
		case strings.HasPrefix(route, "/api/v1/docs/decisions/"):
			id = decisionID
		case strings.HasPrefix(route, "/api/v1/incidents/"):
			id = incidentID
		case strings.HasPrefix(route, "/api/v1/doc-shares/"):
			id = shareID
		}
		return strings.NewReplacer("{kind}", "list", "{ownerId}", l.ID, "{pageId}", pageID,
			"{tab}", "overview", "{versionId}", "v-1", "{linkId}", "link-1",
			"{attachmentId}", "att-1", "{id}", id).Replace(route)
	}
	writes := func(method, route string) bool {
		if method == http.MethodGet || method == http.MethodHead {
			return false
		}
		return strings.HasPrefix(route, "/api/v1/docs/") || strings.HasPrefix(route, "/api/v1/incidents/") ||
			strings.HasPrefix(route, "/api/v1/doc-shares/")
	}
	// Las que un lector sí puede, cada una con su porqué.
	readerMay := map[string]string{
		// Cerrar un enlace filtrado no cambia la documentación, y lo tiene que
		// poder hacer quien se entere primero (ver docShareHandler.Revoke).
		"DELETE /api/v1/doc-shares/{id}": "revocar",
	}

	type route struct{ method, pattern string }
	var routes []route
	if err := chi.Walk(r, func(method, pattern string, _ http.Handler, _ ...func(http.Handler) http.Handler) error {
		if writes(method, pattern) {
			routes = append(routes, route{method, pattern})
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	// Sin esto, un recorrido que no encontrara las rutas pasaría sin mirar.
	if len(routes) < 23 {
		t.Fatalf("el recorrido vio %d rutas que escriben documentación", len(routes))
	}

	refused := func(rec *httptest.ResponseRecorder) bool {
		return rec.Code == http.StatusForbidden && strings.Contains(rec.Body.String(), `"viewer-read-only"`)
	}
	for _, rt := range routes {
		key := rt.method + " " + rt.pattern
		if _, ok := readerMay[key]; ok {
			continue
		}
		if rec := do(reader, rt.method, fill(rt.pattern), "{}"); !refused(rec) {
			t.Errorf("%s: un lector → %d %s, se esperaba 403 viewer-read-only", key, rec.Code, rec.Body.String())
		}
	}

	// Leer sigue abierto para el lector.
	for _, path := range []string{node, node + "/pages", node + "/pages/" + pageID,
		"/api/v1/docs/decisions/" + decisionID, "/api/v1/incidents/" + incidentID} {
		if rec := do(reader, http.MethodGet, path, ""); rec.Code != http.StatusOK {
			t.Errorf("un lector lee %s → %d %s", path, rec.Code, rec.Body.String())
		}
	}

	// Un token personal es su persona: el de un lector no escribe aunque lleve
	// el scope; el de un miembro, sí.
	pat := func(tok string) string { return repository.PATPrefix + tok }
	if rec := do(pat("viewer_docs:write"), http.MethodPost, node+"/pages", `{"title":"Otra"}`); !refused(rec) {
		t.Errorf("el token de un lector con docs:write crea una página → %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(pat("member_docs:write"), http.MethodPost, node+"/pages", `{"title":"Otra"}`); rec.Code != http.StatusCreated {
		t.Errorf("el token de un miembro con docs:write no crea una página → %d %s", rec.Code, rec.Body.String())
	}

	// El control: a un miembro ninguna de esas rutas le contesta lo mismo. Si
	// no, el 403 de arriba podría no tener nada que ver con el rol.
	for _, rt := range routes {
		key := rt.method + " " + rt.pattern
		if rec := do(member, rt.method, fill(rt.pattern), "{}"); refused(rec) {
			t.Errorf("%s: a un miembro → %d %s", key, rec.Code, rec.Body.String())
		}
	}
}

func docReaderDB(t *testing.T) *gorm.DB {
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
	const name = "cac_test_doc_reader_route"
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
	if err := db.AutoMigrate(
		&domain.Organization{}, &domain.OrgMembership{}, &domain.User{},
		&domain.TaskSpace{}, &domain.TaskFolder{}, &domain.TaskList{}, &domain.Item{}, &domain.GraphLink{},
		&domain.Notification{}, &domain.NotificationPrefs{}, &domain.Server{},
		&domain.Doc{}, &domain.DocTab{}, &domain.DocVersion{}, &domain.Decision{}, &domain.DecisionLink{},
		&domain.DocAttachment{}, &domain.DocPage{}, &domain.DocPageVersion{}, &domain.DocSection{},
		&domain.DocShare{},
		&domain.Incident{}, &domain.IncidentServer{}, &domain.IncidentArea{}, &domain.IncidentCommit{}, &domain.IncidentGuard{},
	); err != nil {
		t.Fatal(err)
	}
	if err := repository.EnsureSearchIndexes(db); err != nil {
		t.Fatal(err)
	}
	return db
}
