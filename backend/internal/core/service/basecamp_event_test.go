package service

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Basecamp «por contestar» y el evento completo (command-and-control-558, D),
// contra el Basecamp de mentira de basecamp_test.go.

// withJournal: el diario del evento completo, sólo de este servicio.
func (f *bcFixture) withJournal(t *testing.T) *EventJournal {
	t.Helper()
	j := NewEventJournal(repository.NewUserEventRepository(f.db), nil)
	f.svc.WithJournal(j)
	return j
}

// bcEvents: los `basecamp:event` del diario de una persona, leídos como
// miembro de org-1 y org-2.
func bcEvents(t *testing.T, j *EventJournal, user string) []domain.EventEnvelope {
	t.Helper()
	page, err := j.Read(user, 0, 100, map[string]bool{"org-1": true, "org-2": true})
	if err != nil {
		t.Fatal(err)
	}
	var out []domain.EventEnvelope
	for _, e := range page.Events {
		if e.Type == domain.UserEventBasecamp {
			out = append(out, e)
		}
	}
	return out
}

func eventData(t *testing.T, e domain.EventEnvelope) map[string]any {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal(e.Data, &m); err != nil {
		t.Fatal(err)
	}
	return m
}

// commentAt: un comentario en el to-do 5001, escrito por `who` a la hora `at`.
func commentAt(eventID, commentID int64, who domain.BasecampPersonWire, content, at string) []byte {
	var ev map[string]any
	_ = json.Unmarshal(commentEvent(eventID, commentID, who, content), &ev)
	ev["created_at"] = at
	raw, _ := json.Marshal(ev)
	return raw
}

func (f *bcFixture) thread(t *testing.T, recording int64) *domain.BasecampThread {
	t.Helper()
	var th domain.BasecampThread
	if err := f.db.First(&th, "org_id = ? AND recording_id = ?", "org-1", recording).Error; err != nil {
		t.Fatalf("thread %d: %v", recording, err)
	}
	return &th
}

func (f *bcFixture) awaiting(t *testing.T) []domain.BasecampAwaiting {
	t.Helper()
	list, err := f.svc.Awaiting("org-1")
	if err != nil {
		t.Fatal(err)
	}
	return list
}

// Un comentario del cliente deja el hilo esperando respuesta, y así sale en
// la lista y en el estado, con quién lo dijo y qué.
//
// Roturas que atrapa: no marcar el hilo; contar a un cliente como del equipo;
// no listarlo o no contarlo en el estado; perder quién lo dijo.
func TestAClientCommentLeavesTheThreadAwaiting(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	f.deliver(t, commentAt(9001, 8001, bcClient, "<div>¿Ya quedó el login?</div>", "2026-11-26T12:00:00Z"))

	th := f.thread(t, bcTodo)
	if !th.AwaitingReply || th.AwaitingSince == nil || th.LastExternalName != "Cliente" || !th.LastExternalClient {
		t.Fatalf("after a client comment the thread is %+v", th)
	}
	list := f.awaiting(t)
	if len(list) != 1 {
		t.Fatalf("awaiting: %+v", list)
	}
	a := list[0]
	if a.RecordingID != bcTodo || a.ProjectID != bcProject || a.ProjectName != "Proteus" || a.SpaceID != "sp-1" ||
		a.TaskID != "t-1" || a.TaskRef != "pr-1" || a.Title != "Arreglar el login" ||
		a.LastExternal.Name != "Cliente" || !a.LastExternal.Client || a.LastExternal.Excerpt != "¿Ya quedó el login?" {
		t.Fatalf("the awaiting row is %+v", a)
	}
	st, err := f.svc.Status("org-1")
	if err != nil || st.Awaiting != 1 {
		t.Fatalf("status awaiting: %+v %v", st, err)
	}
}

// Una respuesta de alguien del equipo lo limpia y apunta quién contestó; y en
// el evento de esa persona `answeredAfter` es verdad, en el de los demás no.
//
// Roturas que atrapa: no limpiar con la respuesta del equipo; no apuntar
// quién; `answeredAfter` para todos, o para nadie.
func TestATeamReplyClearsIt(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	f.linkTodo(t)
	f.deliver(t, commentAt(9001, 8001, bcClient, "<div>¿Ya quedó?</div>", "2026-11-26T12:00:00Z"))
	f.deliver(t, commentAt(9002, 8002, bcAna, "<div>Sí, ya está</div>", "2026-11-26T12:05:00Z"))

	th := f.thread(t, bcTodo)
	if th.AwaitingReply || th.LastTeamReplyByID != "u-ana" {
		t.Fatalf("after Ana's reply the thread is %+v", th)
	}
	if list := f.awaiting(t); len(list) != 0 {
		t.Fatalf("a thread the team answered is still awaiting: %+v", list)
	}
	you := func(user string) map[string]any {
		evs := bcEvents(t, j, user)
		if len(evs) != 2 {
			t.Fatalf("%s has %d events", user, len(evs))
		}
		return eventData(t, evs[1])["you"].(map[string]any)
	}
	if you("u-ana")["answeredAfter"] != true {
		t.Fatalf("Ana answered last and her event says %v", you("u-ana"))
	}
	if you("u-bob")["answeredAfter"] != false {
		t.Fatalf("Bob did not answer and his event says %v", you("u-bob"))
	}
}

// Una persona de Basecamp sin casar es de fuera, como un cliente: su
// comentario deja el hilo esperando y el evento sale marcado externo.
//
// Roturas que atrapa: contar como del equipo a quien no casó; no marcar el
// evento externo.
func TestAnUnmatchedPersonCountsAsExternal(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	f.deliver(t, commentAt(9001, 8001, bcNewcomer, "<div>Hola</div>", "2026-11-26T12:00:00Z"))
	if th := f.thread(t, bcTodo); !th.AwaitingReply {
		t.Fatalf("an unmatched person's comment left %+v", th)
	}
	evs := bcEvents(t, j, "u-bob")
	if len(evs) != 1 || !evs[0].External || eventData(t, evs[0])["team"] != false {
		t.Fatalf("the event of an unmatched person: %+v", evs)
	}

	// Y el del equipo no es externo.
	f.deliver(t, commentAt(9002, 8002, bcAna, "<div>Hola</div>", "2026-11-26T12:01:00Z"))
	evs = bcEvents(t, j, "u-bob")
	if len(evs) != 2 || evs[1].External {
		t.Fatalf("a team member's event came out external: %+v", evs)
	}

	// Un cliente sigue siendo de fuera aunque un admin lo haya casado con un
	// miembro.
	if _, err := f.svc.MatchPerson("org-1", bcClient.ID, "u-carla"); err != nil {
		t.Fatal(err)
	}
	f.deliver(t, commentAt(9003, 8003, bcClient, "<div>¿Y?</div>", "2026-11-26T12:02:00Z"))
	if th := f.thread(t, bcTodo); !th.AwaitingReply {
		t.Fatalf("a client married to a member counted as the team: %+v", th)
	}
}

// Basecamp no entrega en orden: lo que se dijo antes que lo último aplicado
// no cambia el hilo, en ninguno de los dos sentidos.
//
// Roturas que atrapa: aplicar cada entrega al llegar (sin mirar la hora); o
// comparar al revés.
func TestAnOlderDeliveryDoesNotReopenTheThread(t *testing.T) {
	f := newBasecampFixture(t)
	f.deliver(t, commentAt(9002, 8002, bcAna, "<div>Listo</div>", "2026-11-26T12:05:00Z"))
	f.deliver(t, commentAt(9001, 8001, bcClient, "<div>¿Y esto?</div>", "2026-11-26T12:00:00Z"))
	if th := f.thread(t, bcTodo); th.AwaitingReply {
		t.Fatalf("an older client comment delivered late reopened %+v", th)
	}

	f.deliver(t, commentAt(9003, 8003, bcClient, "<div>Otra cosa</div>", "2026-11-26T12:10:00Z"))
	f.deliver(t, commentAt(9004, 8004, bcAna, "<div>Viejo</div>", "2026-11-26T12:07:00Z"))
	th := f.thread(t, bcTodo)
	if !th.AwaitingReply || th.LastExternalExcerpt != "Otra cosa" {
		t.Fatalf("an older team reply delivered late cleared %+v", th)
	}
}

// El evento completo le llega a cada miembro de la org del proyecto (también
// a quien sólo mira) y a nadie más: ni a quien sólo es de otra org, ni a un
// superadmin que no es miembro. Y dice de qué va.
//
// Roturas que atrapa: mandarlo sólo a quien se avisa; mandarlo a otra org o a
// un superadmin de fuera; dejar fuera a un viewer; perder el texto, las
// menciones, la tarea o el hilo; `mine` sin la mención.
func TestTheFullEventReachesOnlyMembersOfThatOrg(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	f.addSecondOrg(t)
	vic := &domain.User{Username: "vic", Email: "vic@dwit.mx"}
	vic.ID = "u-vic"
	mustCreate(t, f.db, vic)
	mustCreate(t, f.db, &domain.OrgMembership{OrgID: "org-1", UserID: "u-vic", Role: domain.OrgRoleViewer})
	root := &domain.User{Username: "root", Email: "root@guz.dev", IsSuperadmin: true}
	root.ID = "u-root"
	mustCreate(t, f.db, root)
	f.linkTodo(t)

	f.deliver(t, commentAt(9001, 8001, bcClient, `<div>`+mentionHTML("sgid-bob", "Bob Bc")+` mira el **login**</div>`,
		"2026-11-26T12:00:00Z"))

	for _, u := range []string{"u-ana", "u-bob", "u-carla", "u-vic"} {
		if evs := bcEvents(t, j, u); len(evs) != 1 || evs[0].OrgID != "org-1" || evs[0].Key != "bc:comment_created:8001" {
			t.Fatalf("%s, a member of org-1, got %+v", u, evs)
		}
	}
	for _, u := range []string{"u-dan", "u-root"} {
		if evs := bcEvents(t, j, u); len(evs) != 0 {
			t.Fatalf("%s, who is not a member of org-1, got %+v", u, evs)
		}
	}

	bob := bcEvents(t, j, "u-bob")[0]
	ana := bcEvents(t, j, "u-ana")[0]
	if !bob.Mine || ana.Mine {
		t.Fatalf("mine: Bob (mentioned) %v, Ana %v", bob.Mine, ana.Mine)
	}
	d := eventData(t, bob)
	if d["kind"] != "comment_created" || !strings.Contains(d["text"].(string), "@Bob Bc mira el") {
		t.Fatalf("kind and text: %v %q", d["kind"], d["text"])
	}
	if ms := d["mentions"].([]any); len(ms) != 1 || ms[0].(map[string]any)["userId"] != "u-bob" {
		t.Fatalf("mentions: %v", d["mentions"])
	}
	if you := d["you"].(map[string]any); you["mentioned"] != true {
		t.Fatalf("Bob's you: %v", you)
	}
	if a := d["author"].(map[string]any); a["name"] != "Cliente" || a["client"] != true || a["userId"] != "" {
		t.Fatalf("author: %v", a)
	}
	if c := d["cac"].(map[string]any); c["taskId"] != "t-1" || c["taskRef"] != "pr-1" {
		t.Fatalf("cac: %v", c)
	}
	if p := d["project"].(map[string]any); p["name"] != "Proteus" || p["spaceId"] != "sp-1" {
		t.Fatalf("project: %v", p)
	}
	th := d["thread"].(map[string]any)
	if th["awaitingReply"] != true || th["recordingId"] != float64(bcTodo) {
		t.Fatalf("thread: %v", th)
	}
	if rec := d["recording"].(map[string]any); !strings.Contains(rec["appUrl"].(string), "#__recording_8001") {
		t.Fatalf("recording: %v", rec)
	}
}

// Un webhook que Basecamp reintenta, o el mismo comentario en otra entrega,
// es un solo evento en el diario de cada uno.
//
// Rotura que atrapa: apuntar el evento sin clave, o con una que cambia por
// entrega.
func TestARetriedWebhookIsOneEvent(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	body := commentAt(9001, 8001, bcClient, "<div>Hola</div>", "2026-11-26T12:00:00Z")
	f.deliver(t, body)
	f.deliver(t, body)
	f.deliver(t, commentAt(9002, 8001, bcClient, "<div>Hola</div>", "2026-11-26T12:00:00Z"))
	if evs := bcEvents(t, j, "u-ana"); len(evs) != 1 {
		t.Fatalf("one comment delivered three times left %d events", len(evs))
	}
}

// Si Basecamp no contesta al completar el evento, el webhook no falla y el
// evento sale igual, marcado `enriched: false`.
//
// Roturas que atrapa: que el fallo al completar tumbe el webhook; que el
// evento no salga; que diga que salió completo.
func TestBasecampBeingDownStillDeliversTheEvent(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	f.linkTodo(t)
	f.fake.mu.Lock()
	f.fake.recordingsDown = true
	f.fake.mu.Unlock()
	if err := f.svc.HandleWebhook(f.secret, commentAt(9001, 8001, bcClient, "<div>Hola</div>", "2026-11-26T12:00:00Z")); err != nil {
		t.Fatalf("Basecamp being down failed the webhook: %v", err)
	}
	evs := bcEvents(t, j, "u-ana")
	if len(evs) != 1 || eventData(t, evs[0])["enriched"] != false {
		t.Fatalf("with Basecamp down: %+v", evs)
	}
	if len(f.comments(t, "t-1")) != 1 {
		t.Fatal("the comment did not land in the task")
	}

	// Y con Basecamp de vuelta, completo: el hilo trae sus últimos comentarios
	// y la lista del to-do.
	f.fake.mu.Lock()
	f.fake.recordingsDown = false
	f.fake.recordings[fmt.Sprintf("recordings/%d/comments", bcTodo)] = `[
		{"id":8001,"created_at":"2026-11-26T12:00:00Z","content":"<div>Hola</div>","creator":{"id":200,"name":"Cliente","client":true}},
		{"id":8002,"created_at":"2026-11-26T12:05:00Z","content":"<div>Va</div>","creator":{"id":101,"name":"Ana Bc","email_address":"ana@dwit.mx"}}]`
	f.fake.recordings[fmt.Sprintf("todos/%d", bcTodo)] = fmt.Sprintf(`{"id":%d,"title":"Arreglar el login","due_on":"2026-12-01",
		"completed":false,"assignees":[{"id":101,"name":"Ana Bc","email_address":"ana@dwit.mx"}],
		"parent":{"id":77,"title":"Sprint 3","type":"Todolist","app_url":"https://3.basecamp.com/999/buckets/42/todolists/77"},
		"bucket":{"id":%d}}`, bcTodo, bcProject)
	f.fake.mu.Unlock()
	f.deliver(t, commentAt(9002, 8002, bcAna, "<div>Va</div>", "2026-11-26T12:05:00Z"))
	evs = bcEvents(t, j, "u-ana")
	d := eventData(t, evs[1])
	if d["enriched"] != true || len(d["thread"].(map[string]any)["recent"].([]any)) != 2 {
		t.Fatalf("with Basecamp back: enriched %v, thread %v", d["enriched"], d["thread"])
	}
	todo := d["todo"].(map[string]any)
	if todo["dueOn"] != "2026-12-01" || d["list"].(map[string]any)["title"] != "Sprint 3" {
		t.Fatalf("todo %v list %v", todo, d["list"])
	}
	if !evs[1].Mine || d["you"].(map[string]any)["assigned"] != true {
		t.Fatalf("Ana is assigned to the to-do and her event says mine=%v you=%v", evs[1].Mine, d["you"])
	}
}

// Si Basecamp se queda colgado al completar, se le espera el plazo y no más:
// el webhook contesta y el evento sale sin completar.
//
// Rotura que atrapa: pedir sin plazo (el webhook se queda colgado). El
// cliente HTTP de esta prueba no tiene plazo propio, así que sólo el del
// evento lo corta; el reloj de abajo no mide nada fino, sólo detecta el
// cuelgue.
func TestAHangingBasecampDoesNotHangTheWebhook(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	f.svc.WithEnrichTimeout(50 * time.Millisecond)
	f.svc.http = &http.Client{}
	hang := make(chan struct{})
	defer close(hang)
	f.fake.mu.Lock()
	f.fake.recordingsHang = hang
	f.fake.mu.Unlock()

	done := make(chan error, 1)
	go func() {
		done <- f.svc.HandleWebhook(f.secret, commentAt(9001, 8001, bcClient, "<div>Hola</div>", "2026-11-26T12:00:00Z"))
	}()
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("a hanging Basecamp failed the webhook: %v", err)
		}
	case <-time.After(30 * time.Second):
		t.Fatal("a hanging Basecamp hung the webhook")
	}
	if evs := bcEvents(t, j, "u-ana"); len(evs) != 1 || eventData(t, evs[0])["enriched"] != false {
		t.Fatalf("with Basecamp hanging: %+v", evs)
	}
}

// Sólo viajan como adjuntos las direcciones de Basecamp: lo que llega no va
// firmado, y un enlace a otro sitio no se hace pasar por un fichero.
//
// Rotura que atrapa: no pasar la dirección por SafeBasecampURL.
func TestOnlyBasecampURLsTravelAsAttachments(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	content := `<div>Mira<bc-attachment sgid="a1" content-type="image/png" filename="captura.png" ` +
		`href="https://storage.3.basecamp.com/999/blobs/1/download/captura.png"></bc-attachment>` +
		`<bc-attachment sgid="a2" content-type="application/pdf" filename="malo.pdf" href="https://evil.example/malo.pdf"></bc-attachment>` +
		`<bc-attachment sgid="a3" content-type="text/html" filename="x.html" href="javascript:alert(1)"></bc-attachment></div>`
	f.deliver(t, commentAt(9001, 8001, bcClient, content, "2026-11-26T12:00:00Z"))
	d := eventData(t, bcEvents(t, j, "u-ana")[0])
	atts := d["attachments"].([]any)
	if len(atts) != 1 {
		t.Fatalf("attachments: %v", atts)
	}
	a := atts[0].(map[string]any)
	if a["name"] != "captura.png" || a["contentType"] != "image/png" ||
		a["url"] != "https://storage.3.basecamp.com/999/blobs/1/download/captura.png" {
		t.Fatalf("the attachment is %v", a)
	}
}

// El evento va al diario **antes** que cualquier aviso, y cada aviso lleva
// su clave: quien escucha puede casar el aviso con el evento.
//
// Roturas que atrapa: publicar el evento después de avisar; avisos sin
// SourceKey, o con otra clave.
func TestTheEventComesBeforeItsNoticesAndTheyShareTheKey(t *testing.T) {
	f := newBasecampFixture(t)
	j := f.withJournal(t)
	f.linkTodo(t)
	seen := map[string]int{}
	f.svc.inbox = notifierFunc(func(a domain.Aviso) {
		seen[a.UserID] = len(bcEvents(t, j, a.UserID))
		f.inbox.Notify(a)
	})
	f.deliver(t, commentAt(9001, 8001, bcClient, `<div>`+mentionHTML("sgid-carla", "Carla Bc")+` hola</div>`,
		"2026-11-26T12:00:00Z"))
	notices := f.inbox.list
	if len(notices) < 2 {
		t.Fatalf("expected notices to the mentioned and the assignees, got %+v", notices)
	}
	for _, a := range notices {
		if seen[a.UserID] != 1 {
			t.Fatalf("%s was told before the event was in their journal", a.UserID)
		}
		if a.SourceKey != "bc:comment_created:8001" {
			t.Fatalf("the notice to %s carries SourceKey %q", a.UserID, a.SourceKey)
		}
	}

	// Y lo mismo con un to-do que se vuelve tarea.
	f.inbox.reset()
	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcBob)
	f.deliver(t, todoEvent(9100, domain.BasecampTodoCreated, bcNewTodo, bcAna))
	assigned := noticesOfKind(f.inbox.of("u-bob"), "task:assigned")
	if len(assigned) != 1 || assigned[0].SourceKey != fmt.Sprintf("bc:todo_created:%d", bcNewTodo) || seen["u-bob"] != 2 {
		t.Fatalf("the to-do's notice: %+v (events before it: %d)", assigned, seen["u-bob"])
	}
	evs := bcEvents(t, j, "u-bob")
	if !evs[1].Mine || eventData(t, evs[1])["you"].(map[string]any)["assigned"] != true {
		t.Fatalf("Bob is assigned to the new to-do and his event says %+v", evs[1])
	}
}

type notifierFunc func(domain.Aviso)

func (n notifierFunc) Notify(a domain.Aviso) { n(a) }
