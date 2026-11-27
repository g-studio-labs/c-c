package http

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// Las rutas de Basecamp, montadas como en producción (command-and-control-553).
//
// El webhook va fuera del JWT y Basecamp no firma: la dirección lleva el
// secreto del proyecto. Uno que no existe es un 404 **sin cuerpo**, el mismo
// que una ruta que no existe, para no decirle a quien prueba que ha dado con
// algo. Sin la app de 37signals configurada, conectar dice «basecamp-off» y el
// resto del servidor contesta igual.
//
// Roturas que atrapa: contestar a un secreto desconocido con algo más que un
// 404 vacío; montar el webhook detrás del JWT; dejar conectar sin la app
// configurada; dejar conectar a quien no es admin.
func TestTheBasecampWebhookIsA404ForAnUnknownSecret(t *testing.T) {
	db, cleanup := githubDB(t)
	defer cleanup()
	if err := db.AutoMigrate(&domain.BasecampConnection{}, &domain.BasecampProject{}, &domain.BasecampPerson{},
		&domain.BasecampLink{}, &domain.BasecampEvent{}); err != nil {
		t.Fatal(err)
	}
	const secret = "s3cr3t-de-prueba-largo-de-sobra-para-valer"
	if err := db.Create(&domain.BasecampProject{OrgID: "org-1", SpaceID: "sp-1", ProjectID: 42, Name: "Proteus",
		SecretHash: domain.BasecampSecretHash(secret)}).Error; err != nil {
		t.Fatal(err)
	}

	r := chi.NewRouter()
	InitBasecampRoutesWith(r, service.NewBasecampService(repository.NewBasecampRepository(db), service.BasecampConfig{}, nil, nil, nil))
	post := func(path string) *httptest.ResponseRecorder {
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, httptest.NewRequest(http.MethodPost, path,
			strings.NewReader(`{"id":1,"kind":"todo_created","recording":{"bucket":{"id":42}}}`)))
		return rec
	}
	if rec := post("/webhooks/basecamp/" + secret + "x"); rec.Code != http.StatusNotFound || rec.Body.Len() != 0 {
		t.Fatalf("an unknown secret: %d %q", rec.Code, rec.Body.String())
	}
	if rec := post("/webhooks/basecamp/" + secret); rec.Code != http.StatusNoContent {
		t.Fatalf("the project's secret, without a JWT: %d %q", rec.Code, rec.Body.String())
	}

	call := func(role domain.OrgRole) *httptest.ResponseRecorder {
		pair, err := repository.GenerateTokens("u-1", "ana", false, []domain.OrgMembershipClaim{{OrgID: "org-1", Role: role}})
		if err != nil {
			t.Fatal(err)
		}
		req := httptest.NewRequest(http.MethodPost, "/api/v1/organizations/org-1/basecamp/connect", nil)
		req.Header.Set("Authorization", "Bearer "+pair.AccessToken)
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, req)
		return rec
	}
	if rec := call(domain.OrgRoleAdmin); rec.Code != http.StatusServiceUnavailable || !strings.Contains(rec.Body.String(), "basecamp-off") {
		t.Fatalf("connecting without the app configured: %d %s", rec.Code, rec.Body.String())
	}
	if rec := call(domain.OrgRoleMember); rec.Code != http.StatusForbidden {
		t.Fatalf("a member who is not an admin connecting: %d %s", rec.Code, rec.Body.String())
	}
}

// «Por contestar» (command-and-control-558): cualquier miembro de la org ve
// los hilos de Basecamp que esperan respuesta; quien no es de la org no ve
// nada, ni siquiera que existen.
//
// Roturas que atrapa: abrir la lista a quien no es miembro; pedir más que
// mirar (un viewer también la ve); devolver hilos que ya no esperan.
func TestOnlyMembersSeeWhatAwaitsAReply(t *testing.T) {
	db, cleanup := githubDB(t)
	defer cleanup()
	if err := db.AutoMigrate(&domain.BasecampConnection{}, &domain.BasecampProject{}, &domain.BasecampPerson{},
		&domain.BasecampLink{}, &domain.BasecampEvent{}, &domain.BasecampThread{}); err != nil {
		t.Fatal(err)
	}
	if err := db.Create(&domain.BasecampProject{OrgID: "org-1", SpaceID: "sp-1", ProjectID: 42, Name: "Proteus",
		SecretHash: "h"}).Error; err != nil {
		t.Fatal(err)
	}
	since := time.Date(2026, 11, 26, 12, 0, 0, 0, time.UTC)
	for _, th := range []domain.BasecampThread{
		{OrgID: "org-1", RecordingID: 5001, ProjectID: 42, Title: "Hilo del cliente", AwaitingReply: true,
			AwaitingSince: &since, LastExternalName: "Cliente"},
		{OrgID: "org-1", RecordingID: 5002, ProjectID: 42, Title: "Ya contestado"},
	} {
		if err := db.Create(&th).Error; err != nil {
			t.Fatal(err)
		}
	}

	r := chi.NewRouter()
	InitBasecampRoutesWith(r, service.NewBasecampService(repository.NewBasecampRepository(db), service.BasecampConfig{}, nil, nil, nil))
	call := func(org string, role domain.OrgRole) *httptest.ResponseRecorder {
		pair, err := repository.GenerateTokens("u-1", "ana", false, []domain.OrgMembershipClaim{{OrgID: org, Role: role}})
		if err != nil {
			t.Fatal(err)
		}
		req := httptest.NewRequest(http.MethodGet, "/api/v1/organizations/org-1/basecamp/awaiting", nil)
		req.Header.Set("Authorization", "Bearer "+pair.AccessToken)
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, req)
		return rec
	}
	rec := call("org-1", domain.OrgRoleViewer)
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), "Hilo del cliente") ||
		strings.Contains(rec.Body.String(), "Ya contestado") {
		t.Fatalf("a viewer of org-1: %d %s", rec.Code, rec.Body.String())
	}
	rec = call("org-2", domain.OrgRoleAdmin)
	if rec.Code == http.StatusOK || strings.Contains(rec.Body.String(), "Hilo del cliente") {
		t.Fatalf("an admin of another org: %d %s", rec.Code, rec.Body.String())
	}
}
