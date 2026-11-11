package service

import (
	"context"
	"strings"
	"testing"
	"time"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Mover una tarea a otra lista: lo que tiene que viajar con ella y lo que no
// puede cruzar. TestACardCanMoveToAnotherList (task_visibility_test.go) cubre
// lo básico; aquí van las reglas que faltaban cuando el movimiento se abrió a
// la app y al MCP (command-and-control-328).

// moveFixture: el espacio de visibilityDB con clave, y un segundo espacio con
// su lista. Las claves hacen falta para leer la ref en el rastro.
func moveFixture(t *testing.T, db *gorm.DB) {
	t.Helper()
	if err := db.Model(&domain.TaskSpace{}).Where("id = ?", "space-1").Update("key", "uno").Error; err != nil {
		t.Fatal(err)
	}
	space2 := &domain.TaskSpace{OrgID: "org-1", Name: "Segundo", Key: "dos", Rank: "0.6"}
	space2.ID = "space-2"
	other := &domain.TaskList{SpaceID: "space-2", Name: "Otra", Rank: "0.5"}
	other.ID = "list-2"
	for _, m := range []any{space2, other} {
		if err := db.Create(m).Error; err != nil {
			t.Fatal(err)
		}
	}
}

func moveSvc(db *gorm.DB, hub *events.Hub) (*TaskService, *repository.TaskRepository) {
	repo := repository.NewTaskRepository(db)
	return NewTaskService(repo, repository.NewReportRepository(db), repository.NewOrganizationRepository(db), hub), repo
}

func commentBodies(t *testing.T, db *gorm.DB, itemID string) []string {
	t.Helper()
	var bodies []string
	if err := db.Model(&domain.ItemComment{}).Where("item_id = ?", itemID).Pluck("body", &bodies).Error; err != nil {
		t.Fatal(err)
	}
	return bodies
}

// Una interna se numera por espacio. Al cruzar toma el siguiente número del
// destino; con el suyo chocaba con el #1 que ya había allí, o su ref cambiaba
// de prefijo sin que nadie lo dijera.
func TestMovingAcrossSpacesTakesTheNextNumberThere(t *testing.T) {
	db, cleanup := visibilityDB(t)
	defer cleanup()
	moveFixture(t, db)
	svc, repo := moveSvc(db, nil)
	ctx := context.Background()
	list1, _ := repo.FindList("list-1")
	list2, _ := repo.FindList("list-2")

	// El destino ya tiene su #1.
	if _, err := svc.CreateTask(ctx, list2, "org-1", "u-1", domain.CreateTaskRequest{Title: "la que ya estaba"}); err != nil {
		t.Fatal(err)
	}
	card, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "en el espacio equivocado"})
	if err != nil {
		t.Fatal(err)
	}
	if card.Seq != 1 {
		t.Fatalf("fixture: la tarjeta debía nacer #1, nació #%d", card.Seq)
	}

	dest := "list-2"
	if err := svc.UpdateTask(ctx, card.ID, "u-1", domain.UpdateTaskRequest{ListID: &dest}); err != nil {
		t.Fatal(err)
	}
	moved, _ := repo.FindTask(card.ID)
	if moved.Seq != 2 {
		t.Errorf("en el espacio de destino debía ser la #2 (la #1 ya existe), quedó #%d", moved.Seq)
	}

	bodies := commentBodies(t, db, card.ID)
	if len(bodies) != 1 || !strings.Contains(bodies[0], "`uno-1`") || !strings.Contains(bodies[0], "`dos-2`") {
		t.Errorf("tenía que quedar un comentario con la ref vieja y la nueva, hay %q", bodies)
	}
}

// Dentro del mismo espacio la ref no cambia, y no hay rastro que dejar.
func TestMovingWithinASpaceKeepsTheRef(t *testing.T) {
	db, cleanup := visibilityDB(t)
	defer cleanup()
	moveFixture(t, db)
	svc, repo := moveSvc(db, nil)
	ctx := context.Background()
	sibling := &domain.TaskList{SpaceID: "space-1", Name: "Hermana", Rank: "0.7"}
	sibling.ID = "list-3"
	if err := db.Create(sibling).Error; err != nil {
		t.Fatal(err)
	}
	list1, _ := repo.FindList("list-1")
	card, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "de lista, no de espacio"})
	if err != nil {
		t.Fatal(err)
	}

	dest := "list-3"
	if err := svc.UpdateTask(ctx, card.ID, "u-1", domain.UpdateTaskRequest{ListID: &dest}); err != nil {
		t.Fatal(err)
	}
	moved, _ := repo.FindTask(card.ID)
	if moved.ListID != "list-3" || moved.Seq != card.Seq {
		t.Errorf("debía estar en list-3 con el mismo número (#%d), está en %s #%d", card.Seq, moved.ListID, moved.Seq)
	}
	if bodies := commentBodies(t, db, card.ID); len(bodies) != 0 {
		t.Errorf("sin cambio de ref no hay rastro que dejar, hay %q", bodies)
	}
}

// Las subtareas viven en la lista de su padre: viajan con él, y cruzando de
// espacio se renumeran también (son internas siempre).
func TestSubtasksTravelWithTheirParent(t *testing.T) {
	db, cleanup := visibilityDB(t)
	defer cleanup()
	moveFixture(t, db)
	svc, repo := moveSvc(db, nil)
	ctx := context.Background()
	list1, _ := repo.FindList("list-1")
	list2, _ := repo.FindList("list-2")
	if _, err := svc.CreateTask(ctx, list2, "org-1", "u-1", domain.CreateTaskRequest{Title: "ocupa el #1"}); err != nil {
		t.Fatal(err)
	}
	parent, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "padre"})
	if err != nil {
		t.Fatal(err)
	}
	child, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "paso", ParentID: parent.ID})
	if err != nil {
		t.Fatal(err)
	}
	grandchild, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "sub-paso", ParentID: child.ID})
	if err != nil {
		t.Fatal(err)
	}

	dest := "list-2"
	if err := svc.UpdateTask(ctx, parent.ID, "u-1", domain.UpdateTaskRequest{ListID: &dest}); err != nil {
		t.Fatal(err)
	}
	seqs := map[int]bool{}
	for _, id := range []string{parent.ID, child.ID, grandchild.ID} {
		it, _ := repo.FindTask(id)
		if it.ListID != "list-2" || it.SpaceID != "space-2" {
			t.Errorf("%q debía ir con su padre a list-2/space-2, está en %s/%s", it.Title, it.ListID, it.SpaceID)
		}
		if it.Seq == 1 || seqs[it.Seq] {
			t.Errorf("%q quedó con el #%d, repetido en el espacio de destino", it.Title, it.Seq)
		}
		seqs[it.Seq] = true
	}
}

func TestASubtaskCannotMoveAlone(t *testing.T) {
	db, cleanup := visibilityDB(t)
	defer cleanup()
	moveFixture(t, db)
	svc, repo := moveSvc(db, nil)
	ctx := context.Background()
	list1, _ := repo.FindList("list-1")
	parent, _ := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "padre"})
	child, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "paso", ParentID: parent.ID})
	if err != nil {
		t.Fatal(err)
	}

	dest := "list-2"
	err = svc.UpdateTask(ctx, child.ID, "u-1", domain.UpdateTaskRequest{ListID: &dest})
	if err != repository.ErrMoveSubtask {
		t.Errorf("una subtarea sola no se mueve: esperaba ErrMoveSubtask, salió %v", err)
	}
	if it, _ := repo.FindTask(child.ID); it.ListID != "list-1" {
		t.Errorf("y debía quedarse donde estaba, está en %s", it.ListID)
	}
}

// Una tarjeta con folio sale en el tablero del cliente por su project_id, no
// por su lista: fuera de su canal quedaría en su tablero colgando de una lista
// interna. Una interna, en cambio, sí puede entrar en la lista de un cliente.
func TestAClientCardStaysInsideItsChannel(t *testing.T) {
	db, cleanup := visibilityDB(t)
	defer cleanup()
	moveFixture(t, db)
	svc, repo := moveSvc(db, nil)
	ctx := context.Background()
	proj2 := &domain.ReportProject{OrgID: "org-1", Name: "Otro cliente", Slug: "otro", IngestKeyHash: []byte("h2")}
	proj2.ID = "proj-2"
	sameChannel := &domain.TaskList{SpaceID: "space-2", Name: "Del mismo cliente", Rank: "0.6"}
	sameChannel.ID = "list-same"
	otherChannel := &domain.TaskList{SpaceID: "space-2", Name: "De otro cliente", Rank: "0.7"}
	otherChannel.ID = "list-other"
	for _, m := range []any{proj2, sameChannel, otherChannel} {
		if err := db.Create(m).Error; err != nil {
			t.Fatal(err)
		}
	}
	for list, proj := range map[string]string{"list-1": "proj-1", "list-same": "proj-1", "list-other": "proj-2"} {
		if err := repo.BindListToChannel(list, proj); err != nil {
			t.Fatal(err)
		}
	}
	list1, _ := repo.FindList("list-1")
	card, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "del cliente"})
	if err != nil {
		t.Fatal(err)
	}
	if card.ProjectID != "proj-1" {
		t.Fatalf("fixture: debía nacer con el canal proj-1, tiene %q", card.ProjectID)
	}

	for _, dest := range []string{"list-2", "list-other"} {
		d := dest
		if err := svc.UpdateTask(ctx, card.ID, "u-1", domain.UpdateTaskRequest{ListID: &d}); err != repository.ErrMoveLeavesChannel {
			t.Errorf("a %s no puede ir: esperaba ErrMoveLeavesChannel, salió %v", dest, err)
		}
	}

	// A otra lista del mismo cliente sí, y con su folio intacto aunque cambie de espacio.
	same := "list-same"
	if err := svc.UpdateTask(ctx, card.ID, "u-1", domain.UpdateTaskRequest{ListID: &same}); err != nil {
		t.Fatalf("dentro de su canal debía poder moverse: %v", err)
	}
	if it, _ := repo.FindTask(card.ID); it.ListID != "list-same" || it.Seq != card.Seq {
		t.Errorf("debía estar en list-same con el folio #%d, está en %s #%d", card.Seq, it.ListID, it.Seq)
	}

	// Una interna sin canal entra en la lista de un cliente y sigue siendo interna.
	plain, _ := repo.FindList("list-2")
	ours, err := svc.CreateTask(ctx, plain, "org-1", "u-1", domain.CreateTaskRequest{Title: "nuestra"})
	if err != nil {
		t.Fatal(err)
	}
	other := "list-other"
	if err := svc.UpdateTask(ctx, ours.ID, "u-1", domain.UpdateTaskRequest{ListID: &other}); err != nil {
		t.Fatalf("una interna puede entrar en la lista de un cliente: %v", err)
	}
	if it, _ := repo.FindTask(ours.ID); it.IsVisibleToChannel() {
		t.Error("y no por eso la ve el cliente: publicarla es otra decisión")
	}
}

// El tablero que la tenía se entera: si no, la tarjeta se queda pintada en el
// origen hasta recargar.
func TestTheListItLeftIsTold(t *testing.T) {
	db, cleanup := visibilityDB(t)
	defer cleanup()
	moveFixture(t, db)
	hub := events.NewHub()
	hears, closeHub := hub.Subscribe("u-observer", []string{"org-1"})
	defer closeHub()
	svc, repo := moveSvc(db, hub)
	ctx := context.Background()
	list1, _ := repo.FindList("list-1")
	card, err := svc.CreateTask(ctx, list1, "org-1", "u-1", domain.CreateTaskRequest{Title: "se va"})
	if err != nil {
		t.Fatal(err)
	}
	drain(hears)

	dest := "list-2"
	if err := svc.UpdateTask(ctx, card.ID, "u-1", domain.UpdateTaskRequest{ListID: &dest}); err != nil {
		t.Fatal(err)
	}
	lists := map[string]bool{}
	for {
		select {
		case e := <-hears:
			if d, ok := e.Data.(map[string]string); ok && d["taskId"] == card.ID {
				lists[d["listId"]] = true
			}
			continue
		case <-time.After(250 * time.Millisecond):
		}
		break
	}
	if !lists["list-1"] {
		t.Errorf("la lista de origen no recibió ningún evento; recibieron %v", lists)
	}
	if !lists["list-2"] {
		t.Errorf("la de destino tampoco puede quedarse sin el suyo; recibieron %v", lists)
	}
}

func drain(c <-chan events.Event) {
	for {
		select {
		case <-c:
		case <-time.After(100 * time.Millisecond):
			return
		}
	}
}
