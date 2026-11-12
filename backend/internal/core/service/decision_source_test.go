package service

import (
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Una decisión tomada fuera de cac (un comentario de Jira) lleva su fuente, y
// la misma fuente no entra dos veces: quien relee el ticket mañana encuentra
// el mismo comentario, y el registro no se puede podar
// (command-and-control-353).
//
// Roturas que atrapa: escribir otra entrada con la misma fuente; no decir que
// la respuesta es la que ya había; tomar por repetida la de otra organización;
// tratar dos decisiones sin fuente como la misma; no guardar la fuente ni
// quién la dijo.
func TestTheSameOutsideSourceIsRecordedOnce(t *testing.T) {
	db, cleanup := decisionDB(t)
	defer cleanup()
	svc := NewDocService(repository.NewDocRepository(db))
	const comment = "https://tcg.atlassian.net/browse/PROTEUS-165?focusedCommentId=10234"
	record := func(org, list, title, source string) *domain.Decision {
		t.Helper()
		d, err := svc.AddDecision(org, domain.DocOwnerList, list, "u-1", "", "",
			domain.DecisionRequest{Title: title, Origin: "doc", SourceURL: source, SourceAuthor: "Itzel"})
		if err != nil {
			t.Fatal(err)
		}
		return d
	}
	count := func() int64 {
		var n int64
		db.Model(&domain.Decision{}).Count(&n)
		return n
	}

	first := record("org-1", "list-1", "El DSD se paga por parte", comment)
	if first.AlreadyRecorded || first.SourceURL != comment || first.SourceAuthor != "Itzel" {
		t.Fatalf("la primera = %+v; quiero una entrada nueva con su fuente y quién la dijo", first)
	}
	again := record("org-1", "list-1", "El DSD se paga por parte (releída)", comment)
	if again.ID != first.ID || !again.AlreadyRecorded {
		t.Errorf("la misma fuente dio otra entrada: %s (ya apuntada: %v), quiero %s", again.ID, again.AlreadyRecorded, first.ID)
	}
	if n := count(); n != 1 {
		t.Errorf("hay %d decisiones tras releer la misma fuente; quiero 1", n)
	}

	// La misma URL en otra organización es otra decisión.
	elsewhere := record("org-2", "list-9", "De otro cliente", comment)
	if elsewhere.ID == first.ID || elsewhere.AlreadyRecorded {
		t.Errorf("la fuente de otra organización se tomó por repetida")
	}
	// Sin fuente no hay con qué comparar: dos decisiones son dos.
	a, b := record("org-1", "list-1", "Una", ""), record("org-1", "list-1", "Otra", "")
	if a.ID == b.ID || b.AlreadyRecorded {
		t.Errorf("dos decisiones sin fuente se tomaron por la misma")
	}
}
