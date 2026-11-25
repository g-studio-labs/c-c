package domain

import (
	"context"
	"testing"
)

// Lo que escribe un token personal se firma como de un agente de esa
// persona, aunque el cliente no diga cuál; lo que escribe una sesión de la
// app, como la persona, aunque mande `agent` (command-and-control-522).
//
// Roturas que detecta: firmar como persona lo que escribe un token que no se
// declara (lo de antes: `handoffBy` decía «Jose Guzman»); aceptar la firma de
// agente de una sesión de la app; perder el rol o el modelo; no limpiar los
// espacios; y que lo guardado por la app salga firmado por un agente.
func TestATokenSignsAsAnAgentOfItsOwner(t *testing.T) {
	sent := &AgentStamp{Role: " builder ", Model: "sonnet", Session: "s-1"}

	if a := AgentStampFor(&ClaimsJWT{UserID: "u1"}, sent); a != nil {
		t.Errorf("una sesión de la app firmó como agente: %+v", a)
	}
	if a := AgentStampFor(nil, sent); a != nil {
		t.Errorf("sin claims firmó como agente: %+v", a)
	}
	if a := AgentStampFor(&ClaimsJWT{UserID: "u1", ViaToken: true}, nil); a == nil {
		t.Error("un token que no dice qué agente es firmó como la persona")
	}
	got := AgentStampFor(&ClaimsJWT{UserID: "u1", ViaToken: true}, sent)
	if got == nil || *got != (AgentStamp{Role: "builder", Model: "sonnet", Session: "s-1"}) {
		t.Errorf("la firma del agente: %+v", got)
	}

	if s := SignedByAgent(ViaApp, "Ana", AgentStamp{Role: "builder"}); s != nil {
		t.Errorf("lo guardado por la app salió firmado por un agente: %+v", s)
	}
	s := SignedByAgent(ViaMCP, "Ana", *got)
	if s == nil || *s != (AgentSignature{Of: "Ana", Role: "builder", Model: "sonnet", Session: "s-1"}) {
		t.Errorf("la firma que se contesta: %+v", s)
	}

	ctx := WithAgent(context.Background(), got)
	if AgentFrom(ctx) != got || AgentFrom(WithAgent(context.Background(), nil)) != nil {
		t.Error("la firma no llega al servicio por el contexto")
	}
}
