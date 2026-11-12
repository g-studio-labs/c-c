package middleware

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

func withPAT(t *testing.T, scopes ...string) {
	t.Helper()
	antes, antesRec := patAuth, agentCallRecorder
	t.Cleanup(func() { patAuth, agentCallRecorder = antes, antesRec })
	UsePATAuthenticator(func(string) (*domain.ClaimsJWT, error) {
		return &domain.ClaimsJWT{UserID: "u1", Scopes: scopes}, nil
	})
}

func patRequest(method, path string, headers map[string]string) *http.Request {
	req := httptest.NewRequest(method, path, strings.NewReader("{}"))
	req.Header.Set("Authorization", "Bearer "+repository.PATPrefix+"x")
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	return req
}

// Una llamada del MCP se cuenta con su herramienta y lo que tocó, y sólo si la
// declara (command-and-control-347).
//
// Roturas que atrapa: no contar; contar lo que no llega por el MCP (la app no
// es un agente); contar sin `X-Cac-Call` (sin él, cada petición de una misma
// herramienta sumaría); perder el id que tocó, que es con lo que se cruza lo
// que enseñan los hooks.
func TestAnAgentCallIsCountedWhenTheMCPNamesItsTool(t *testing.T) {
	withPAT(t)
	got := make(chan [4]string, 4)
	UseAgentCallRecorder(func(u, tool, call, target string) { got <- [4]string{u, tool, call, target} })
	h := AuthMiddleware(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	mcp := map[string]string{domain.HeaderVia: domain.ViaMCP, domain.HeaderAgentTool: "get_incident",
		domain.HeaderAgentCall: "c-1", domain.HeaderAgentTarget: "inc-9"}

	h.ServeHTTP(httptest.NewRecorder(), patRequest(http.MethodGet, "/api/v1/incidents/inc-9", mcp))
	select {
	case g := <-got:
		if g != [4]string{"u1", "get_incident", "c-1", "inc-9"} {
			t.Errorf("cuenta otra cosa: %v", g)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no cuenta la llamada del MCP")
	}

	app := map[string]string{domain.HeaderAgentTool: "get_incident", domain.HeaderAgentCall: "c-2"}
	noCall := map[string]string{domain.HeaderVia: domain.ViaMCP, domain.HeaderAgentTool: "get_incident"}
	for name, hs := range map[string]map[string]string{"sin Via": app, "sin X-Cac-Call": noCall} {
		h.ServeHTTP(httptest.NewRecorder(), patRequest(http.MethodGet, "/api/v1/incidents/x", hs))
		select {
		case g := <-got:
			t.Errorf("%s: cuenta %v", name, g)
		case <-time.After(200 * time.Millisecond):
		}
	}
}

// Cualquier token puede decir qué le enseñó un hook —sólo habla de su propio
// uso—, y eso no abre nada más.
//
// Roturas que atrapa: exigir un scope (los hooks usan el token del MCP, que
// puede ser de sólo lectura, y no contaría nada); que la excepción abra otro
// endpoint; que un token sin scope escriba donde antes no podía.
func TestAnyTokenCanReportWhatItWasShownAndNothingElse(t *testing.T) {
	withPAT(t)
	reached := false
	h := AuthMiddleware(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { reached = true }))

	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, patRequest(http.MethodPost, "/api/v1/agents/shown", nil))
	if !reached || rec.Code != http.StatusOK {
		t.Fatalf("un token sin scopes no puede decir lo que se le enseñó: %d", rec.Code)
	}
	for _, path := range []string{"/api/v1/agents/shown/x", "/api/v1/agents/usage", "/api/v1/tasks/t1/comments"} {
		reached = false
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, patRequest(http.MethodPost, path, nil))
		if reached || rec.Code != http.StatusForbidden {
			t.Errorf("%s: un token sin scopes escribe (%d)", path, rec.Code)
		}
	}
}
