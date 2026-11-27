package domain

import (
	"strings"
	"testing"
	"time"
)

// El texto de un evento de Basecamp: las menciones como «@Nombre», el
// formato en markdown, los adjuntos fuera del texto y los enlaces que no son
// de Basecamp a la vista, no pulsables.
//
// Roturas que atrapa: perder el nombre de una mención o no ponerle la «@»;
// convertir un enlace de fuera en uno de markdown; cortar el texto.
func TestBasecampMarkdown(t *testing.T) {
	html := `<div>` + `<bc-attachment sgid="s" content-type="application/vnd.basecamp.mention"><figure><figcaption>Ana Bc</figcaption></figure></bc-attachment>` +
		` revisa <strong>esto</strong>:</div><ul><li>uno</li><li>dos</li></ul>` +
		`<div><a href="https://evil.example/x">aquí</a></div>` +
		`<bc-attachment sgid="f" content-type="image/png" filename="captura.png" href="https://storage.3.basecamp.com/1/x.png"></bc-attachment>` +
		`<div>` + strings.Repeat("a", 2000) + `</div>`
	got := BasecampMarkdown(html)
	for _, want := range []string{"@Ana Bc revisa **esto**:", "- uno\n- dos", "aquí (https://evil.example/x)", "[captura.png]",
		strings.Repeat("a", 2000)} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %q in\n%s", want, got)
		}
	}
	if strings.Contains(got, "](https://evil") || strings.Contains(got, "<") {
		t.Fatalf("markup or a clickable foreign link survived:\n%s", got)
	}
}

// El orden de un hilo lo pone la hora de Basecamp, no la llegada.
//
// Roturas que atrapa: aplicar lo viejo que llega tarde; que algo que no es un
// comentario o un mensaje mueva el estado; que una respuesta del equipo no
// limpie; mover `awaitingSince` con cada comentario de fuera.
func TestABasecampThreadFollowsBasecampsClock(t *testing.T) {
	at := func(m int) time.Time { return time.Date(2026, 11, 26, 12, m, 0, 0, time.UTC) }
	var th BasecampThread
	th.Apply(BasecampThreadWrite{Said: true, At: at(0), External: true, AuthorName: "Cliente"})
	th.Apply(BasecampThreadWrite{Said: true, At: at(2), External: true, AuthorName: "Cliente"})
	if !th.AwaitingReply || !th.AwaitingSince.Equal(at(0)) {
		t.Fatalf("awaiting since the first external comment: %+v", th)
	}
	th.Apply(BasecampThreadWrite{Said: false, At: at(3), External: false, AuthorUserID: "u-ana"})
	if !th.AwaitingReply {
		t.Fatal("an event that is not a comment cleared the thread")
	}
	th.Apply(BasecampThreadWrite{Said: true, At: at(1), External: false, AuthorUserID: "u-ana"})
	if !th.AwaitingReply {
		t.Fatal("an older team reply delivered late cleared the thread")
	}
	th.Apply(BasecampThreadWrite{Said: true, At: at(5), External: false, AuthorUserID: "u-ana"})
	if th.AwaitingReply || th.AwaitingSince != nil || !th.AnsweredBy("u-ana") || th.AnsweredBy("u-bob") {
		t.Fatalf("after Ana's reply: %+v", th)
	}
	th.Apply(BasecampThreadWrite{Said: true, At: at(4), External: true})
	if th.AwaitingReply || !th.AnsweredBy("u-ana") {
		t.Fatalf("an older client comment delivered late reopened the thread: %+v", th)
	}
}
