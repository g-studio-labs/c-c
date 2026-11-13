package handler

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Lo que crea un agente nace interno; lo que crea una persona, como siempre.
// Y el agente publica cuando se lo piden. Roturas que detecta: no mirar si es
// un token (todo interno, también lo de las personas), no aplicarlo (un agente
// vuelve a publicar sin querer), o pisar un `public` pedido a propósito.
func TestAnAgentsTaskIsBornInternal(t *testing.T) {
	pide := func(body string, viaToken bool) domain.ItemVisibility {
		r := httptest.NewRequest(http.MethodPost, "/api/v1/task-lists/l/tasks", strings.NewReader(body))
		r.Header.Set("Content-Type", "application/json")
		c := &domain.ClaimsJWT{UserID: "u-1", ViaToken: viaToken}
		r = r.WithContext(context.WithValue(r.Context(), repository.UserContextKey, c))
		req, err := createTaskRequest(r)
		if err != nil {
			t.Fatal(err)
		}
		return req.Visibility
	}
	if v := pide(`{"title":"Migrar a Postgres"}`, true); v != domain.VisibilityInternal {
		t.Errorf("un agente sin decir nada crea %q, no interna", v)
	}
	if v := pide(`{"title":"Lo que pidió el cliente","visibility":"public"}`, true); v != domain.VisibilityPublic {
		t.Errorf("un agente que pide publicarla crea %q", v)
	}
	if v := pide(`{"title":"Desde la app"}`, false); v != "" {
		t.Errorf("una persona en la app ya no crea como antes: %q", v)
	}
}

// Lo que comenta un agente es una nota interna salvo que pida publicarla, y
// queda marcado como de un agente; una persona en la app comenta como siempre
// (command-and-control-359). En Portento, donde el cliente ve el tablero, las
// notas de trabajo de los agentes le salían a él.
//
// Roturas que detecta: no mirar si es un token (las personas también
// comentarían en interno), no aplicarlo (un agente vuelve a comentar a la
// vista del cliente), pisar un `public` pedido a propósito, o no marcar como de
// un agente lo que escribe un token que no lo declara.
func TestAnAgentsCommentIsAnInternalNote(t *testing.T) {
	origin := func(want domain.ItemVisibility, viaToken bool) (string, domain.ItemVisibility) {
		r := httptest.NewRequest(http.MethodPost, "/api/v1/tasks/t/comments", nil)
		c := &domain.ClaimsJWT{UserID: "u-1", ViaToken: viaToken}
		r = r.WithContext(context.WithValue(r.Context(), repository.UserContextKey, c))
		ctx, got := commentOrigin(r, want)
		return domain.ViaFrom(ctx), got
	}
	if via, v := origin("", true); v != domain.VisibilityInternal || via != domain.ViaMCP {
		t.Errorf("un agente sin decir nada comenta %q por %q; quiero interno y marcado como agente", v, via)
	}
	if via, v := origin(domain.VisibilityPublic, true); v != domain.VisibilityPublic || via != domain.ViaMCP {
		t.Errorf("un agente que pide publicar comenta %q por %q", v, via)
	}
	if via, v := origin("", false); v != "" || via != domain.ViaApp {
		t.Errorf("una persona en la app ya no comenta como antes: %q por %q", v, via)
	}
}
