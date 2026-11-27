package events

import "testing"

// Quien escucha con un token (SubscribePersonal) sólo recibe el diario de su
// dueño: lo dirigido a él y con número. Ni su organización, ni un
// comunicado, ni lo de otra persona, ni un aviso suyo sin número.
func TestAPersonalSubscriberOnlyGetsItsOwnJournal(t *testing.T) {
	h := NewHub()
	ch, unsubscribe := h.SubscribePersonal("u-ana")
	defer unsubscribe()

	h.Publish(Event{Type: "task:moved", OrgID: "org-1"})
	h.Publish(Event{Type: "post:new", Broadcast: true})
	h.Publish(Event{Type: "notification:new", UserID: "u-bea", Seq: 1})
	h.Publish(Event{Type: "session:revoked", UserID: "u-ana"})
	h.Publish(Event{Type: "notification:new", UserID: "u-ana", Seq: 7})

	got := recv(t, ch)
	if got == nil || got.Seq != 7 || got.UserID != "u-ana" {
		t.Fatalf("se esperaba sólo su evento 7, llegó %+v", got)
	}
	if extra := recv(t, ch); extra != nil {
		t.Errorf("llegó algo más: %+v", extra)
	}
}

// El número cruza el bus: sin él, en otro pod el stream de un token no
// entregaría el evento.
func TestTheSeqSurvivesTheWire(t *testing.T) {
	payload, err := encodeEvent(Event{Type: "notification:new", UserID: "u-ana", Seq: 42, Data: map[string]int{"seq": 42}})
	if err != nil {
		t.Fatal(err)
	}
	out, err := decodeEvent(payload)
	if err != nil {
		t.Fatal(err)
	}
	if out.Seq != 42 || out.UserID != "u-ana" {
		t.Errorf("el número se perdió por el bus: %+v", out)
	}
}
