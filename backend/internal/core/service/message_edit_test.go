package service

import (
	"errors"
	"sort"
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Corregir y retirar un mensaje de canal o de directo, desde la app y desde un
// agente con el token de su dueño (command-and-control-547).
//
// Roturas que detecta: avisar de la corrección a quien sigue el canal (o a
// nadie); no avisar a quien contestaba el original; avisar a quien corrige;
// poner el texto de un directo en el aviso; no decir que lo corrigió un
// agente; dejar que el token de un superadmin corrija o retire lo ajeno;
// dejar que un token retire lo que la persona tecleó; avisar al retirar; y
// no cambiar la marca `via` al corregir.

// editSpy: un espía que guarda el aviso entero, cuerpo incluido.
type editSpy struct{ all []domain.Aviso }

func (n *editSpy) Notify(a domain.Aviso) { n.all = append(n.all, a) }

func (n *editSpy) of(kind string) []domain.Aviso {
	var out []domain.Aviso
	for _, a := range n.all {
		if a.Kind == kind {
			out = append(out, a)
		}
	}
	return out
}

func recipients(list []domain.Aviso) []string {
	out := []string{}
	for _, a := range list {
		out = append(out, a.UserID)
	}
	sort.Strings(out)
	return out
}

func viaOf(t *testing.T, svc *ChatService, id string) string {
	t.Helper()
	msgs, err := svc.List("esp-1", "", time.Time{}, 50)
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range msgs {
		if m.ID == id {
			return m.Via
		}
	}
	t.Fatalf("el mensaje %s no está en el canal", id)
	return ""
}

var anasAgent = domain.MessageChanger{UserID: ana, ViaToken: true, Via: domain.ViaMCP}

func TestEditingAChannelMessageTellsWhoTheOriginalNamed(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	spy := &editSpy{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(spy)

	m, err := svc.Post("esp-1", "org-1", ana, "oye [bea](cac:user/"+bea+") mira el PR")
	if err != nil {
		t.Fatal(err)
	}
	spy.all = nil
	if err := svc.EditAs(anasAgent, m.ID, "oye [bea](cac:user/"+bea+") mira el PR #12"); err != nil {
		t.Fatal(err)
	}
	got := spy.of(domain.MessageEditedChat)
	if r := recipients(got); len(r) != 1 || r[0] != bea {
		t.Fatalf("sólo a bea, a quien nombraba; no a caro, que sigue el canal, ni a ana: %v", r)
	}
	a := got[0]
	if a.TitleKey != "notify.message.edited.agent.in" || a.TitleArgs["who"] != "ana" || a.Via != domain.ViaMCP {
		t.Errorf("tiene que decir que lo corrigió el agente de ana: %+v", a)
	}
	if a.Body == "" || a.Link != "/chat?space=esp-1" {
		t.Errorf("en un canal lleva el texto nuevo y el enlace al canal: %+v", a)
	}
	if len(spy.all) != 1 {
		t.Errorf("una corrección es un aviso, no una mención nueva ni un mensaje corriente: %+v", spy.all)
	}
	if v := viaOf(t, svc, m.ID); v != domain.ViaMCP {
		t.Errorf("corregido por un agente se lee «vía agente»; via %q", v)
	}

	// Corregido a mano, el aviso es de la persona y la marca se va.
	spy.all = nil
	if err := svc.EditAs(domain.MessageChanger{UserID: ana}, m.ID, "oye [bea](cac:user/"+bea+") ya está"); err != nil {
		t.Fatal(err)
	}
	if got := spy.of(domain.MessageEditedChat); len(got) != 1 || got[0].TitleKey != "notify.message.edited.in" || got[0].Via != "" {
		t.Errorf("a mano, «ana editó su mensaje» sin agente: %+v", got)
	}
	if v := viaOf(t, svc, m.ID); v != "" {
		t.Errorf("el texto que queda lo tecleó ana; via %q", v)
	}
}

func TestEditingAReplyTellsWhomItAnswered(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	spy := &editSpy{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(spy)

	q, err := svc.Post("esp-1", "org-1", bea, "¿quién revisa?")
	if err != nil {
		t.Fatal(err)
	}
	r, err := svc.Reply("esp-1", "org-1", caro, "yo", q.ID)
	if err != nil {
		t.Fatal(err)
	}
	spy.all = nil
	if err := svc.Edit(r.ID, caro, false, "yo, a las 5"); err != nil {
		t.Fatal(err)
	}
	if got := recipients(spy.of(domain.MessageEditedChat)); len(got) != 1 || got[0] != bea {
		t.Errorf("a bea, a quien contestaba; avisados %v", got)
	}

	// Contestarse a uno mismo no hace que te avises de tu corrección.
	self, _ := svc.Reply("esp-1", "org-1", caro, "y otra cosa", r.ID)
	spy.all = nil
	if err := svc.Edit(self.ID, caro, false, "y otra cosa más"); err != nil {
		t.Fatal(err)
	}
	if len(spy.all) != 0 {
		t.Errorf("caro se contestaba a sí misma: nadie a quien avisar, %+v", spy.all)
	}

	// Un mensaje que no avisó a nadie por su nombre no avisa al corregirse.
	plain, _ := svc.Post("esp-1", "org-1", ana, "hola")
	spy.all = nil
	if err := svc.Edit(plain.ID, ana, false, "hola a todos"); err != nil {
		t.Fatal(err)
	}
	if len(spy.all) != 0 {
		t.Errorf("sin nombrar a nadie, corregir no avisa: %+v", spy.all)
	}
}

func TestATokenOnlyChangesItsOwnersMessages(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	svc := NewChatService(repository.NewChatRepository(db), nil)

	beas, _ := svc.ReplyVia(domain.ViaMCP, "esp-1", "org-1", bea, "lo escribió el agente de bea", "")
	rootsToken := domain.MessageChanger{UserID: ana, Superadmin: true, ViaToken: true, Via: domain.ViaMCP}
	if err := svc.EditAs(rootsToken, beas.ID, "x"); !errors.Is(err, ErrNotTheAuthor) {
		t.Errorf("el token de un superadmin no corrige lo ajeno: %v", err)
	}
	if err := svc.WithdrawAs(rootsToken, beas.ID); !errors.Is(err, ErrNotTheAuthor) {
		t.Errorf("ni lo retira: %v", err)
	}
	if err := svc.EditAs(domain.MessageChanger{UserID: ana, Superadmin: true}, beas.ID, "limpio"); err != nil {
		t.Errorf("el superadmin en persona sí: %v", err)
	}
}

func TestATokenWithdrawsOnlyWhatAnAgentWrote(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	spy := &editSpy{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(spy)

	typed, _ := svc.Post("esp-1", "org-1", ana, "lo tecleé yo, [bea](cac:user/"+bea+")")
	byAgent, _ := svc.ReplyVia(domain.ViaMCP, "esp-1", "org-1", ana, "lo dejó el agente, [bea](cac:user/"+bea+")", "")
	spy.all = nil

	if err := svc.WithdrawAs(anasAgent, typed.ID); !errors.Is(err, ErrTokenWithdrawsOnlyAgentWords) {
		t.Errorf("un token no retira lo que ana tecleó: %v", err)
	}
	if err := svc.WithdrawAs(anasAgent, byAgent.ID); err != nil {
		t.Errorf("lo que escribió el agente sí: %v", err)
	}
	if err := svc.WithdrawAs(domain.MessageChanger{UserID: ana}, typed.ID); err != nil {
		t.Errorf("ana en la app retira lo suyo: %v", err)
	}
	if len(spy.all) != 0 {
		t.Errorf("retirar no deja aviso en la campana: %+v", spy.all)
	}
}

func TestEditingADirectMessageTellsTheOtherWithoutTheText(t *testing.T) {
	db, cleanup := dmDB(t)
	defer cleanup()
	repo := repository.NewDMRepository(db)
	spy := &editSpy{}
	dms := NewDMService(repo, nil).WithNotifier(spy)
	c, _ := repo.OpenWith("org-1", "u-ana", "u-bea")

	m, err := dms.Post(c.ID, "u-ana", "¿vienes?")
	if err != nil {
		t.Fatal(err)
	}
	spy.all = nil
	agent := domain.MessageChanger{UserID: "u-ana", ViaToken: true, Via: domain.ViaMCP}
	if err := dms.EditAs(agent, c.ID, m.ID, "¿vienes hoy a las 5?"); err != nil {
		t.Fatal(err)
	}
	got := spy.of(domain.MessageEditedDM)
	if len(got) != 1 || got[0].UserID != "u-bea" {
		t.Fatalf("a bea y a nadie más: %+v", spy.all)
	}
	a := got[0]
	if a.Body != "" {
		t.Errorf("un directo no lleva el texto en el aviso: %q", a.Body)
	}
	if a.TitleKey != "notify.message.edited.agent" || a.TitleArgs["who"] != "ana" || a.Via != domain.ViaMCP || a.Link != "/dm?c="+c.ID {
		t.Errorf("«el agente de ana editó su mensaje», hacia la conversación: %+v", a)
	}
	if len(spy.all) != 1 {
		t.Errorf("corregir no es un directo nuevo: %+v", spy.all)
	}

	// La otra persona no corrige lo de ana, ni retira con su token.
	if err := dms.EditAs(domain.MessageChanger{UserID: "u-bea", Superadmin: true, ViaToken: true}, c.ID, m.ID, "x"); !errors.Is(err, ErrNotTheAuthor) {
		t.Errorf("el token de un superadmin no corrige lo ajeno: %v", err)
	}

	typed, _ := dms.Post(c.ID, "u-ana", "a mano")
	spy.all = nil
	if err := dms.WithdrawAs(agent, c.ID, typed.ID); !errors.Is(err, ErrTokenWithdrawsOnlyAgentWords) {
		t.Errorf("un token no retira lo que ana tecleó: %v", err)
	}
	// El que corrigió el agente quedó «vía agente», así que el agente sí.
	if err := dms.WithdrawAs(agent, c.ID, m.ID); err != nil {
		t.Errorf("lo que dejó el agente sí: %v", err)
	}
	if len(spy.all) != 0 {
		t.Errorf("retirar no deja aviso: %+v", spy.all)
	}
}
