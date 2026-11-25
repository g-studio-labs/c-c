package handler

import (
	"net/http/httptest"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Una respuesta que llega por el MCP o con un token personal es de una sesión
// (la persona lo dijo en la terminal), y la de la app, de un botón.
//
// Roturas que atrapa: marcar como del botón lo que apuntó un agente (quien
// lea el pedido creería que la persona pulsó), o al revés.
func TestAnAnswerFromATokenIsMarkedAsFromASession(t *testing.T) {
	app := httptest.NewRequest("POST", "/api/v1/requests/x/answer", nil)
	if fromSession(app, &domain.ClaimsJWT{}) {
		t.Error("la app es un botón, no una sesión")
	}
	if !fromSession(app, &domain.ClaimsJWT{ViaToken: true}) {
		t.Error("con un token personal es una sesión")
	}
	mcp := app.WithContext(domain.WithVia(app.Context(), domain.ViaMCP))
	if !fromSession(mcp, &domain.ClaimsJWT{}) {
		t.Error("por el MCP es una sesión")
	}
}
