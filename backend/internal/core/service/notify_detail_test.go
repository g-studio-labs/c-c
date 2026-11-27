package service

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// El `detail` de un aviso (command-and-control-558): lo que el diario de
// eventos lleva y la campana no cabe.
//
// Roturas que atrapan: dejar el texto en los 400 caracteres del cuerpo; perder
// el aviso de uno de los tres caminos del canal (nombrado, contestado,
// seguidor); decir que escribió una persona lo que escribió un agente; que el
// directo salga a alguien que no sea el otro participante; perder el
// `finalText` o la razón de un rechazo; avisar a quien lo apunta desde su
// propia terminal.

func longText() string {
	return strings.Repeat("palabra ", 120) + "y el final que no cabe en el aviso"
}

func detailOf(t *testing.T, a domain.Aviso) map[string]any {
	t.Helper()
	d, ok := a.Detail.(map[string]any)
	if !ok {
		t.Fatalf("el aviso %s no lleva detalle: %#v", a.Kind, a.Detail)
	}
	return d
}

func TestAChannelNoticeCarriesTheWholeText(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	inbox := &requestInbox{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(inbox)

	text := longText()
	if _, err := svc.ReplyVia(domain.ViaMCP, "esp-1", "org-1", ana, text, ""); err != nil {
		t.Fatal(err)
	}
	if len(inbox.sent) != 2 {
		t.Fatalf("bea y caro siguen el canal: %+v", inbox.sent)
	}
	for _, a := range inbox.sent {
		d := detailOf(t, a)
		if d["text"] != text || len(a.Body) >= len(text) {
			t.Errorf("el detalle lleva el texto entero y el cuerpo sólo el adelanto: %d / %d", len(a.Body), len(text))
		}
		if d["spaceId"] != "esp-1" || d["author"] != "ana" || d["viaAgent"] != true || d["messageId"] == "" {
			t.Errorf("detalle de un mensaje de canal: %v", d)
		}
	}
}

func TestANamedOrAnsweredPersonGetsTheWholeTextToo(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	inbox := &requestInbox{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(inbox)

	first, err := svc.Post("esp-1", "org-1", caro, "¿alguien mira esto?")
	if err != nil {
		t.Fatal(err)
	}
	inbox.sent = nil
	text := "[bea](cac:user/" + bea + ") " + longText()
	if _, err := svc.Reply("esp-1", "org-1", ana, text, first.ID); err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, a := range inbox.sent {
		d := detailOf(t, a)
		if d["text"] != text || d["viaAgent"] != false {
			t.Errorf("%s: el detalle lleva el texto entero y no es de un agente: %v", a.Kind, d)
		}
		seen[a.UserID] = true
	}
	if len(inbox.sent) != 2 || !seen[bea] || !seen[caro] {
		t.Fatalf("bea (nombrada) y caro (contestado) reciben el suyo: %+v", inbox.sent)
	}
}

func TestADirectCarriesItsTextOnlyToTheOtherParticipant(t *testing.T) {
	db, cleanup := dmDB(t)
	defer cleanup()
	repo := repository.NewDMRepository(db)
	inbox := &requestInbox{}
	svc := NewDMService(repo, events.NewHub()).WithNotifier(inbox)
	c, _ := repo.OpenWith("org-1", "u-ana", "u-bea")

	text := longText()
	if _, err := svc.ReplyVia(domain.ViaMCP, c.ID, "u-ana", text, ""); err != nil {
		t.Fatal(err)
	}
	if len(inbox.sent) != 1 || inbox.sent[0].UserID != "u-bea" {
		t.Fatalf("sólo el otro participante recibe: %+v", inbox.sent)
	}
	a := inbox.sent[0]
	d := detailOf(t, a)
	if a.Body != "" || d["text"] != text || d["threadId"] != c.ID || d["author"] != "ana" || d["viaAgent"] != true {
		t.Errorf("el cuerpo de la campana sigue vacío y el texto va en el detalle: body=%q %v", a.Body, d)
	}
}

func TestAnAnsweredRequestCarriesItsFinalTextToWhoAskedForIt(t *testing.T) {
	f := newRequestFixture(t)
	v := f.create(t, "cac-10", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestApproveMessage,
		`{"draft":"Hola equipo","destination":"Slack"}`)
	f.inbox.sent = nil

	yes := true
	if _, err := f.svc.Answer(f.find(t, v.Code), "u-bea", false,
		domain.RequestAnswer{Approved: &yes, FinalText: "Hola, equipo", Note: "con coma"}); err != nil {
		t.Fatal(err)
	}
	if len(f.inbox.sent) != 1 || f.inbox.sent[0].UserID != "u-ana" || f.inbox.sent[0].Kind != domain.RequestAnsweredNotifyKind {
		t.Fatalf("la respuesta le llega a quien lo pidió: %+v", f.inbox.sent)
	}
	d := detailOf(t, f.inbox.sent[0])
	raw, _ := json.Marshal(d)
	want := `{"approved":true,"code":"` + v.Code + `","finalText":"Hola, equipo","kind":"approve_message","note":"con coma","reason":""}`
	if string(raw) != want {
		t.Errorf("detalle de un pedido contestado:\n got %s\nwant %s", raw, want)
	}
}

func TestARejectionCarriesItsReasonAndASessionAnswerDoesNotNotify(t *testing.T) {
	f := newRequestFixture(t)
	v := f.create(t, "cac-10", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestApproveMessage,
		`{"draft":"Hola equipo","destination":"Slack"}`)
	f.inbox.sent = nil
	no := false
	if _, err := f.svc.Answer(f.find(t, v.Code), "u-bea", false,
		domain.RequestAnswer{Approved: &no, Reason: "mejor mañana"}); err != nil {
		t.Fatal(err)
	}
	d := detailOf(t, f.inbox.sent[0])
	if d["approved"] != false || d["reason"] != "mejor mañana" {
		t.Errorf("un rechazo lleva su razón: %v", d)
	}

	w := f.create(t, "cac-10", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestApproveMessage,
		`{"draft":"Otra","destination":"Slack"}`)
	f.inbox.sent = nil
	if _, err := f.svc.Answer(f.find(t, w.Code), "u-bea", true, domain.RequestAnswer{Approved: &yesVal, FinalText: "Otra"}); err != nil {
		t.Fatal(err)
	}
	if len(f.inbox.sent) != 0 {
		t.Errorf("lo que apunta la propia sesión no se le avisa a la sesión: %+v", f.inbox.sent)
	}
}

var yesVal = true
