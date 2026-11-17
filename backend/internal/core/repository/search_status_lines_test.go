package repository

import (
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Lo que el servidor anota en un hilo no hace aparecer su tarea al buscar.
//
// Desde el 17-nov-2026 todo cambio de estado deja «status: x → y» también en
// las tareas internas (command-and-control-383). Si esas líneas se buscaran,
// «resolved» devolvería cada tarea que alguna vez se resolvió: el nombre del
// estado está escrito en su hilo. Lo que viene de fuera con su `source_key` (el
// mensaje de un commit) sí se busca, y lo que escribe una persona también.
//
// Roturas que atrapa: buscar en las líneas de estado (en la consulta de texto
// completo o en la de `LIKE`, por separado); y pasarse y dejar de buscar en las
// líneas de GitHub o en los comentarios de la gente.
func TestServerStatusLinesDoNotBringTheirTaskUpInSearch(t *testing.T) {
	db := searchFTSDB(t)
	for _, id := range []string{"moved", "commit", "said"} {
		ftsTask(t, db, id, "org-1", "Tarea "+id, "")
	}
	line := func(id, item, source, body string) {
		t.Helper()
		c := &domain.ItemComment{ItemID: item, Kind: domain.CommentKindSystem, Visibility: domain.VisibilityInternal, Body: body, SourceKey: source}
		c.ID = id
		if err := db.Create(c).Error; err != nil {
			t.Fatal(err)
		}
	}
	line("c-status", "moved", "", "status: pending → resolved")
	line("c-commit", "commit", "gh:push:abc123", "fix: resolved the leak")
	ftsComment(t, db, "c-said", "said", "ya quedó resolved de mi lado")

	repo := NewSearchRepository(db)
	fts, err := repo.Tasks("resolved", "org-1", 10)
	if err != nil {
		t.Fatal(err)
	}
	like, err := repo.tasksLike("resolved", "org-1", 10)
	if err != nil {
		t.Fatal(err)
	}
	for name, hits := range map[string][]domain.SearchHit{"texto completo": fts, "LIKE": like} {
		got := map[string]bool{}
		for _, id := range hitIDs(hits) {
			got[id] = true
		}
		if got["moved"] {
			t.Errorf("%s: una tarea salió sólo porque su estado se llama así: %v", name, hitIDs(hits))
		}
		if !got["commit"] || !got["said"] {
			t.Errorf("%s: el mensaje de un commit y lo que dice una persona se siguen buscando: %v", name, hitIDs(hits))
		}
	}
}
