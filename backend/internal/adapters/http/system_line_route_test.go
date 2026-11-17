package http

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// La línea que deja un cambio de estado lleva a quien lo hizo, y aun así no es
// suya: por la ruta de verdad no puede ni reescribirla ni retirarla.
//
// Hasta el 17-nov-2026 una línea de sistema no tenía autor, y `commentScope`
// sólo preguntaba «¿el autor eres tú?». Con el autor puesto
// (command-and-control-383), esa pregunta sola le daba a cada uno el registro
// de lo que hizo para que lo corrigiera.
//
// Roturas que atrapa: dejar que quien figura en una línea de sistema la edite o
// la borre. Y el control, para que el 403 no sea de otra cosa: su propio
// comentario sí lo edita por la misma ruta.
func TestWhoChangedTheStatusCannotRewriteOrWithdrawTheLine(t *testing.T) {
	db := systemLineDB(t)
	r := chi.NewRouter()
	InitTaskRoutes(db, r, events.NewHub(), nil)

	const ana = "u-ana"
	item := &domain.Item{OrgID: "org-1", ListID: "list-1", Title: "Interna", Status: domain.ReportInProgress, Visibility: domain.VisibilityInternal}
	item.ID = "it-1"
	author := ana
	line := &domain.ItemComment{ItemID: "it-1", Kind: domain.CommentKindSystem, Visibility: domain.VisibilityInternal, Body: "status: pending → in_progress", AuthorUserID: &author}
	line.ID = "c-line"
	own := &domain.ItemComment{ItemID: "it-1", Kind: domain.CommentKindUser, Visibility: domain.VisibilityInternal, Body: "lo miro", AuthorUserID: &author}
	own.ID = "c-own"
	for _, m := range []any{item, line, own} {
		if err := db.Create(m).Error; err != nil {
			t.Fatal(err)
		}
	}

	pair, err := repository.GenerateTokens(ana, "ana", false, []domain.OrgMembershipClaim{{OrgID: "org-1", Role: domain.OrgRoleAdmin}})
	if err != nil {
		t.Fatal(err)
	}
	send := func(method, commentID, body string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(method, "/api/v1/tasks/it-1/comments/"+commentID, strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer "+pair.AccessToken)
		req.Header.Set("Content-Type", "application/json")
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, req)
		return rec
	}

	if rec := send(http.MethodPatch, "c-line", `{"body":"status: pending → closed"}`); rec.Code != http.StatusForbidden || !strings.Contains(rec.Body.String(), "not-the-author") {
		t.Errorf("editar la línea de su propio cambio → %d %s", rec.Code, rec.Body.String())
	}
	if rec := send(http.MethodDelete, "c-line", ""); rec.Code != http.StatusForbidden || !strings.Contains(rec.Body.String(), "not-the-author") {
		t.Errorf("borrar la línea de su propio cambio → %d %s", rec.Code, rec.Body.String())
	}
	var kept domain.ItemComment
	if err := db.First(&kept, "id = ?", "c-line").Error; err != nil || kept.Body != "status: pending → in_progress" {
		t.Fatalf("la línea tenía que seguir como estaba: %q %v", kept.Body, err)
	}

	if rec := send(http.MethodPatch, "c-own", `{"body":"lo miro mañana"}`); rec.Code != http.StatusOK {
		t.Fatalf("su propio comentario sí es suyo → %d %s", rec.Code, rec.Body.String())
	}
}

func systemLineDB(t *testing.T) *gorm.DB {
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
	const name = "cac_test_system_line"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	if err := db.AutoMigrate(&domain.Organization{}, &domain.User{}, &domain.TaskSpace{}, &domain.TaskList{},
		&domain.Item{}, &domain.ItemComment{}, &domain.ItemAttachment{}, &domain.GraphLink{}); err != nil {
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
	return db
}
