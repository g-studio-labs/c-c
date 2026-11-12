package service

import (
	"errors"
	"fmt"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Lo relacionado con una cosa sale desde los dos extremos, junte la tabla que
// junte (command-and-control-348, decisión 7b3cc169). Todo se escribe por los
// caminos de siempre —crear una página, apuntar una decisión, un incidente,
// un comentario—, que es lo que tiene que dejar las aristas del texto.
//
// Roturas que atrapa: no guardar lo que nombra un texto (una ref o un enlace
// de la app); no recalcularlo al reescribirlo; que el hilo de una tarea no
// cuente; no enseñar una arista desde el otro extremo (decisión, incidente,
// bloqueo, subtarea, servidor); responder a quien no es de la org; no
// resolver una tarea por su ref.
func TestNeighborsAreSeenFromBothEnds(t *testing.T) {
	db := incidentDB(t)
	if err := db.AutoMigrate(&domain.GraphLink{}, &domain.SpaceKeyAlias{}, &domain.ItemComment{}, &domain.DocPageVersion{}); err != nil {
		t.Fatal(err)
	}
	tasks := repository.NewTaskRepository(db)
	docs := repository.NewDocRepository(db)
	task := func(id string, seq int, parent *string) {
		it := &domain.Item{OrgID: "org-1", SpaceID: "sp-1", ListID: "l-back", Title: "tarea " + id, Status: domain.ReportPending, Seq: seq, ParentID: parent}
		it.ID = id
		mustCreate(t, db, it)
	}
	a := "t-a"
	task(a, 10, nil)
	task("t-b", 11, &a)
	task("t-c", 12, nil)
	mustCreate(t, db, &domain.ItemBlock{ItemID: "t-c", BlockedByID: a})

	page, err := docs.CreatePage("org-1", domain.DocOwnerList, "l-back",
		domain.CreateDocPageRequest{Title: "Diseño", Body: "Sale en cc-10; ver [la otra](/tasks?task=t-c-no-es-uuid) y [esta](/tasks?task=00000000-0000-0000-0000-00000000000c)."}, "u-1")
	if err != nil {
		t.Fatal(err)
	}
	// Un enlace por id sólo cuenta si la cosa existe: la tarea con ese uuid, sí.
	cID := "00000000-0000-0000-0000-00000000000c"
	db.Exec(`UPDATE items SET id = ? WHERE id = 't-c'`, cID)
	db.Exec(`UPDATE item_blocks SET item_id = ? WHERE item_id = 't-c'`, cID)
	if err := docs.SavePageRenaming(page, nil, strPtr(page.Body), "u-1", nil); err != nil {
		t.Fatal(err)
	}

	doc, _ := docs.Find(domain.DocOwnerList, "l-back")
	dec, err := docs.AddDecisionWithLinks(&domain.Decision{DocID: doc.ID, Title: "Va así", Body: "Lo hace cc-11."}, "org-1", []string{a}, nil)
	if err != nil {
		t.Fatal(err)
	}
	inc := record(t, incidentSvc(db), "l-back", domain.IncidentRequest{Title: "mordió", Body: "En cc-12.",
		Servers: []string{"gstd"}, OriginTaskID: a})
	if err := tasks.CreateComment(&domain.TaskComment{ItemID: cID, Body: "depende de cc-10"}); err != nil {
		t.Fatal(err)
	}

	g := NewGraphService(repository.NewGraphRepository(db), tasks)
	has := func(kind domain.EntityKind, id string, want ...string) {
		t.Helper()
		got, err := g.Related(kind, id, []string{"org-1"}, false)
		if err != nil {
			t.Fatalf("%s %s: %v", kind, id, err)
		}
		set := map[string]bool{}
		for _, n := range got.Neighbors {
			set[fmt.Sprintf("%s %s %s:%s", n.Dir, n.Rel, n.Kind, n.ID)] = true
		}
		for _, w := range want {
			if !set[w] {
				t.Errorf("%s %s no ve «%s»: %v", kind, id, w, set)
			}
		}
	}
	has(domain.EntityTask, a,
		"in mentions page:"+page.ID, "in applies_to decision:"+dec.ID, "in origin incident:"+inc.ID,
		"out parent_of task:t-b", "out blocks task:"+cID, "in mentions task:"+cID)
	has(domain.EntityPage, page.ID, "out mentions task:"+a, "out mentions task:"+cID)
	has(domain.EntityIncident, inc.ID, "out origin task:"+a, "out bit_on server:srv-gstd", "out mentions task:"+cID)
	has(domain.EntityServer, "srv-gstd", "in bit_on incident:"+inc.ID)
	has(domain.EntityDecision, dec.ID, "out applies_to task:"+a, "out mentions task:t-b")
	has(domain.EntityTask, "t-b", "in parent_of task:"+a, "in mentions decision:"+dec.ID)
	has(domain.EntityTask, cID, "in blocks task:"+a, "in mentions page:"+page.ID, "in mentions incident:"+inc.ID)

	// Por su ref, con su título y su ref.
	byRef, err := g.Related(domain.EntityTask, "cc-10", []string{"org-1"}, false)
	if err != nil || byRef.ID != a || byRef.Title != "tarea t-a" {
		t.Fatalf("por ref: %+v %v", byRef, err)
	}
	for _, n := range byRef.Neighbors {
		if n.ID == "t-b" && n.Ref != "cc-11" {
			t.Errorf("un vecino sin su ref: %+v", n)
		}
	}
	// Otra org no ve nada, ni que existe.
	if _, err := g.Related(domain.EntityTask, a, []string{"org-2"}, false); !errors.Is(err, ErrGraphNotFound) {
		t.Errorf("otra org ve la tarea: %v", err)
	}
	// Reescribir el texto recalcula lo que nombra.
	if err := docs.SavePageRenaming(page, nil, strPtr("ya no nombra nada"), "u-1", nil); err != nil {
		t.Fatal(err)
	}
	got, _ := g.Related(domain.EntityTask, a, []string{"org-1"}, false)
	for _, n := range got.Neighbors {
		if n.Kind == domain.EntityPage {
			t.Errorf("la página sigue nombrando la tarea tras reescribirla: %+v", n)
		}
	}
}

func strPtr(s string) *string { return &s }
