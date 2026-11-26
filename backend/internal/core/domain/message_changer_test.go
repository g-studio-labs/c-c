package domain

import (
	"context"
	"testing"
)

// Quién cambia un mensaje sale de las claims de la petición, y de ahí tiene
// que llegar `ViaToken` (command-and-control-547). Sin él, el token de un
// superadmin corregiría lo de cualquiera y retiraría lo tecleado a mano.
//
// Roturas que detecta: no copiar ViaToken o Superadmin de las claims; no
// tomar un token por agente cuando el cliente no manda la marca.
func TestATokenChangesMessagesAsAnAgentAndNeverAsSuperadmin(t *testing.T) {
	token := ChangerFrom(context.Background(), &ClaimsJWT{UserID: "u1", Superadmin: true, ViaToken: true})
	if token.MayChangeOthers() || !token.AsAgent() || token.WrittenVia() != ViaMCP || token.MayWithdraw(ViaApp) {
		t.Errorf("un token es un agente, nunca superadmin, y no retira lo tecleado: %+v", token)
	}
	person := ChangerFrom(context.Background(), &ClaimsJWT{UserID: "u1", Superadmin: true})
	if !person.MayChangeOthers() || person.AsAgent() || !person.MayWithdraw(ViaApp) {
		t.Errorf("en persona, el superadmin sí: %+v", person)
	}
	viaHeader := ChangerFrom(WithVia(context.Background(), ViaMCP), &ClaimsJWT{UserID: "u1"})
	if viaHeader.WrittenVia() != ViaMCP {
		t.Errorf("lo que dice venir del MCP queda marcado: %+v", viaHeader)
	}
}
