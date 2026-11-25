package service

import (
	"context"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Un handoff y un comentario escritos con token salen en el detalle firmados
// como de un agente de la persona, con cuál; los escritos desde la app, como
// la persona. Y un handoff nuevo de la app no hereda la firma del agente que
// escribió el anterior (command-and-control-522).
//
// Roturas que detecta: no guardar la firma del handoff (o no leerla al
// contestar); dejar puesta la de un agente cuando después escribe la persona;
// no guardar qué agente escribió un comentario; y firmar como agente lo que
// comenta la persona.
func TestWhatAnAgentWritesIsSignedAsItsOwnersAgent(t *testing.T) {
	db, cleanup := myWorkDB(t)
	defer cleanup()
	svc := myWorkSvc(db)
	jose := &domain.User{Username: "jose", Name: "Jose Guzman"}
	jose.ID = "u-jose"
	if err := db.Create(jose).Error; err != nil {
		t.Fatal(err)
	}
	agent := domain.WithAgent(context.Background(), &domain.AgentStamp{Role: "builder", Model: "sonnet", Session: "s-9"})
	app := context.Background()

	if err := svc.UpdateTask(agent, "it-creada", "u-jose", domain.UpdateTaskRequest{Handoff: ptr("Falta la release.")}); err != nil {
		t.Fatal(err)
	}
	d, err := svc.Detail("it-creada")
	if err != nil {
		t.Fatal(err)
	}
	want := domain.AgentSignature{Of: "Jose Guzman", Role: "builder", Model: "sonnet", Session: "s-9"}
	if d.HandoffAgent == nil || *d.HandoffAgent != want || d.HandoffBy != "Jose Guzman" {
		t.Fatalf("el handoff de un agente: %q %+v", d.HandoffBy, d.HandoffAgent)
	}

	if err := svc.UpdateTask(app, "it-creada", "u-jose", domain.UpdateTaskRequest{Handoff: ptr("Ya salió.")}); err != nil {
		t.Fatal(err)
	}
	if d, _ = svc.Detail("it-creada"); d.HandoffAgent != nil || d.HandoffBy != "Jose Guzman" {
		t.Fatalf("el handoff de la persona salió firmado por un agente: %+v", d.HandoffAgent)
	}

	if _, err := svc.AddComment(agent, "it-creada", "u-jose", "Hallazgos.", ""); err != nil {
		t.Fatal(err)
	}
	if _, err := svc.AddComment(app, "it-creada", "u-jose", "Lo miro yo.", ""); err != nil {
		t.Fatal(err)
	}
	d, _ = svc.Detail("it-creada")
	signed := map[string]*domain.AgentSignature{}
	for _, c := range d.Comments {
		signed[c.Body] = c.Agent
	}
	if a := signed["Hallazgos."]; a == nil || *a != want {
		t.Fatalf("el comentario de un agente: %+v", a)
	}
	if a, ok := signed["Lo miro yo."]; !ok || a != nil {
		t.Fatalf("el comentario de la persona salió firmado por un agente: %+v (%v)", a, ok)
	}
}
