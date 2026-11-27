package service

import (
	"encoding/json"
	"strings"
	"testing"
	"unicode/utf8"

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

// journalFor: un diario de verdad sobre la base de la prueba, puesto como el
// del proceso.
func journalFor(t *testing.T, f *requestFixture) *EventJournal {
	t.Helper()
	if err := f.db.AutoMigrate(&domain.UserEvent{}, &domain.UserEventHead{}); err != nil {
		t.Fatal(err)
	}
	j := NewEventJournal(repository.NewUserEventRepository(f.db), nil)
	SetEventJournal(j)
	t.Cleanup(func() { SetEventJournal(nil) })
	return j
}

func journalOf(t *testing.T, j *EventJournal, user string) []domain.EventEnvelope {
	t.Helper()
	page, err := j.Read(user, 0, 50, map[string]bool{"org-1": true})
	if err != nil {
		t.Fatal(err)
	}
	return page.Events
}

func TestAnAnsweredRequestGoesOnlyToTheJournalOfWhoAskedForIt(t *testing.T) {
	f := newRequestFixture(t)
	j := journalFor(t, f)
	v := f.create(t, "cac-10", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestApproveMessage,
		`{"draft":"Hola equipo","destination":"Slack"}`)
	f.inbox.sent = nil

	yes := true
	if _, err := f.svc.Answer(f.find(t, v.Code), "u-bea", false,
		domain.RequestAnswer{Approved: &yes, FinalText: "Hola, equipo", Note: "con coma"}); err != nil {
		t.Fatal(err)
	}
	if len(f.inbox.sent) != 0 {
		t.Errorf("ni fila en la campana ni push: ninguna campana conoce esta clase: %+v", f.inbox.sent)
	}
	if got := journalOf(t, j, "u-bea"); len(got) != 0 {
		t.Errorf("quien contesta no recibe su propia respuesta: %+v", got)
	}
	got := journalOf(t, j, "u-ana")
	if len(got) != 1 || got[0].Type != domain.RequestAnsweredEvent || !got[0].Mine || got[0].OrgID != "org-1" {
		t.Fatalf("la respuesta llega al diario de quien lo pidió: %+v", got)
	}
	var data map[string]any
	_ = json.Unmarshal(got[0].Data, &data)
	raw, _ := json.Marshal(data)
	want := `{"approved":true,"code":"` + v.Code + `","finalText":"Hola, equipo","kind":"approve_message","note":"con coma","reason":""}`
	if string(raw) != want {
		t.Errorf("detalle de un pedido contestado:\n got %s\nwant %s", raw, want)
	}
}

func TestARejectionCarriesItsReasonAndNoOneIsToldTwice(t *testing.T) {
	f := newRequestFixture(t)
	j := journalFor(t, f)
	v := f.create(t, "cac-10", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestApproveMessage,
		`{"draft":"Hola equipo","destination":"Slack"}`)
	no := false
	q := f.find(t, v.Code)
	if _, err := f.svc.Answer(q, "u-bea", false, domain.RequestAnswer{Approved: &no, Reason: "mejor mañana"}); err != nil {
		t.Fatal(err)
	}
	got := journalOf(t, j, "u-ana")
	if len(got) != 1 {
		t.Fatalf("un evento: %+v", got)
	}
	var d map[string]any
	_ = json.Unmarshal(got[0].Data, &d)
	if d["approved"] != false || d["reason"] != "mejor mañana" {
		t.Errorf("un rechazo lleva su razón: %v", d)
	}
	// La misma respuesta apuntada otra vez (un reintento) no duplica.
	f.svc.notifyAnswered(q, "u-bea", domain.AnsweredViaApp, domain.RequestAnswer{Approved: &no, Reason: "mejor mañana"})
	if again := journalOf(t, j, "u-ana"); len(again) != 1 {
		t.Errorf("la clave evita duplicar: %+v", again)
	}
}

func TestWhoAnswersWhatHeAskedForIsToldNothing(t *testing.T) {
	f := newRequestFixture(t)
	j := journalFor(t, f)
	// El agente de Ana le abre un pedido a Ana, y Ana lo contesta en la app.
	v := f.create(t, "cac-10", "u-ana", "Ana Ruiz", domain.ViaMCP, domain.RequestApproveMessage,
		`{"draft":"Hola equipo","destination":"Slack"}`)
	f.inbox.sent = nil
	yes := true
	if _, err := f.svc.Answer(f.find(t, v.Code), "u-ana", false,
		domain.RequestAnswer{Approved: &yes, FinalText: "Hola equipo"}); err != nil {
		t.Fatal(err)
	}
	if len(f.inbox.sent) != 0 || len(journalOf(t, j, "u-ana")) != 0 {
		t.Errorf("quien contesta lo suyo no se entera de lo que acaba de hacer: %+v / %+v",
			f.inbox.sent, journalOf(t, j, "u-ana"))
	}
}

func TestASessionAnswerIsNotToldToTheSession(t *testing.T) {
	f := newRequestFixture(t)
	j := journalFor(t, f)
	w := f.create(t, "cac-10", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestApproveMessage,
		`{"draft":"Otra","destination":"Slack"}`)
	f.inbox.sent = nil
	if _, err := f.svc.Answer(f.find(t, w.Code), "u-bea", true, domain.RequestAnswer{Approved: &yesVal, FinalText: "Otra"}); err != nil {
		t.Fatal(err)
	}
	if len(f.inbox.sent) != 0 || len(journalOf(t, j, "u-ana")) != 0 {
		t.Errorf("lo que apunta la propia sesión no se le avisa a la sesión: %+v", f.inbox.sent)
	}
}

var yesVal = true

func TestAHugeMessageKeepsItsIdsAndSaysItWasCut(t *testing.T) {
	f := newRequestFixture(t)
	j := journalFor(t, f)
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	inbox := &requestInbox{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(inbox)

	// Con comillas y saltos de línea, que escapados en JSON pesan el doble.
	huge := strings.Repeat("ñ\"\n", 40*1024)
	m, err := svc.Reply("esp-1", "org-1", ana, huge, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(inbox.sent) == 0 {
		t.Fatal("alguien sigue el canal")
	}
	a := inbox.sent[0]
	n := &domain.Notification{UserID: a.UserID, OrgID: "org-1", Kind: a.Kind, Title: a.Title, Body: a.Body, Link: a.Link}
	n.ID = "n-1"
	if _, _, err = j.Append(notificationEvent(a, n)); err != nil {
		t.Fatal(err)
	}
	var data struct {
		Truncated      bool           `json:"truncated"`
		NotificationID string         `json:"notificationId"`
		Link           string         `json:"link"`
		Detail         map[string]any `json:"detail"`
	}
	got := journalOf(t, j, a.UserID)
	if len(got) != 1 {
		t.Fatalf("un evento: %+v", got)
	}
	_ = json.Unmarshal(got[0].Data, &data)
	if data.Truncated || data.NotificationID != "n-1" || data.Link == "" ||
		data.Detail["messageId"] != m.ID || data.Detail["spaceId"] != "esp-1" {
		t.Fatalf("el evento pierde sus ids: %s", got[0].Data)
	}
	text, _ := data.Detail["text"].(string)
	if data.Detail["textTruncated"] != true || text == "" || len(text) >= len(huge) || !strings.HasPrefix(huge, text) {
		t.Errorf("el texto va recortado y marcado: truncated=%v len=%d", data.Detail["textTruncated"], len(text))
	}
}

func TestClipTextNeverSplitsACharacterAndLeavesShortTextAlone(t *testing.T) {
	if s, cut := clipText("corto", 100); s != "corto" || cut {
		t.Errorf("un texto corto no se toca: %q %v", s, cut)
	}
	s, cut := clipText(strings.Repeat("ñ", 5000), 100)
	if !cut || !utf8.ValidString(s) || len(s) == 0 {
		t.Errorf("recorta sin partir un carácter: cut=%v valid=%v len=%d", cut, utf8.ValidString(s), len(s))
	}
	if b, _ := json.Marshal(s); len(b) > 100 {
		t.Errorf("escapado cabe en el tope: %d", len(b))
	}
}

func TestAHugeDirectIsCutToo(t *testing.T) {
	db, cleanup := dmDB(t)
	defer cleanup()
	repo := repository.NewDMRepository(db)
	inbox := &requestInbox{}
	svc := NewDMService(repo, events.NewHub()).WithNotifier(inbox)
	c, _ := repo.OpenWith("org-1", "u-ana", "u-bea")

	huge := strings.Repeat("palabra ", 20*1024)
	if _, err := svc.ReplyVia(domain.ViaMCP, c.ID, "u-ana", huge, ""); err != nil {
		t.Fatal(err)
	}
	d := detailOf(t, inbox.sent[0])
	text, _ := d["text"].(string)
	if d["textTruncated"] != true || len(text) >= len(huge) || d["threadId"] != c.ID || d["messageId"] == "" {
		t.Errorf("el directo enorme se recorta y conserva sus ids: truncated=%v len=%d %v", d["textTruncated"], len(text), d)
	}
}
