package middleware

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// `events:read` es escuchar y nada más (command-and-control-558): ninguna
// ruta que escribe lo pide, y un token que sólo lo tiene no escribe en
// ninguna. Si alguien lo añade a la tabla de rutas de un token, esto lo dice.
func TestEventsReadOpensNoMutation(t *testing.T) {
	for _, e := range patWritable {
		if e.scope == domain.ScopeEventsRead {
			t.Errorf("%s %s pide events:read: escuchar abriría una escritura", e.method, e.path)
		}
	}

	antes := patAuth
	defer func() { patAuth = antes }()
	UsePATAuthenticator(func(string) (*domain.ClaimsJWT, error) {
		return &domain.ClaimsJWT{UserID: "u1", Superadmin: true, Scopes: []string{domain.ScopeEventsRead}}, nil
	})
	h := AuthMiddleware(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusTeapot)
	}))
	for _, e := range patWritable {
		// Lo que un token cuenta de sí mismo (qué se le enseñó) lo hace
		// cualquier token: no es escribir en nada de nadie.
		if e.scope == scopeSelfReport {
			continue
		}
		path := samplePath(e.path.String())
		if !e.path.MatchString(path) {
			t.Fatalf("la ruta de muestra %q no casa con %s", path, e.path)
		}
		req := httptest.NewRequest(e.method, path, nil)
		req.Header.Set("Authorization", "Bearer "+repository.PATPrefix+"escucha")
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code == http.StatusTeapot {
			t.Errorf("%s %s pasó con un token que sólo escucha", e.method, req.URL.Path)
		}
	}
}

// samplePath: una ruta que casa con el patrón, para pedirla.
func samplePath(pattern string) string {
	out := []rune{}
	rs := []rune(pattern)
	for i := 0; i < len(rs); i++ {
		switch {
		case rs[i] == '^' || rs[i] == '$' || rs[i] == '?':
		case rs[i] == '/' && i+1 < len(rs) && rs[i+1] == '?':
			// «/?» final: sin barra.
		case rs[i] == '(':
			// Un grupo «(a|b)»: la primera alternativa.
			i++
			for i < len(rs) && rs[i] != '|' && rs[i] != ')' {
				out = append(out, rs[i])
				i++
			}
			for i < len(rs) && rs[i] != ')' {
				i++
			}
		case rs[i] == '[':
			start := i
			for i < len(rs) && rs[i] != ']' {
				i++
			}
			// Una clase de sólo dígitos (el id de un recording de Basecamp)
			// pide un número, no un UUID.
			digits := string(rs[start+1:i]) == "0-9"
			if i+1 < len(rs) && (rs[i+1] == '+') {
				i++
			} else if i+3 < len(rs) && rs[i+1] == '{' {
				for i < len(rs) && rs[i] != '}' {
					i++
				}
			}
			if digits {
				out = append(out, '1')
			} else {
				out = append(out, []rune("00000000-0000-0000-0000-000000000000")...)
			}
		default:
			out = append(out, rs[i])
		}
	}
	return string(out)
}
