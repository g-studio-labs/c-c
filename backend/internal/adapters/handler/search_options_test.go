package handler

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// El MCP no pide personas, canal ni directos, que tira; la app sí; y `kinds`
// manda sobre las dos (command-and-control-570).
//
// Roturas que atrapa: no mirar por dónde entra la búsqueda, darle al MCP lo
// de la app (o al revés), o ignorar `kinds`, con org y sin ella.
func TestTheSearchHandlerAsksOnlyForWhatReachesWhoSearches(t *testing.T) {
	claims := &domain.ClaimsJWT{UserID: "u-ana", Orgs: []domain.OrgMembershipClaim{{OrgID: "org-1", Role: domain.OrgRoleMember}}}
	ask := func(via, query string) domain.SearchOptions {
		t.Helper()
		f := &fakeSearcher{res: domain.SearchResults{Tasks: []domain.SearchHit{{ID: "t"}}}}
		h := &searchHandler{svc: f}
		r := httptest.NewRequest(http.MethodGet, "/api/v1/search/?q=despliegue"+query, nil)
		ctx := context.WithValue(r.Context(), repository.UserContextKey, claims)
		h.Search(httptest.NewRecorder(), r.WithContext(domain.WithVia(ctx, via)))
		if len(f.opts) != 1 {
			t.Fatalf("%s %q: el servicio se llamó %d veces", via, query, len(f.opts))
		}
		return f.opts[0]
	}
	for _, org := range []string{"", "&orgId=org-1"} {
		if got := ask(domain.ViaMCP, org); got != domain.SearchForAgent() {
			t.Errorf("%q: el MCP pidió %+v", org, got)
		}
		if got := ask(domain.ViaApp, org); got != domain.SearchForPeople() {
			t.Errorf("%q: la app pidió %+v", org, got)
		}
		if got := ask(domain.ViaApp, org+"&kinds=tasks,notes,docs"); got != (domain.SearchOptions{}) {
			t.Errorf("%q: con kinds=tasks,notes,docs la app pidió %+v", org, got)
		}
		if got := ask(domain.ViaMCP, org+"&kinds=docs,People"); got != (domain.SearchOptions{People: true, WholeShortWords: true}) {
			t.Errorf("%q: con kinds=docs,People el MCP pidió %+v", org, got)
		}
		if got := ask(domain.ViaMCP, org+"&kinds=messages,%20dms"); got != (domain.SearchOptions{Messages: true, DMs: true, WholeShortWords: true}) {
			t.Errorf("%q: con kinds=messages,dms el MCP pidió %+v", org, got)
		}
	}
}
