package service

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"testing"
	"time"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

type requestInbox struct {
	mu   sync.Mutex
	sent []domain.Aviso
}

func (i *requestInbox) Notify(a domain.Aviso) {
	i.mu.Lock()
	defer i.mu.Unlock()
	i.sent = append(i.sent, a)
}

type requestFixture struct {
	db    *gorm.DB
	svc   *RequestService
	hub   *events.Hub
	inbox *requestInbox
	tasks *repository.TaskRepository
}

func newRequestFixture(t *testing.T) *requestFixture {
	t.Helper()
	if repository.GetEnv("DB_HOST", "") == "" {
		t.Skip("no database configured")
	}
	dsn := func(name string) string {
		return fmt.Sprintf("host=%s port=%s user=%s password=%s dbname=%s sslmode=%s",
			repository.GetEnv("DB_HOST", "localhost"), repository.GetEnv("DB_PORT", "5432"),
			repository.GetEnv("DB_USER", "postgres"), repository.GetEnv("DB_PASSWORD", ""),
			name, repository.GetEnv("DB_SSLMODE", "disable"))
	}
	admin, err := gorm.Open(postgres.Open(dsn(repository.GetEnv("DB_NAME", "cac"))), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatalf("DB_HOST is set but the database does not answer (a skipped test is not a green one): %v", err)
	}
	name := fmt.Sprintf("cac_test_requests_%d", time.Now().UnixNano()%1_000_000)
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	adminSQL, _ := admin.DB()
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		adminSQL.Close()
	})
	if err := db.AutoMigrate(
		&domain.Organization{}, &domain.TaskSpace{}, &domain.TaskList{}, &domain.Item{}, &domain.User{},
		&domain.OrgMembership{}, &domain.ReportProject{}, &domain.Request{}, &domain.RequestVersion{},
		// Lo que leen el detalle de la tarea, el tablero y el mapa de contexto.
		&domain.ItemAssignee{}, &domain.ItemBlock{}, &domain.ItemWatcher{}, &domain.TaskTag{}, &domain.TaskTagLink{},
		&domain.ItemComment{}, &domain.ItemAttachment{}, &domain.TaskGitLink{},
		&domain.TaskFolder{}, &domain.Doc{}, &domain.DocTab{}, &domain.DocPage{}, &domain.Decision{},
		&domain.GitHubRepo{}, &domain.RepoDoc{},
	); err != nil {
		t.Fatal(err)
	}
	sp := &domain.TaskSpace{OrgID: "org-1", Name: "Command and control", Key: "cac", Rank: "0.5"}
	sp.ID = "space-1"
	li := &domain.TaskList{SpaceID: "space-1", Name: "App", Rank: "0.5"}
	li.ID = "list-1"
	rows := []any{sp, li}
	for _, u := range []struct{ id, user, name string }{{"u-ana", "ana", "Ana Ruiz"}, {"u-bea", "bea", "Bea Gil"}, {"u-out", "out", "Fuera"}} {
		usr := &domain.User{Username: u.user, Name: u.name, Password: "x"}
		usr.ID = u.id
		rows = append(rows, usr)
	}
	rows = append(rows,
		&domain.OrgMembership{OrgID: "org-1", UserID: "u-ana", Role: domain.OrgRoleMember},
		&domain.OrgMembership{OrgID: "org-1", UserID: "u-bea", Role: domain.OrgRoleMember},
	)
	for i, p := range []domain.ItemPriority{domain.ItemPriorityLow, domain.ItemPriorityUrgent, domain.ItemPriorityMedium} {
		it := &domain.Item{OrgID: "org-1", SpaceID: "space-1", ListID: "list-1", Seq: 10 + i,
			Title: fmt.Sprintf("Tarea %d", i), Priority: p, Status: domain.ReportInProgress,
			Handoff: "Falta probar el panel."}
		it.ID = fmt.Sprintf("it-%d", i)
		rows = append(rows, it)
	}
	for _, r := range rows {
		if err := db.Create(r).Error; err != nil {
			t.Fatal(err)
		}
	}
	f := &requestFixture{db: db, hub: events.NewHub(), inbox: &requestInbox{}, tasks: repository.NewTaskRepository(db)}
	f.svc = NewRequestService(repository.NewRequestRepository(db), f.tasks, repository.NewDocRepository(db), f.hub, f.inbox)
	return f
}

func (f *requestFixture) create(t *testing.T, task, by, of, via string, kind domain.RequestKind, body string) *domain.RequestView {
	t.Helper()
	it, err := f.svc.ResolveTask([]string{"org-1"}, task)
	if err != nil {
		t.Fatal(err)
	}
	v, err := f.svc.Create(it, by, via, domain.CreateRequestRequest{
		TaskID: task, Kind: kind, Summary: "Mira esto", Body: json.RawMessage(body), RequestedOf: of,
	})
	if err != nil {
		t.Fatal(err)
	}
	return v
}

func (f *requestFixture) find(t *testing.T, code string) *domain.Request {
	t.Helper()
	q, err := f.svc.Find(code)
	if err != nil {
		t.Fatal(err)
	}
	return q
}

const testBody = `{"url":"/tasks","steps":["abrir el panel"],"expected":"entrada 07:30"}`

// Un pedido se deja por la ref de la tarea, a una persona por su nombre, le
// avisa con enlace directo a él, y sólo ella lo contesta, una vez; la
// respuesta dice si llegó por el botón o desde una sesión.
//
// Roturas que atrapa: que conteste otro (en «Equipo» se ven los ajenos); dos
// respuestas que se pisan (el botón y la terminal a la vez); no avisar, avisar
// a quien pidió o con un enlace que no abre el pedido; perder por dónde llegó
// la respuesta; aceptar a alguien de fuera de la organización.
func TestOnlyThePersonAnswersARequestAndOnlyOnce(t *testing.T) {
	f := newRequestFixture(t)
	v := f.create(t, "cac-10", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestTest, testBody)
	if v.RequestedOf.ID != "u-bea" || v.Task.Ref != "cac-10" || v.Status != domain.RequestOpen || v.CanAnswer {
		t.Fatalf("pedido mal hecho: %+v", v)
	}
	if len(f.inbox.sent) != 1 || f.inbox.sent[0].UserID != "u-bea" || f.inbox.sent[0].Link != "/requests?open="+v.Code {
		t.Fatalf("el aviso va a Bea y abre el pedido: %+v", f.inbox.sent)
	}
	it, _ := f.svc.ResolveTask([]string{"org-1"}, "it-0")
	if _, err := f.svc.Create(it, "u-ana", "", domain.CreateRequestRequest{Kind: domain.RequestTest, Summary: "x",
		Body: json.RawMessage(testBody), RequestedOf: "out"}); err == nil {
		t.Fatal("a alguien de fuera de la organización no se le pide nada")
	}

	no := false
	q := f.find(t, v.Code)
	if _, err := f.svc.Answer(q, "u-ana", false, domain.RequestAnswer{Works: &no, Note: "no"}); !errors.Is(err, ErrRequestNotYours) {
		t.Fatalf("Ana no contesta por Bea: %v", err)
	}
	got, err := f.svc.Answer(q, "u-bea", true, domain.RequestAnswer{Works: &no, Note: "sale 07:00"})
	if err != nil {
		t.Fatal(err)
	}
	if got.Status != domain.RequestAnswered || got.AnsweredVia != domain.AnsweredViaSession || got.AnsweredBy == nil ||
		got.AnsweredBy.Name != "Bea Gil" || got.Answer == nil || got.Answer.Note != "sale 07:00" {
		t.Fatalf("respuesta mal guardada: %+v", got)
	}
	if _, err := f.svc.Answer(q, "u-bea", false, domain.RequestAnswer{Works: &no, Note: "otra"}); !errors.Is(err, repository.ErrRequestNotOpen) {
		t.Fatalf("una segunda respuesta no pisa la primera: %v", err)
	}
}

// Rehacer un pedido deja la versión anterior en el historial, y el detalle
// trae ese historial y el handoff de la tarea: lo que una sesión necesita
// para seguir desde la terminal.
//
// Roturas que atrapa: pisar sin guardar lo de antes; no subir la versión; un
// detalle sin historial o sin handoff; rehacer uno ya contestado.
func TestReissuingARequestKeepsWhatItSaidBefore(t *testing.T) {
	f := newRequestFixture(t)
	v := f.create(t, "it-1", "u-ana", "", domain.ViaMCP, domain.RequestTest, testBody)
	summary := "Ahora con la salida"
	got, err := f.svc.Update(f.find(t, v.Code), "u-ana", domain.ViaMCP, domain.UpdateRequestRequest{
		Summary: &summary, Body: json.RawMessage(`{"steps":["abrir"],"expected":"salida 18:39"}`),
	})
	if err != nil {
		t.Fatal(err)
	}
	if got.Version != 2 || got.Summary != summary || got.Body.Expected != "salida 18:39" {
		t.Fatalf("la versión nueva: %+v", got)
	}
	if len(got.History) != 1 || got.History[0].Version != 1 || got.History[0].Summary != "Mira esto" ||
		got.History[0].Body.Expected != "entrada 07:30" || got.History[0].By.Name != "Ana Ruiz" {
		t.Fatalf("la anterior tiene que quedar en el historial: %+v", got.History)
	}
	if got.Handoff == nil || got.Handoff.Text != "Falta probar el panel." {
		t.Fatalf("el detalle trae el handoff de la tarea: %+v", got.Handoff)
	}
	yes := true
	if _, err := f.svc.Answer(f.find(t, v.Code), "u-ana", false, domain.RequestAnswer{Works: &yes}); err != nil {
		t.Fatal(err)
	}
	if _, err := f.svc.Update(f.find(t, v.Code), "u-ana", "", domain.UpdateRequestRequest{Summary: &summary}); !errors.Is(err, repository.ErrRequestNotOpen) {
		t.Fatalf("uno contestado no se rehace: %v", err)
	}
}

// La bandeja: lo pedido a quien mira, por urgencia de la tarea (con la media
// guardada como `medium`), y la tarea dice a quién espera mientras tenga
// pedidos abiertos.
//
// Roturas que atrapa: ordenar por fecha o por el nombre de la API (`normal`)
// de la prioridad; nombrar dos veces a quien tiene dos pedidos; colar los pedidos de otro en «míos»; una tarea que sigue
// diciendo «espera a…» con el pedido ya contestado.
func TestTheInboxPutsTheUrgentFirstAndTasksSayWhoTheyWaitOn(t *testing.T) {
	f := newRequestFixture(t)
	low := f.create(t, "it-0", "u-ana", "", domain.ViaMCP, domain.RequestTest, testBody)
	urgent := f.create(t, "it-1", "u-ana", "", domain.ViaMCP, domain.RequestTest, testBody)
	medium := f.create(t, "it-2", "u-ana", "", domain.ViaMCP, domain.RequestTest, testBody)
	f.create(t, "it-1", "u-ana", "bea", domain.ViaMCP, domain.RequestTest, testBody)
	again := f.create(t, "it-1", "u-ana", "", domain.ViaMCP, domain.RequestTest, testBody)

	mine, err := f.svc.List(repository.RequestFilter{OrgIDs: []string{"org-1"}, RequestedOf: "u-ana",
		Statuses: []domain.RequestStatus{domain.RequestOpen}}, "u-ana")
	if err != nil {
		t.Fatal(err)
	}
	var order []string
	for _, v := range mine {
		order = append(order, v.Code)
	}
	if fmt.Sprint(order) != fmt.Sprint([]string{urgent.Code, again.Code, medium.Code, low.Code}) {
		t.Fatalf("urgente (la más vieja primero), media, baja: %v", order)
	}
	team, _ := f.svc.List(repository.RequestFilter{OrgIDs: []string{"org-1"}, Statuses: []domain.RequestStatus{domain.RequestOpen}}, "u-ana")
	if len(team) != 5 {
		t.Fatalf("en «Equipo» están los cinco: %d", len(team))
	}

	marks := OpenRequestsOfTasks(f.tasks.RequestRepo(), []string{"it-1", "it-0"})
	if m := marks["it-1"]; m == nil || m.Count != 3 || fmt.Sprint(m.WaitingOn) != "[Ana Ruiz Bea Gil]" {
		t.Fatalf("it-1 espera a Ana y a Bea: %+v", m)
	}
	yes := true
	if _, err := f.svc.Answer(f.find(t, low.Code), "u-ana", false, domain.RequestAnswer{Works: &yes}); err != nil {
		t.Fatal(err)
	}
	if m := OpenRequestsOfTasks(f.tasks.RequestRepo(), []string{"it-0"})["it-0"]; m != nil {
		t.Fatalf("contestado, la tarea ya no espera: %+v", m)
	}
	if my := f.svc.MyOpen([]string{"org-1"}, "u-ana", 1); my.Count != 3 || len(my.Top) != 1 || my.Top[0].Code != urgent.Code || my.Top[0].TaskRef != "cac-11" {
		t.Fatalf("la sesión que arranca ve cuántos y el primero: %+v", my)
	}
}

// Pedirle a la app que abra un pedido llega sólo a la persona a quien se le
// pidió, y `delivered` es que una app dijo «lo abrí»: sin ninguna, falso.
//
// Roturas que atrapa: el evento a la organización entera (lo pintaría la app
// de otro); abrirlo en la app de otra persona; decir entregado sin que nadie
// lo enseñara; no reconocer el «lo abrí» de la app.
func TestOpeningARequestReachesOnlyItsPersonAndSaysIfAnAppShowedIt(t *testing.T) {
	f := newRequestFixture(t)
	v := f.create(t, "it-0", "u-ana", "bea", domain.ViaMCP, domain.RequestTest, testBody)
	q := f.find(t, v.Code)

	if _, err := f.svc.Open(context.Background(), q, "u-ana"); !errors.Is(err, ErrRequestNotYours) {
		t.Fatalf("Ana no abre el pedido de Bea en la app de Bea: %v", err)
	}

	bea, stopBea := f.hub.Subscribe("u-bea", []string{"org-1"})
	defer stopBea()
	ana, stopAna := f.hub.Subscribe("u-ana", []string{"org-1"})
	defer stopAna()

	done := make(chan domain.RequestOpened, 1)
	go func() {
		out, err := f.svc.Open(context.Background(), q, "u-bea")
		if err != nil {
			t.Error(err)
		}
		done <- out
	}()
	// La app de Bea recibe el evento y contesta con su nonce.
	deadline := time.After(10 * time.Second)
	for got := false; !got; {
		select {
		case ev := <-bea:
			if ev.Type == domain.RequestOpenEvent {
				if eventField(ev.Data, "code") != v.Code {
					t.Fatalf("abre otro pedido: %+v", ev.Data)
				}
				f.svc.Opened("u-bea", "org-1", eventField(ev.Data, "nonce"))
				got = true
			}
		case <-deadline:
			t.Fatal("la app de Bea no recibió el evento")
		}
	}
	select {
	case out := <-done:
		if !out.Delivered || out.Link != "/requests?open="+v.Code {
			t.Fatalf("la app dijo que lo abrió: %+v", out)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("Open no se enteró del «lo abrí»")
	}
	for drained := false; !drained; {
		select {
		case ev := <-ana:
			if ev.Type == domain.RequestOpenEvent || ev.Type == domain.RequestOpenedEvent {
				t.Fatalf("a Ana no le llega abrir el pedido de Bea: %s", ev.Type)
			}
		default:
			drained = true
		}
	}

	// Sin ninguna app que conteste: no entregado. El plazo es el del propio
	// servicio, acortado; no se cronometra nada.
	f.svc.openAck = 20 * time.Millisecond
	stopBea()
	out, err := f.svc.Open(context.Background(), q, "u-bea")
	if err != nil || out.Delivered {
		t.Fatalf("sin app no se puede decir entregado: %+v %v", out, err)
	}
}

// Esperar la respuesta vuelve en cuanto llega, no al acabar el plazo, y con
// la respuesta dentro; y lo mismo si se retira.
//
// Roturas que atrapa: esperar el plazo entero (el agente se quedaría colgado
// diez minutos con la respuesta ya dada); despertar con el pedido de otro;
// devolver el pedido sin la respuesta.
func TestWaitingWakesUpWithTheAnswer(t *testing.T) {
	f := newRequestFixture(t)
	f.svc.waitCap = time.Hour
	v := f.create(t, "it-0", "u-ana", "", domain.ViaMCP, domain.RequestDecide,
		`{"options":[{"label":"Sí","consequence":"x"},{"label":"No","consequence":"y"}]}`)
	other := f.create(t, "it-1", "u-ana", "", domain.ViaMCP, domain.RequestObtain, `{"what":"llave","from":"Bea"}`)

	listening := make(chan struct{}, 8)
	f.svc.afterCheck = func() { listening <- struct{}{} }
	done := make(chan domain.RequestWait, 1)
	go func() {
		out, err := f.svc.Wait(context.Background(), f.find(t, v.Code), "u-ana", time.Hour)
		if err != nil {
			t.Error(err)
		}
		done <- out
	}()
	<-listening
	// Contestar otro pedido no la despierta: vuelve a escuchar.
	if _, err := f.svc.Answer(f.find(t, other.Code), "u-ana", false, domain.RequestAnswer{}); err != nil {
		t.Fatal(err)
	}
	select {
	case <-listening:
	case out := <-done:
		t.Fatalf("despertó con el pedido de otro: %+v", out)
	}
	if _, err := f.svc.Answer(f.find(t, v.Code), "u-ana", false, domain.RequestAnswer{Option: "No", Note: "mejor no"}); err != nil {
		t.Fatal(err)
	}
	select {
	case out := <-done:
		if !out.Settled || out.Request == nil || out.Request.Code != v.Code || out.Request.Answer == nil || out.Request.Answer.Option != "No" {
			t.Fatalf("vuelve con la respuesta: %+v", out)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("la espera no despertó con la respuesta")
	}

	// Retirado también despierta.
	third := f.create(t, "it-2", "u-ana", "", domain.ViaMCP, domain.RequestObtain, `{"what":"llave","from":"Bea"}`)
	listening = make(chan struct{}, 8)
	f.svc.afterCheck = func() { listening <- struct{}{} }
	done = make(chan domain.RequestWait, 1)
	go func() {
		out, _ := f.svc.Wait(context.Background(), f.find(t, third.Code), "u-ana", time.Hour)
		done <- out
	}()
	<-listening
	if _, err := f.svc.Withdraw(f.find(t, third.Code), "u-ana"); err != nil {
		t.Fatal(err)
	}
	select {
	case out := <-done:
		if !out.Settled || out.Request.Status != domain.RequestWithdrawn {
			t.Fatalf("retirado: %+v", out)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("la espera no despertó al retirarlo")
	}
}

// La tarea enseña a quién espera allí donde se lee: el detalle (get_task), la
// tarjeta del tablero y las tareas en curso de get_context.
//
// Roturas que atrapa: calcular la marca y no ponerla en alguno de los tres;
// ponerla en la tarea equivocada.
func TestATaskSaysWhoItWaitsOnWhereverItIsRead(t *testing.T) {
	f := newRequestFixture(t)
	f.create(t, "it-1", "u-ana", "bea", domain.ViaMCP, domain.RequestTest, testBody)
	tasks := NewTaskService(f.tasks, repository.NewReportRepository(f.db), repository.NewOrganizationRepository(f.db), nil)

	d, err := tasks.Detail("it-1")
	if err != nil {
		t.Fatal(err)
	}
	if d.OpenRequests == nil || d.OpenRequests.Count != 1 || fmt.Sprint(d.OpenRequests.WaitingOn) != "[Bea Gil]" {
		t.Fatalf("detalle: %+v", d.OpenRequests)
	}
	if d, _ := tasks.Detail("it-0"); d == nil || d.OpenRequests != nil {
		t.Fatalf("it-0 no espera a nadie: %+v", d)
	}
	b, err := tasks.Board("list-1")
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range b.Tasks {
		want := ""
		if c.ID == "it-1" {
			want = "[Bea Gil]"
		}
		if got := fmt.Sprint(c.WaitingOn); (want == "" && len(c.WaitingOn) > 0) || (want != "" && got != want) {
			t.Fatalf("tarjeta %s: %v", c.ID, c.WaitingOn)
		}
	}
	m, err := NewContextService(f.db).MapWith("org-1", "space-1", nil, domain.ContextOptions{Only: []string{"inProgress"}})
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, c := range m.InProgress {
		if c.ID == "it-1" {
			found = c.OpenRequests != nil && c.OpenRequests.Count == 1
		} else if c.OpenRequests != nil {
			t.Fatalf("%s no espera a nadie: %+v", c.ID, c.OpenRequests)
		}
	}
	if !found {
		t.Fatalf("get_context: %+v", m.InProgress)
	}
}
