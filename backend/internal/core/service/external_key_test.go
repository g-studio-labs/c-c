package service

import (
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// La clave de un tracker de fuera (`PROTEUS-165`, de Jira) lleva a la tarea
// que la nombra, y nunca a la tarea de cac con ese número: numeran aparte
// (command-and-control-353). Se guarda al crear y al editar la tarea, por los
// caminos de siempre.
//
// Roturas que atrapa: no guardar las claves al crear o al editar; dejar la
// clave vieja cuando la descripción deja de nombrarla; mirar sólo la
// descripción y no el título; contestar con la tarea de otra organización;
// enseñar una archivada; no poner las claves en la tarjeta del tablero.
func TestATrackerKeyLeadsToTheTaskThatNamesIt(t *testing.T) {
	db := incidentDB(t)
	if err := db.AutoMigrate(&domain.GraphLink{}, &domain.SpaceKeyAlias{}, &domain.ItemComment{},
		&domain.ItemAttachment{}, &domain.TaskExternalKey{}); err != nil {
		t.Fatal(err)
	}
	tasks := repository.NewTaskRepository(db)
	create := func(org, title, description string) string {
		it := &domain.Task{OrgID: org, ListID: "l-back", Title: title, Description: description, Status: domain.ReportPending}
		if err := tasks.CreateTask(it, "sp-1"); err != nil {
			t.Fatal(err)
		}
		return it.ID
	}
	named := func(orgs []string, key string) []string {
		t.Helper()
		found, err := tasks.FindByExternalKey(orgs, key)
		if err != nil {
			t.Fatal(err)
		}
		ids := []string{}
		for _, x := range found {
			ids = append(ids, x.ID)
		}
		return ids
	}
	same := func(got []string, want ...string) bool {
		if len(got) != len(want) {
			return false
		}
		for i := range want {
			if got[i] != want[i] {
				return false
			}
		}
		return true
	}

	plain := create("org-1", "Una tarea cualquiera", "No nombra nada de fuera, sólo proteus-165 en minúsculas.")
	jira := create("org-1", "El cierre diario cuadra", "**Jira:** PROTEUS-165 · **Referencias:** PROTEUS-12")
	other := create("org-2", "La de otro cliente", "También dice PROTEUS-165.")

	if got := named([]string{"org-1"}, "PROTEUS-165"); !same(got, jira) {
		t.Fatalf("PROTEUS-165 en org-1 = %v; quiero sólo %s (ni %s, que lo escribe en minúsculas, ni %s, de otra org)", got, jira, plain, other)
	}
	if got := named([]string{"org-1"}, "PROTEUS-12"); !same(got, jira) {
		t.Errorf("la segunda clave de la misma descripción no se guardó: %v", got)
	}

	// Editar la descripción cambia lo que nombra: la clave vieja se va.
	if err := tasks.UpdateTask(jira, map[string]any{"description": "**Jira:** PROTEUS-200"}); err != nil {
		t.Fatal(err)
	}
	if got := named([]string{"org-1"}, "PROTEUS-165"); len(got) != 0 {
		t.Errorf("la descripción ya no nombra PROTEUS-165 y sigue saliendo: %v", got)
	}
	if got := named([]string{"org-1"}, "PROTEUS-200"); !same(got, jira) {
		t.Errorf("la clave nueva de la descripción no se guardó: %v", got)
	}
	// El título también cuenta.
	if err := tasks.UpdateTask(plain, map[string]any{"title": "PROTEUS-300 · una tarea cualquiera"}); err != nil {
		t.Fatal(err)
	}
	if got := named([]string{"org-1"}, "PROTEUS-300"); !same(got, plain) {
		t.Errorf("una clave en el título no se guardó: %v", got)
	}

	// El tablero la enseña en la tarjeta.
	cards, err := tasks.Board("l-back")
	if err != nil {
		t.Fatal(err)
	}
	keys := map[string][]string{}
	for _, c := range cards {
		keys[c.ID] = c.Keys
	}
	if !same(keys[jira], "PROTEUS-200") || !same(keys[plain], "PROTEUS-300") {
		t.Errorf("claves en las tarjetas = %v", keys)
	}

	// Una archivada no es a donde lleva la clave.
	db.Exec(`UPDATE items SET archived_at = now() WHERE id = ?`, jira)
	if got := named([]string{"org-1"}, "PROTEUS-200"); len(got) != 0 {
		t.Errorf("una tarea archivada sigue saliendo por su clave: %v", got)
	}
}

// Sólo es una clave de fuera lo que va entero en mayúsculas: la ref de cac
// sigue su camino. Rotura que atrapa: aceptar la ref en minúsculas, o un texto
// que sólo contiene una clave.
func TestOnlyAnUppercaseKeyIsATrackerKey(t *testing.T) {
	for s, want := range map[string]bool{
		"PROTEUS-165": true, "CC-7": true, "proteus-165": false, "Proteus-165": false,
		"ver PROTEUS-165": false, "PROTEUS-165 hoy": false, "PROTEUS-": false, "": false,
	} {
		if got := domain.IsExternalKey(s); got != want {
			t.Errorf("IsExternalKey(%q) = %v, quiero %v", s, got, want)
		}
	}
}
