package service

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// La fase 3 de Basecamp (command-and-control-557): un to-do nuevo o asignado
// es una tarea enlazada. Contra el Basecamp de mentira de basecamp_test.go.

const bcNewTodo = int64(6001)

var (
	bcBob   = domain.BasecampPersonWire{ID: 102, Name: "Bob Bc", EmailAddress: "bob@dwit.mx", AttachableSGID: "sgid-bob"}
	bcCarla = domain.BasecampPersonWire{ID: 103, Name: "Carla Bc", EmailAddress: "carla@dwit.mx", AttachableSGID: "sgid-carla"}
)

func todoURL(id int64) string {
	return fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/todos/%d", bcAccount, bcProject, id)
}

// setTodo: cómo contesta la API por el to-do `id`, con sus asignados.
func (f *bcFixture) setTodo(id int64, title, notes string, assignees ...domain.BasecampPersonWire) {
	if assignees == nil {
		assignees = []domain.BasecampPersonWire{}
	}
	body, _ := json.Marshal(map[string]any{
		"id": id, "type": "Todo", "title": title, "content": title, "description": notes,
		"app_url": todoURL(id), "completed": false, "assignees": assignees,
		"bucket": map[string]any{"id": bcProject, "name": "Proteus", "type": "Project"},
	})
	f.fake.mu.Lock()
	f.fake.recordings[fmt.Sprintf("todos/%d", id)] = string(body)
	f.fake.mu.Unlock()
}

// todoEvent: un evento del to-do `id` (`todo_created`, `todo_assignment_changed`…).
func todoEvent(eventID int64, kind string, id int64, who domain.BasecampPersonWire) []byte {
	ev := map[string]any{
		"id": eventID, "kind": kind, "created_at": "2026-11-27T10:00:00Z",
		"recording": map[string]any{
			"id": id, "type": "Todo", "title": "Revisar el checkout", "content": "Revisar el checkout",
			"app_url": todoURL(id),
			"bucket":  map[string]any{"id": bcProject, "name": "Proteus", "type": "Project"},
		},
		"creator": who,
		"details": map[string]any{"added_person_ids": []int64{}, "removed_person_ids": []int64{}},
	}
	raw, _ := json.Marshal(ev)
	return raw
}

func (f *bcFixture) deliver(t *testing.T, body []byte) {
	t.Helper()
	if err := f.svc.HandleWebhook(f.secret, body); err != nil {
		t.Fatalf("webhook: %v", err)
	}
}

// taskOf: la tarea enlazada al recording, o nil.
func (f *bcFixture) taskOf(t *testing.T, recording int64) *domain.Item {
	t.Helper()
	var l domain.BasecampLink
	if err := f.db.First(&l, "org_id = ? AND recording_id = ?", "org-1", recording).Error; err != nil {
		return nil
	}
	var it domain.Item
	if err := f.db.First(&it, "id = ?", l.ItemID).Error; err != nil {
		t.Fatal(err)
	}
	return &it
}

func (f *bcFixture) assignees(t *testing.T, itemID string) []string {
	t.Helper()
	ids, err := repository.NewBasecampRepository(f.db).Assignees(itemID)
	if err != nil {
		t.Fatal(err)
	}
	return ids
}

func (f *bcFixture) countItems(t *testing.T) int64 {
	t.Helper()
	var n int64
	f.db.Model(&domain.Item{}).Count(&n)
	return n
}

func noticesOfKind(notices []domain.Aviso, kind string) []domain.Aviso {
	var out []domain.Aviso
	for _, a := range notices {
		if a.Kind == kind {
			out = append(out, a)
		}
	}
	return out
}

// El criterio de aceptación: un to-do creado y asignado a un miembro aparece
// como tarea interna enlazada en el espacio del proyecto, con esa persona de
// responsable y avisada; y un comentario después cae en esa tarea, no como
// aviso suelto.
//
// Roturas que atrapa: no crear la tarea; crearla sin enlace (el comentario
// cae suelto); crearla visible para el cliente; no copiar las notas o la URL;
// no poner al asignado o no avisarle; avisar a quien creó el to-do, o a quien
// se asignó a sí misma.
func TestANewAssignedTodoBecomesALinkedTask(t *testing.T) {
	f := newBasecampFixture(t)
	f.setTodo(bcNewTodo, "Revisar el checkout", "<div>Falla con <strong>tarjetas</strong> de débito</div>", bcBob)
	f.deliver(t, todoEvent(200, domain.BasecampTodoCreated, bcNewTodo, bcAna))

	task := f.taskOf(t, bcNewTodo)
	if task == nil {
		t.Fatal("a new to-do made no linked task")
	}
	if task.OrgID != "org-1" || task.SpaceID != "sp-1" || task.ListID != "l-1" || task.Title != "Revisar el checkout" {
		t.Fatalf("the task is %+v", task)
	}
	if task.Visibility != domain.VisibilityInternal || task.Status != domain.ReportPending || task.ParentID != nil {
		t.Fatalf("the task is not an internal open task: %s %s", task.Visibility, task.Status)
	}
	if !strings.Contains(task.Description, "Falla con tarjetas de débito") || strings.Contains(task.Description, "<strong>") ||
		!strings.Contains(task.Description, "**Basecamp:** "+todoURL(bcNewTodo)) {
		t.Fatalf("the description is %q", task.Description)
	}
	if got := f.assignees(t, task.ID); len(got) != 1 || got[0] != "u-bob" {
		t.Fatalf("assignees %v, want Bob", got)
	}
	bob := noticesOfKind(f.inbox.of("u-bob"), "task:assigned")
	if len(bob) != 1 || bob[0].Link != "/tasks?task="+task.ID {
		t.Fatalf("Bob was told %+v", f.inbox.of("u-bob"))
	}
	if ana := f.inbox.of("u-ana"); len(ana) != 0 {
		t.Fatalf("Ana created the to-do herself and was told %v", kinds(ana))
	}
	// Quien se asigna a sí misma no recibe aviso de ello.
	f.setTodo(6005, "Lo hago yo", "", bcAna)
	f.deliver(t, todoEvent(205, domain.BasecampTodoCreated, 6005, bcAna))
	if got := f.assignees(t, f.taskOf(t, 6005).ID); strings.Join(got, ",") != "u-ana" {
		t.Fatalf("assignees %v", got)
	}
	if ana := f.inbox.of("u-ana"); len(ana) != 0 {
		t.Fatalf("Ana assigned herself and was told %v", kinds(ana))
	}

	// Un comentario después cae en la tarea y avisa a su responsable.
	f.inbox.reset()
	comment := []byte(strings.ReplaceAll(string(commentEvent(201, 9201, bcClient, "<div>¿Para cuándo?</div>")),
		fmt.Sprint(bcTodo), fmt.Sprint(bcNewTodo)))
	f.deliver(t, comment)
	if cs := f.comments(t, task.ID); len(cs) != 1 || !strings.Contains(cs[0].Body, "¿Para cuándo?") {
		t.Fatalf("the comment did not land in the task: %+v", cs)
	}
	if bob := f.inbox.of("u-bob"); len(bob) != 1 || bob[0].Kind != "task:comment" {
		t.Fatalf("Bob got %v", kinds(bob))
	}
	if ana := f.inbox.of("u-ana"); len(ana) != 0 {
		t.Fatalf("the comment was also a loose notice to whoever linked the project: %v", kinds(ana))
	}
}

// El mismo to-do nunca hace dos tareas: ni el mismo evento dos veces, ni el
// alta y la asignación (en cualquier orden, o a la vez), ni un to-do que ya
// estaba enlazado a mano.
//
// Roturas que atrapa: crear una tarea por evento; no mirar el enlace que ya
// había; dejar que dos entregas a la vez creen dos tareas.
func TestTheSameTodoNeverMakesTwoTasks(t *testing.T) {
	f := newBasecampFixture(t)
	before := f.countItems(t)
	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcBob)
	// La asignación llega antes que el alta.
	f.deliver(t, todoEvent(210, domain.BasecampTodoAssignment, bcNewTodo, bcAna))
	f.deliver(t, todoEvent(211, domain.BasecampTodoCreated, bcNewTodo, bcAna))
	f.deliver(t, todoEvent(211, domain.BasecampTodoCreated, bcNewTodo, bcAna))
	if n := f.countItems(t) - before; n != 1 {
		t.Fatalf("one to-do made %d tasks", n)
	}
	if bob := noticesOfKind(f.inbox.of("u-bob"), "task:assigned"); len(bob) != 1 {
		t.Fatalf("Bob was told %d times he has the task", len(bob))
	}

	// A la vez: varias entregas distintas del mismo to-do nuevo.
	const racing = int64(6002)
	f.setTodo(racing, "Otro", "", bcBob)
	var wg sync.WaitGroup
	for i := int64(0); i < 6; i++ {
		wg.Add(1)
		go func(i int64) {
			defer wg.Done()
			kind := domain.BasecampTodoCreated
			if i%2 == 1 {
				kind = domain.BasecampTodoAssignment
			}
			if err := f.svc.HandleWebhook(f.secret, todoEvent(220+i, kind, racing, bcAna)); err != nil {
				t.Errorf("delivery %d: %v", i, err)
			}
		}(i)
	}
	wg.Wait()
	if n := f.countItems(t) - before; n != 2 {
		t.Fatalf("six deliveries at once of one new to-do left %d tasks (want 2 in all)", n)
	}

	// Ya enlazado a mano a t-1: no hay tarea nueva, y su asignado entra en t-1.
	const manual = int64(6003)
	item, _ := f.svc.Task("t-1")
	if _, err := f.svc.LinkTask(item, todoURL(manual)); err != nil {
		t.Fatal(err)
	}
	f.setTodo(manual, "A mano", "", bcCarla)
	f.deliver(t, todoEvent(230, domain.BasecampTodoCreated, manual, bcAna))
	if n := f.countItems(t) - before; n != 2 {
		t.Fatalf("a to-do linked by hand made a task of its own (%d new)", n)
	}
	if got := f.assignees(t, "t-1"); strings.Join(got, ",") != "u-ana,u-bob,u-carla" {
		t.Fatalf("t-1 assignees %v", got)
	}
}

// Las dos guardas de «una sola tarea», cada una por su lado: la del servicio
// encuentra el enlace hecho a mano aunque el espacio del proyecto no tenga
// ninguna lista donde crear; y la del repositorio, si otro enlazó el to-do
// entre que se miró y se creó, devuelve la tarea de ese otro y no deja una
// segunda.
//
// Roturas que atrapa: no mirar el enlace antes de buscar lista (el to-do
// enlazado a mano se pierde); no volver al enlace ganador tras chocar con el
// índice único (el evento falla y la tarea nueva se pierde o se duplica).
func TestBothGuardsOfOneTaskPerTodo(t *testing.T) {
	f := newBasecampFixture(t)
	// El enlace a mano ya ganó: el repositorio no crea otra.
	const raced = int64(6040)
	item, _ := f.svc.Task("t-1")
	if _, err := f.svc.LinkTask(item, todoURL(raced)); err != nil {
		t.Fatal(err)
	}
	before := f.countItems(t)
	got, created, err := repository.NewBasecampRepository(f.db).CreateLinkedTask(
		&domain.Item{OrgID: "org-1", SpaceID: "sp-1", ListID: "l-1", Title: "Tarde", Status: domain.ReportPending,
			Visibility: domain.VisibilityInternal},
		&domain.BasecampLink{OrgID: "org-1", RecordingID: raced, ProjectID: bcProject})
	if err != nil || created || got == nil || got.ID != "t-1" {
		t.Fatalf("a lost race gave %+v created=%v err=%v", got, created, err)
	}
	if f.countItems(t) != before {
		t.Fatal("a lost race left a second task")
	}

	// El proyecto pasa a un espacio sin listas: el to-do enlazado a mano a
	// t-1 sigue llegando a t-1.
	sp := &domain.TaskSpace{OrgID: "org-1", Name: "Vacío", Key: "va", Rank: "0.6"}
	sp.ID = "sp-empty"
	mustCreate(t, f.db, sp)
	if _, err := f.svc.LinkProject("org-1", "u-ana", bcProject, "sp-empty"); err != nil {
		t.Fatal(err)
	}
	f.secret = strings.TrimPrefix(f.fake.hookPayloadURL, "https://cac.example/webhooks/basecamp/")
	f.setTodo(raced, "A mano", "", bcCarla)
	f.deliver(t, todoEvent(300, domain.BasecampTodoAssignment, raced, bcClient))
	if got := f.assignees(t, "t-1"); strings.Join(got, ",") != "u-ana,u-bob,u-carla" {
		t.Fatalf("the to-do linked by hand did not reach t-1: %v", got)
	}
}

// Los responsables siguen a Basecamp, pero Basecamp sólo quita a quien puso:
// a quien puso el equipo no lo toca aunque deje de estar asignado allí.
//
// Roturas que atrapa: no quitar a quien Basecamp ya no asigna; quitar también
// a quien puso el equipo; apuntar como «de Basecamp» a quien ya era
// responsable; avisar otra vez a quien ya lo era.
//
// Rotura equivalente, no un hueco: quitar la guarda `RowsAffected == 0` de
// RemoveBasecampAssignee no cambia nada que se vea, porque syncAssignees sólo
// le pasa ids que salen de BasecampAssignees (los que puso Basecamp).
func TestAssigneesFollowBasecampButOnlyThoseItPut(t *testing.T) {
	f := newBasecampFixture(t)
	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcBob)
	f.deliver(t, todoEvent(240, domain.BasecampTodoCreated, bcNewTodo, bcClient))
	task := f.taskOf(t, bcNewTodo)
	// El equipo añade a Ana a mano.
	mustCreate(t, f.db, &domain.ItemAssignee{ItemID: task.ID, UserID: "u-ana"})

	// En Basecamp pasa a Ana y Carla: Bob sale (lo puso Basecamp), Carla entra.
	f.inbox.reset()
	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcAna, bcCarla)
	f.deliver(t, todoEvent(241, domain.BasecampTodoAssignment, bcNewTodo, bcClient))
	if got := f.assignees(t, task.ID); strings.Join(got, ",") != "u-ana,u-carla" {
		t.Fatalf("assignees %v, want Ana and Carla", got)
	}
	if len(noticesOfKind(f.inbox.of("u-carla"), "task:assigned")) != 1 || len(noticesOfKind(f.inbox.of("u-ana"), "task:assigned")) != 0 {
		t.Fatalf("assigned notices: Carla %v, Ana %v", kinds(f.inbox.of("u-carla")), kinds(f.inbox.of("u-ana")))
	}

	// Nadie asignado en Basecamp: Carla sale; Ana, que puso el equipo, se queda.
	f.setTodo(bcNewTodo, "Revisar el checkout", "")
	f.deliver(t, todoEvent(242, domain.BasecampTodoAssignment, bcNewTodo, bcClient))
	if got := f.assignees(t, task.ID); strings.Join(got, ",") != "u-ana" {
		t.Fatalf("assignees %v, want only Ana (put by the team)", got)
	}
}

// Un to-do asignado a alguien sin casar, o a nadie, llega igual como tarea, y
// se le dice a quien enlazó el proyecto. Una persona casada con quien ya no es
// de la org no da responsable.
//
// Roturas que atrapa: no avisar a quien enlazó; avisarle de lo que hizo él;
// dar la tarea a quien no es de la org.
func TestAnUnmatchedAssigneeTellsWhoLinkedTheProject(t *testing.T) {
	f := newBasecampFixture(t)
	f.setTodo(bcNewTodo, "Para el nuevo", "", bcNewcomer)
	f.deliver(t, todoEvent(250, domain.BasecampTodoCreated, bcNewTodo, bcClient))
	task := f.taskOf(t, bcNewTodo)
	if task == nil {
		t.Fatal("a to-do assigned to someone unmatched made no task")
	}
	if got := f.assignees(t, task.ID); len(got) != 0 {
		t.Fatalf("assignees %v", got)
	}
	ana := f.inbox.of("u-ana")
	if len(ana) != 1 || ana[0].Kind != domain.BasecampActivityKind || ana[0].TitleKey != "notify.basecamp.todo_unmatched" ||
		ana[0].Link != "/tasks?task="+task.ID {
		t.Fatalf("whoever linked the project got %+v", ana)
	}

	// A nadie.
	f.inbox.reset()
	f.setTodo(6010, "Sin nadie", "")
	f.deliver(t, todoEvent(251, domain.BasecampTodoCreated, 6010, bcClient))
	if ana := f.inbox.of("u-ana"); len(ana) != 1 || ana[0].TitleKey != "notify.basecamp.todo_unassigned" {
		t.Fatalf("a to-do for nobody told %+v", ana)
	}
	// Si lo hizo quien enlazó, no se le avisa de lo suyo.
	f.inbox.reset()
	f.setTodo(6011, "Mío", "")
	f.deliver(t, todoEvent(252, domain.BasecampTodoCreated, 6011, bcAna))
	if ana := f.inbox.of("u-ana"); len(ana) != 0 {
		t.Fatalf("Ana was told of her own to-do: %v", kinds(ana))
	}

	// Casada con Dan, que no es de org-1: no es responsable.
	mustCreate(t, f.db, &domain.BasecampPerson{OrgID: "org-1", PersonID: 500, Name: "Dan Bc", UserID: "u-dan", MatchedBy: "manual"})
	f.setTodo(6012, "Para Dan", "", domain.BasecampPersonWire{ID: 500, Name: "Dan Bc"})
	f.deliver(t, todoEvent(253, domain.BasecampTodoCreated, 6012, bcClient))
	if got := f.assignees(t, f.taskOf(t, 6012).ID); len(got) != 0 {
		t.Fatalf("someone outside the org was made assignee: %v", got)
	}
	if dan := f.inbox.of("u-dan"); len(dan) != 0 {
		t.Fatalf("Dan, outside the org, was told %v", kinds(dan))
	}
}

// Completar o reabrir un to-do es una línea en la tarea, y no le cambia el
// estado: lo decide el equipo.
//
// Roturas que atrapa: mover el estado al completar o reabrir; no apuntar la
// reapertura.
func TestCompletingOrReopeningATodoDoesNotMoveTheTask(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t)
	f.db.Model(&domain.Item{}).Where("id = ?", "t-1").Update("status", domain.ReportInProgress)
	f.deliver(t, todoCompleted(260, "2026-11-27T10:00:00Z"))
	reopened := []byte(strings.Replace(string(todoCompleted(261, "2026-11-27T11:00:00Z")),
		`"todo_completed"`, `"todo_uncompleted"`, 1))
	f.deliver(t, reopened)
	cs := f.comments(t, "t-1")
	if len(cs) != 2 || !strings.Contains(cs[0].Body, " completed ") || !strings.Contains(cs[1].Body, " reopened ") {
		t.Fatalf("lines %+v", cs)
	}
	var it domain.Item
	f.db.First(&it, "id = ?", "t-1")
	if it.Status != domain.ReportInProgress {
		t.Fatalf("Basecamp moved the task to %s", it.Status)
	}
	if bob := f.inbox.of("u-bob"); len(bob) != 2 || bob[1].TitleKey != "notify.basecamp.todo_reopened" {
		t.Fatalf("Bob got %+v", bob)
	}
}

// Una tarea cuya descripción trae la URL de un to-do de la cuenta conectada
// se enlaza sola, al crearla y al editarla; la de otra cuenta no, y una ya
// enlazada a otra tarea se queda donde estaba.
//
// Roturas que atrapa: no enlazar al crear; no enlazar al editar; enlazar la
// URL de otra cuenta; quitarle el enlace a la tarea que ya lo tenía.
func TestATaskLinksItselfByTheURLInItsDescription(t *testing.T) {
	f := newBasecampFixture(t)
	f.linkTodo(t) // bcTodo ya es de t-1
	tasks := NewTaskService(repository.NewTaskRepository(f.db), repository.NewReportRepository(f.db),
		repository.NewOrganizationRepository(f.db), nil).WithDescriptionLinker(f.svc)
	list := &domain.TaskList{SpaceID: "sp-1", Name: "Backend"}
	list.ID = "l-1"
	desc := fmt.Sprintf("Ver [el to-do](%s).\nOtra cuenta: https://3.basecamp.com/1000/buckets/%d/todos/6100\nY %s, que ya es de otra.",
		todoURL(6020), bcProject, todoURL(bcTodo))
	created, err := tasks.CreateTask(context.Background(), list, "org-1", "u-ana",
		domain.CreateTaskRequest{Title: "Con enlace", Description: desc})
	if err != nil {
		t.Fatal(err)
	}
	if got := f.taskOf(t, 6020); got == nil || got.ID != created.ID {
		t.Fatalf("the to-do in the description was not linked to the new task: %+v", got)
	}
	var other int64
	f.db.Model(&domain.BasecampLink{}).Where("recording_id = ?", 6100).Count(&other)
	if other != 0 {
		t.Fatal("a URL of another Basecamp account was linked")
	}
	if got := f.taskOf(t, bcTodo); got == nil || got.ID != "t-1" {
		t.Fatalf("the to-do already linked to t-1 moved: %+v", got)
	}

	// Al editar.
	edited := "Ahora también " + todoURL(6021)
	if err := tasks.UpdateTask(context.Background(), created.ID, "u-ana", domain.UpdateTaskRequest{Description: &edited}); err != nil {
		t.Fatal(err)
	}
	if got := f.taskOf(t, 6021); got == nil || got.ID != created.ID {
		t.Fatalf("editing the description did not link its to-do: %+v", got)
	}
}

// Nada cruza organizaciones: una tarea de otra org que cita un to-do de esta
// cuenta no se enlaza (esa org no la tiene conectada), y lo que Basecamp
// devuelve de otro proyecto no hace tarea aquí.
//
// Roturas que atrapa: enlazar por la descripción sin mirar la conexión de la
// org de la tarea; crear la tarea con un to-do que Basecamp dice de otro
// proyecto; poner responsables en una tarea enlazada a otro proyecto.
func TestABasecampTodoNeverCrossesOrganizations(t *testing.T) {
	f := newBasecampFixture(t)
	f.addSecondOrg(t)
	foreign := &domain.Item{OrgID: "org-2", Title: "De org-2", Description: "Mira " + todoURL(6030)}
	foreign.ID = "t-2"
	f.svc.LinkFromDescription(foreign)
	var n int64
	f.db.Model(&domain.BasecampLink{}).Where("recording_id = ?", 6030).Count(&n)
	if n != 0 {
		t.Fatal("a task of another organization was linked to this account's to-do")
	}

	before := f.countItems(t)
	body, _ := json.Marshal(map[string]any{"id": 6031, "title": "Ajeno", "app_url": todoURL(6031), "assignees": []any{},
		"bucket": map[string]any{"id": 43, "name": "Otro"}})
	f.fake.mu.Lock()
	f.fake.recordings["todos/6031"] = string(body)
	f.fake.mu.Unlock()
	f.deliver(t, todoEvent(270, domain.BasecampTodoCreated, 6031, bcClient))
	if f.countItems(t) != before {
		t.Fatal("a to-do Basecamp says is of another project made a task here")
	}

	// t-1 enlazada a mano a un recording de otro proyecto: el secreto de éste
	// no le pone responsables.
	item, _ := f.svc.Task("t-1")
	if _, err := f.svc.LinkTask(item, fmt.Sprintf("https://3.basecamp.com/%d/buckets/43/todos/6032", bcAccount)); err != nil {
		t.Fatal(err)
	}
	f.setTodo(6032, "Cruzado", "", bcCarla)
	f.deliver(t, todoEvent(271, domain.BasecampTodoAssignment, 6032, bcClient))
	if got := f.assignees(t, "t-1"); strings.Join(got, ",") != "u-ana,u-bob" {
		t.Fatalf("this project's secret assigned a task linked to another project: %v", got)
	}
}

// Si Basecamp no contesta al leer el to-do, el evento se suelta para que su
// reintento lo traiga; un to-do que ya no existe se olvida.
//
// Roturas que atrapa: dar por atendido un alta que no se pudo leer; pedir el
// reintento de lo que ningún reintento arregla (sin conexión).
func TestATodoThatCannotBeReadIsRetried(t *testing.T) {
	f := newBasecampFixture(t)
	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcBob)
	f.fake.mu.Lock()
	f.fake.recordingsDown = true
	f.fake.mu.Unlock()
	if err := f.svc.HandleWebhook(f.secret, todoEvent(280, domain.BasecampTodoCreated, bcNewTodo, bcAna)); err == nil {
		t.Fatal("a to-do that could not be read was taken as handled")
	}
	f.fake.mu.Lock()
	f.fake.recordingsDown = false
	f.fake.mu.Unlock()
	f.deliver(t, todoEvent(280, domain.BasecampTodoCreated, bcNewTodo, bcAna))
	if f.taskOf(t, bcNewTodo) == nil {
		t.Fatal("the retry did not make the task")
	}
	f.deliver(t, todoEvent(281, domain.BasecampTodoCreated, 6099, bcAna))
	if f.taskOf(t, 6099) != nil {
		t.Fatal("a to-do that no longer exists made a task")
	}
	// Sin conexión, reintentar no arregla nada, y diez fallos desactivan el
	// webhook entero: se contesta que sí.
	f.db.Where("org_id = ?", "org-1").Delete(&domain.BasecampConnection{})
	f.setTodo(6098, "Sin conexión", "", bcBob)
	if err := f.svc.HandleWebhook(f.secret, todoEvent(282, domain.BasecampTodoCreated, 6098, bcAna)); err != nil {
		t.Fatalf("without a connection the event was refused (Basecamp would retry it until it disables the webhook): %v", err)
	}
}

// Una tarjeta nueva y asignada también es una tarea, leída por su ruta.
//
// Rotura que atrapa: leer una tarjeta como si fuera un to-do (sus notas van
// en `content`).
func TestANewCardBecomesALinkedTask(t *testing.T) {
	f := newBasecampFixture(t)
	body, _ := json.Marshal(map[string]any{"id": 7001, "type": "Kanban::Card", "title": "Diseño del alta",
		"content": "<div>Con el logo nuevo</div>", "app_url": fmt.Sprintf("https://3.basecamp.com/%d/buckets/%d/card_tables/cards/7001", bcAccount, bcProject),
		"assignees": []any{bcCarla}, "bucket": map[string]any{"id": bcProject, "name": "Proteus"}})
	f.fake.mu.Lock()
	f.fake.recordings["card_tables/cards/7001"] = string(body)
	f.fake.mu.Unlock()
	ev := []byte(strings.ReplaceAll(strings.ReplaceAll(string(todoEvent(290, domain.BasecampCardCreated, 7001, bcAna)),
		`"type":"Todo"`, `"type":"Kanban::Card"`), "/todos/7001", "/card_tables/cards/7001"))
	f.deliver(t, ev)
	task := f.taskOf(t, 7001)
	if task == nil || task.Title != "Diseño del alta" || !strings.Contains(task.Description, "Con el logo nuevo") {
		t.Fatalf("the card made %+v", task)
	}
	if got := f.assignees(t, task.ID); strings.Join(got, ",") != "u-carla" {
		t.Fatalf("assignees %v", got)
	}
}

// Los webhooks dados de alta con otros tipos (una versión anterior) se ponen
// al día solos, cambiando el que hay con su misma dirección; uno al día no se
// toca, y uno cuya dirección no es la del proyecto tampoco.
//
// Roturas que atrapa: dar de alta otro webhook; cambiarle la dirección; no
// apuntar los tipos nuevos (se cambia en cada vuelta del reloj); tocar uno
// que ya está al día; fiarse de una dirección con otro secreto.
func TestOldWebhooksLearnTheNewTypesWithoutANewOne(t *testing.T) {
	f := newBasecampFixture(t)
	posts := f.fake.hookPosts
	payload := f.fake.hookPayloadURL

	f.svc.ReconcileWebhooks()
	if len(f.fake.hookPuts) != 0 {
		t.Fatalf("a webhook already up to date was changed: %+v", f.fake.hookPuts)
	}

	f.db.Model(&domain.BasecampProject{}).Where("project_id = ?", bcProject).Update("webhook_types", "Comment,Message")
	f.svc.ReconcileWebhooks()
	f.svc.ReconcileWebhooks()
	if f.fake.hookPosts != posts {
		t.Fatal("reconciling registered another webhook")
	}
	if len(f.fake.hookPuts) != 1 {
		t.Fatalf("want one update, got %+v", f.fake.hookPuts)
	}
	put := f.fake.hookPuts[0]
	if put.hook != "7.json" || put.payloadURL != payload || strings.Join(put.types, ",") != domain.BasecampTypesKey(domain.BasecampWebhookTypes) {
		t.Fatalf("the update was %+v", put)
	}

	// Una dirección que no lleva el secreto de la fila no se toca.
	f.db.Model(&domain.BasecampProject{}).Where("project_id = ?", bcProject).Update("webhook_types", "Comment")
	f.fake.mu.Lock()
	f.fake.hookPayloads["7.json"] = "https://cac.example/webhooks/basecamp/otro-secreto-que-no-es-el-de-la-fila"
	f.fake.mu.Unlock()
	f.svc.ReconcileWebhooks()
	if len(f.fake.hookPuts) != 1 {
		t.Fatalf("a webhook with another project's address was changed: %+v", f.fake.hookPuts)
	}
}

// Las URLs de Basecamp de una descripción: dentro de un enlace de markdown o
// al final de una frase, cada recording una vez, y sólo de basecamp.com.
//
// Roturas que atrapa: llevarse el paréntesis o el punto final; repetir un
// recording; aceptar un host que sólo se parece.
func TestBasecampURLsInADescription(t *testing.T) {
	text := "Ver [esto](https://3.basecamp.com/1/buckets/2/todos/3). Otra vez https://3.basecamp.com/1/buckets/2/todos/3, " +
		"y la tarjeta https://3.basecamp.com/1/buckets/2/card_tables/cards/4.\n" +
		"Falsa: https://3.basecamp.com.evil.io/1/buckets/2/todos/5 y http://3.basecamp.com/1/buckets/2/todos/6"
	got := domain.BasecampURLsIn(text)
	want := []string{"https://3.basecamp.com/1/buckets/2/todos/3", "https://3.basecamp.com/1/buckets/2/card_tables/cards/4"}
	if strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("got %q, want %q", got, want)
	}
}

// Un to-do cuya tarea de cac se borró sigue llegando como «sin tarea», igual
// que en la fase 1: no se recrea (quien la borró lo hizo a propósito), se
// avisa a quien enlazó el proyecto, y el webhook contesta que lo atendió.
// Un error aquí haría reintentar a Basecamp diez veces y desactivar el
// webhook entero del proyecto, y con él lo de la fase 1.
//
// Roturas que atrapa: devolver el error de la tarea que no está (tanto al
// buscar la enlazada como al perder la carrera contra un enlace a una tarea
// borrada); recrear la tarea borrada; no avisar a nadie; avisar de la
// asignación como si fuera un alta.
func TestADeletedTaskDoesNotFailTheWebhook(t *testing.T) {
	f := newBasecampFixture(t)
	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcBob)
	f.deliver(t, todoEvent(400, domain.BasecampTodoCreated, bcNewTodo, bcClient))
	task := f.taskOf(t, bcNewTodo)
	if task == nil {
		t.Fatal("a new to-do made no task")
	}
	if err := f.db.Delete(&domain.Item{}, "id = ?", task.ID).Error; err != nil {
		t.Fatal(err)
	}
	before := f.countItems(t)
	f.inbox.reset()

	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcCarla)
	comment := []byte(strings.ReplaceAll(string(commentEvent(403, 9403, bcClient, "<div>¿Sigue?</div>")),
		fmt.Sprint(bcTodo), fmt.Sprint(bcNewTodo)))
	for _, body := range [][]byte{
		todoEvent(401, domain.BasecampTodoAssignment, bcNewTodo, bcClient),
		todoEvent(402, domain.BasecampTodoCompleted, bcNewTodo, bcClient),
		comment,
		todoEvent(404, domain.BasecampTodoUncompleted, bcNewTodo, bcClient),
		todoEvent(405, domain.BasecampTodoCreated, bcNewTodo, bcClient),
	} {
		if err := f.svc.HandleWebhook(f.secret, body); err != nil {
			t.Fatalf("an event of a to-do whose task was deleted failed the webhook: %v", err)
		}
	}
	if n := f.countItems(t) - before; n != 0 {
		t.Fatalf("the deleted task came back as %d new ones", n)
	}
	if got := noticesOfKind(f.inbox.of("u-carla"), "task:assigned"); len(got) != 0 {
		t.Fatalf("Carla was given a deleted task: %+v", got)
	}
	var keys []string
	for _, a := range f.inbox.of("u-ana") {
		keys = append(keys, a.TitleKey)
	}
	want := "notify.basecamp.todo_assigned,notify.basecamp.todo_done,notify.basecamp.comment," +
		"notify.basecamp.todo_reopened,notify.basecamp.todo_created"
	if strings.Join(keys, ",") != want {
		t.Fatalf("whoever linked the project was told %v, want %s", keys, want)
	}

	// La carrera: el enlace que ganó apunta a una tarea borrada.
	got, created, err := repository.NewBasecampRepository(f.db).CreateLinkedTask(
		&domain.Item{OrgID: "org-1", SpaceID: "sp-1", ListID: "l-1", Title: "Tarde", Status: domain.ReportPending,
			Visibility: domain.VisibilityInternal},
		&domain.BasecampLink{OrgID: "org-1", RecordingID: bcNewTodo, ProjectID: bcProject})
	if err != nil || created || got != nil {
		t.Fatalf("losing the race to a deleted task gave %+v created=%v err=%v", got, created, err)
	}
	if n := f.countItems(t) - before; n != 0 {
		t.Fatalf("losing the race left %d tasks", n)
	}
}

// Basecamp contesta que no (403: quien conectó ya no ve ese proyecto) al leer
// el to-do: es una respuesta, no un silencio, y reintentar da lo mismo. Se
// atiende; sólo se suelta cuando Basecamp no contesta (5xx) o pide esperar.
//
// Roturas que atrapa: soltar el evento ante un 403 (Basecamp reintenta hasta
// desactivar el webhook); atender un 429 o un 5xx (se perdería el to-do).
func TestABasecampRefusalIsHandledButAnOutageIsRetried(t *testing.T) {
	f := newBasecampFixture(t)
	f.setTodo(bcNewTodo, "Revisar el checkout", "", bcBob)
	set := func(status int) {
		f.fake.mu.Lock()
		f.fake.recordingsStatus = status
		f.fake.mu.Unlock()
	}
	set(403)
	if err := f.svc.HandleWebhook(f.secret, todoEvent(410, domain.BasecampTodoCreated, bcNewTodo, bcAna)); err != nil {
		t.Fatalf("a 403 from Basecamp was released for a retry that cannot change it: %v", err)
	}
	if f.taskOf(t, bcNewTodo) != nil {
		t.Fatal("a to-do Basecamp refused to show made a task")
	}
	for i, status := range []int{429, 502} {
		set(status)
		if err := f.svc.HandleWebhook(f.secret, todoEvent(411+int64(i), domain.BasecampTodoCreated, bcNewTodo, bcAna)); err == nil {
			t.Fatalf("a %d was taken as handled: the to-do would be lost", status)
		}
	}
	set(0)
	f.deliver(t, todoEvent(412, domain.BasecampTodoCreated, bcNewTodo, bcAna))
	if f.taskOf(t, bcNewTodo) == nil {
		t.Fatal("the retry after an outage made no task")
	}
}
