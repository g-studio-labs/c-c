package service

import (
	"context"
	"sort"
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Borrar una tarea avisa de cada fila que se va, no sólo de la madre: una
// subtarea asignada a otra persona está en su «Mi trabajo», y su cliente sólo
// olvida el taskId que le llega (command-and-control-386).
//
// Roturas que atrapa: publicar sólo el `task:delete` de la madre; y publicar
// los ids de las subtareas con la lista o el orden equivocados.
func TestDeletingATaskAnnouncesEverySubtaskThatWentWithIt(t *testing.T) {
	db, cleanup := visibilityDB(t)
	defer cleanup()
	repo := repository.NewTaskRepository(db)
	hub := events.NewHub()
	hears, closeHub := hub.Subscribe("u-observer", []string{"org-1"})
	defer closeHub()
	svc := NewTaskService(repo, repository.NewReportRepository(db), repository.NewOrganizationRepository(db), hub)
	list, err := repo.FindList("list-1")
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	parent, err := svc.CreateTask(ctx, list, "org-1", "u-1", domain.CreateTaskRequest{Title: "madre"})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{parent.ID}
	for _, title := range []string{"una", "otra"} {
		sub, err := svc.CreateTask(ctx, list, "org-1", "u-1", domain.CreateTaskRequest{Title: title, ParentID: parent.ID})
		if err != nil {
			t.Fatal(err)
		}
		want = append(want, sub.ID)
	}
	survivor, err := svc.CreateTask(ctx, list, "org-1", "u-1", domain.CreateTaskRequest{Title: "ajena"})
	if err != nil {
		t.Fatal(err)
	}
	drain(hears)

	if err := svc.DeleteTask(parent.ID); err != nil {
		t.Fatal(err)
	}

	var got []string
	for {
		select {
		case e := <-hears:
			if e.Type != "task:delete" {
				continue
			}
			data := e.Data.(map[string]string)
			if data["listId"] != "list-1" {
				t.Errorf("el aviso de %s sale con la lista %q", data["taskId"], data["listId"])
			}
			got = append(got, data["taskId"])
			continue
		case <-time.After(250 * time.Millisecond):
		}
		break
	}
	sort.Strings(got)
	sort.Strings(want)
	if len(got) != len(want) {
		t.Fatalf("debían salir task:delete de %v, salieron de %v", want, got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("debían salir task:delete de %v, salieron de %v", want, got)
		}
		if got[i] == survivor.ID {
			t.Fatal("avisó de una tarea que no se borró")
		}
	}
}
