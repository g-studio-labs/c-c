package http

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/adapters/handler"
)

// Ninguna ruta de pedidos se abre sin sesión: un pedido lleva borradores,
// comandos y decisiones de una organización.
//
// Roturas que atrapa: una ruta montada fuera del grupo con AuthMiddleware
// (la espera larga o el «lo abrí», que son las raras); una ruta que se cae
// del enrutado y la app recibe 404.
func TestEveryRequestRouteNeedsASession(t *testing.T) {
	r := chi.NewRouter()
	mountRequestRoutes(r, handler.NewRequestHandler(nil))
	want := map[string]bool{
		"GET /api/v1/requests/": true, "POST /api/v1/requests/": true,
		"GET /api/v1/requests/{id}": true, "PATCH /api/v1/requests/{id}": true,
		"POST /api/v1/requests/{id}/answer": true, "POST /api/v1/requests/{id}/withdraw": true,
		"POST /api/v1/requests/{id}/published": true, "POST /api/v1/requests/{id}/open": true,
		"POST /api/v1/requests/{id}/opened": true, "GET /api/v1/requests/{id}/wait": true,
	}
	seen := 0
	err := chi.Walk(r, func(method, route string, _ http.Handler, _ ...func(http.Handler) http.Handler) error {
		key := method + " " + route
		if !want[key] {
			t.Errorf("unexpected route %s", key)
			return nil
		}
		seen++
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, httptest.NewRequest(method, strings.ReplaceAll(route, "{id}", "req-7k2m9x"), nil))
		if rec.Code != http.StatusUnauthorized {
			t.Errorf("%s without a session → %d, want 401", key, rec.Code)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if seen != len(want) {
		t.Fatalf("saw %d of %d routes", seen, len(want))
	}
}
