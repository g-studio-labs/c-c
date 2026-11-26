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

// Que corrijan un mensaje es de la misma familia que el mensaje: se calla con
// el mismo interruptor (command-and-control-547).
//
// Roturas que detecta: sacar `dm:edited` o `chat:edited` de su caso, que los
// dejaría sonar a quien calló los directos o los mensajes del canal.
func TestAnEditIsSilencedLikeItsMessage(t *testing.T) {
	quiet := DefaultPrefs("u")
	quiet.DMs = false
	if quiet.Allows(MessageEditedDM) {
		t.Error("quien calló los directos no oye que corrigieron uno")
	}
	if !quiet.Allows(MessageEditedChat) {
		t.Error("callar los directos no calla los canales")
	}
	quiet = DefaultPrefs("u")
	quiet.Messages = false
	if quiet.Allows(MessageEditedChat) {
		t.Error("quien calló los mensajes del canal no oye que corrigieron uno")
	}
	if !quiet.Allows(MessageEditedDM) {
		t.Error("callar los canales no calla los directos")
	}
}
