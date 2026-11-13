package service

import (
	"errors"
	"fmt"
	"testing"
	"time"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

func sourcesDB(t *testing.T) *gorm.DB {
	t.Helper()
	db := incidentDB(t)
	if err := db.AutoMigrate(&domain.GraphLink{}, &domain.SpaceKeyAlias{}, &domain.ItemComment{}, &domain.DocPageVersion{},
		&domain.SpaceSource{}); err != nil {
		t.Fatal(err)
	}
	return db
}

// Una regla (`R-012`) citada en un texto del espacio lleva a la sección del
// registro que **es** esa regla (command-and-control-354, decisión 0caaffbe).
//
// Roturas que atrapa: no enlazar la regla; enlazarla sin registro apuntado o
// en otro espacio; sacar `R-0050` de `PR-0050`; dar por regla la sección que
// sólo la cita; que el registro se nombre a sí mismo; no enlazar lo que ya la
// citaba cuando el registro se apunta, ni cuando la sección llega después; no
// quitar el enlace al dejar de citarla o al quitar el registro; no encontrar
// la sección por el nombre de la regla, enseñársela a otra organización, o
// elegir una al azar cuando dos registros la tienen.
func TestARuleNamedInATextLeadsToItsSection(t *testing.T) {
	db := sourcesDB(t)
	tasks := repository.NewTaskRepository(db)
	docs := repository.NewDocRepository(db)
	sources := repository.NewSpaceSourceRepository(db)
	svc := NewSpaceSourceService(sources)
	g := NewGraphService(repository.NewGraphRepository(db), tasks).WithRules(sources)

	// Otro espacio de la misma organización, sin registro.
	other := &domain.TaskSpace{OrgID: "org-1", Name: "sin registro", Key: "sr", Rank: "0.6"}
	other.ID = "sp-3"
	mustCreate(t, db, other)
	task := func(id, space string, seq int) {
		it := &domain.Item{OrgID: "org-1", SpaceID: space, ListID: "l-back", Title: "tarea " + id, Status: domain.ReportPending, Seq: seq}
		it.ID = id
		mustCreate(t, db, it)
	}
	task("t-cita", "sp-1", 20)
	task("t-fuera", "sp-3", 1)

	page, err := docs.CreatePage("org-1", domain.DocOwnerList, "l-back", domain.CreateDocPageRequest{Title: "Registro de reglas",
		Body: "## R-012 · Los pagos van por el banco\n\nReemplaza a R-005.\n\n## R-005 · La vieja\n\nYa no.\n\n## Notas sobre R-030\n\nPendiente.\n"}, "u-1")
	if err != nil {
		t.Fatal(err)
	}
	section := func(heading string) string {
		t.Helper()
		var id string
		db.Raw(`SELECT id FROM doc_sections WHERE page_id = ? AND heading LIKE ? AND removed_at IS NULL`, page.ID, heading+"%").Scan(&id)
		if id == "" {
			t.Fatalf("la sección «%s» no tiene identidad", heading)
		}
		return id
	}
	r012 := section("R-012")

	// La cita llega antes de que nadie apunte el registro.
	cita := &domain.TaskComment{ItemID: "t-cita", Body: "Aplica R-012 (ver PR-005), y también R-030 y R-999."}
	if err := tasks.CreateComment(cita); err != nil {
		t.Fatal(err)
	}
	if err := tasks.CreateComment(&domain.TaskComment{ItemID: "t-fuera", Body: "Aquí R-012 no es de nadie."}); err != nil {
		t.Fatal(err)
	}
	mentions := func(kind domain.EntityKind, id string) map[string]bool {
		t.Helper()
		got, err := g.Related(kind, id, []string{"org-1"}, false)
		if err != nil {
			t.Fatalf("%s %s: %v", kind, id, err)
		}
		out := map[string]bool{}
		for _, n := range got.Neighbors {
			if n.Rel == domain.RelMentions && (n.Kind == domain.EntitySection || kind == domain.EntitySection) {
				out[fmt.Sprintf("%s %s", n.Dir, n.ID)] = true
			}
		}
		return out
	}
	only := func(what string, got map[string]bool, want ...string) {
		t.Helper()
		if len(got) != len(want) {
			t.Errorf("%s: se esperaba %v y hay %v", what, want, got)
			return
		}
		for _, w := range want {
			if !got[w] {
				t.Errorf("%s: falta «%s» en %v", what, w, got)
			}
		}
	}
	only("sin registro apuntado", mentions(domain.EntityTask, "t-cita"))

	sp1, _ := tasks.FindSpace("sp-1")
	rules := domain.SpaceSourceInput{Role: domain.SourceRules, System: domain.SystemCac, Ref: page.ID, Pattern: `R-\d+`}
	if _, err := svc.Replace(sp1, domain.SpaceSourcesRequest{Sources: []domain.SpaceSourceInput{rules}}); err != nil {
		t.Fatal(err)
	}
	// Lo que ya la citaba se enlaza al apuntar el registro. Sólo R-012: de
	// PR-005 no sale R-005, R-030 sólo se cita en un título y R-999 no existe.
	only("al apuntar el registro", mentions(domain.EntityTask, "t-cita"), "out "+r012)
	// En otro espacio la misma cita, escrita con el registro ya apuntado, no
	// es de nadie: el registro es del espacio que lo apunta.
	if err := tasks.CreateComment(&domain.TaskComment{ItemID: "t-fuera", Body: "Y otra vez R-012."}); err != nil {
		t.Fatal(err)
	}
	only("otro espacio", mentions(domain.EntityTask, "t-fuera"))
	only("el registro a sí mismo", mentions(domain.EntityPage, page.ID))
	only("la regla, desde su lado", mentions(domain.EntitySection, r012), "in t-cita")

	// Por el nombre de la regla, que es lo que sabe quien la cita.
	byName, err := g.Related(KindRule, "R-012", []string{"org-1"}, false)
	if err != nil || byName.Kind != domain.EntitySection || byName.ID != r012 || byName.Title != "R-012 · Los pagos van por el banco" {
		t.Fatalf("por su nombre: %+v %v", byName, err)
	}
	// Otra organización tiene su propio registro con su propia R-012: cada
	// una encuentra la suya, y la de la otra ni la ve ni le estorba.
	ajena, err := docs.CreatePage("org-2", domain.DocOwnerList, "l-otro",
		domain.CreateDocPageRequest{Title: "Registro ajeno", Body: "## R-012 · La de otros\n\nx\n"}, "u-2")
	if err != nil {
		t.Fatal(err)
	}
	sp2, _ := tasks.FindSpace("sp-2")
	rules.Ref = ajena.ID
	if _, err := svc.Replace(sp2, domain.SpaceSourcesRequest{Sources: []domain.SpaceSourceInput{rules}}); err != nil {
		t.Fatal(err)
	}
	if mine, err := g.Related(KindRule, "R-012", []string{"org-1"}, false); err != nil || mine.ID != r012 {
		t.Errorf("la regla de otra organización estorba a la propia: %+v %v", mine, err)
	}
	if theirs, err := g.Related(KindRule, "R-012", []string{"org-2"}, false); err != nil || theirs.Title != "R-012 · La de otros" {
		t.Errorf("la otra organización no encuentra la suya: %+v %v", theirs, err)
	}
	// Quien ve las dos (un superadmin) no recibe una al azar.
	if _, err := g.Related(KindRule, "R-012", nil, true); !errors.Is(err, ErrGraphSeveral) {
		t.Errorf("dos registros con la misma regla y sale una: %v", err)
	}
	if _, err := g.Related(KindRule, "R-005", []string{"org-2"}, false); !errors.Is(err, ErrGraphNotFound) {
		t.Errorf("otra organización encuentra una regla que no es suya: %v", err)
	}
	if _, err := g.Related(KindRule, "R-030", []string{"org-1"}, false); !errors.Is(err, ErrGraphNotFound) {
		t.Errorf("una sección que sólo cita la regla pasa por ella: %v", err)
	}

	// La sección de R-999 llega después de la cita.
	if err := docs.AppendPage(page.ID, "\n## R-999 · La nueva\n\nRecién dictada.\n", "u-1"); err != nil {
		t.Fatal(err)
	}
	r999 := section("R-999")
	only("al llegar la sección", mentions(domain.EntityTask, "t-cita"), "out "+r012, "out "+r999)

	// Y si el registro se reescribe sin ella, deja de ser una regla.
	original := "## R-012 · Los pagos van por el banco\n\nReemplaza a R-005.\n\n## R-005 · La vieja\n\nYa no.\n"
	if err := docs.SavePageRenaming(page, nil, &original, "u-1", nil); err != nil {
		t.Fatal(err)
	}
	only("al quitar la sección", mentions(domain.EntityTask, "t-cita"), "out "+r012)

	// Citar otra cambia el enlace.
	if err := tasks.UpdateComment(cita.ID, "Ya sólo R-005."); err != nil {
		t.Fatal(err)
	}
	only("al citar otra", mentions(domain.EntityTask, "t-cita"), "out "+section("R-005"))

	// Sin registro, la regla vuelve a ser texto.
	if _, err := svc.Replace(sp1, domain.SpaceSourcesRequest{}); err != nil {
		t.Fatal(err)
	}
	only("al quitar el registro", mentions(domain.EntityTask, "t-cita"))
}

// Las fuentes de un espacio se guardan como lista entera, y volver a mandarla
// no le quita a ninguna hasta cuándo se leyó (command-and-control-354).
//
// Roturas que atrapa: perder el cursor al reenviar la lista; no quitar la que
// ya no viene; aceptar un papel o un sistema inventados, una fuente sin ref,
// un registro fuera de cac, un patrón vacío o que casa con la nada, una página
// de otra organización, la misma fuente dos veces; mover el cursor al futuro,
// o el de una fuente de otro espacio.
func TestTheSourcesOfASpaceKeepTheirCursor(t *testing.T) {
	db := sourcesDB(t)
	tasks := repository.NewTaskRepository(db)
	svc := NewSpaceSourceService(repository.NewSpaceSourceRepository(db))
	sp1, _ := tasks.FindSpace("sp-1")
	sp2, _ := tasks.FindSpace("sp-2")

	jira := domain.SpaceSourceInput{Role: domain.SourceTracker, System: domain.SystemJira, Ref: " PROTEUS ", Pattern: `R-\d+`}
	notion := domain.SpaceSourceInput{Role: domain.SourceDecisions, System: domain.SystemNotion, Ref: "https://notion.so/decisiones"}
	// La copia del registro que lee el cliente vive fuera: no es el registro,
	// así que ni tiene que estar en cac ni lleva patrón.
	copia := domain.SpaceSourceInput{Role: domain.SourceRulesCopy, System: domain.SystemNotion, Ref: "https://notion.so/reglas"}
	got, err := svc.Replace(sp1, domain.SpaceSourcesRequest{Sources: []domain.SpaceSourceInput{jira, notion, copia}})
	if err != nil || len(got) != 3 || got[2].Role != domain.SourceRulesCopy {
		t.Fatalf("guardar: %+v %v", got, err)
	}
	if got[0].Ref != "PROTEUS" || got[0].Pattern != "" || got[1].Role != domain.SourceDecisions || got[0].LastSyncAt != nil {
		t.Fatalf("cómo quedaron: %+v", got)
	}
	at := time.Now().Add(-time.Hour).UTC().Truncate(time.Second)
	moved, err := svc.MarkSynced("sp-1", got[0].ID, at)
	if err != nil || moved.LastSyncAt == nil || !moved.LastSyncAt.Equal(at) {
		t.Fatalf("mover el cursor: %+v %v", moved, err)
	}

	// La misma lista otra vez, con una nota nueva y sin ids: el cursor sigue.
	jira.Note = "el tablero de Diego e Itzel"
	again, err := svc.Replace(sp1, domain.SpaceSourcesRequest{Sources: []domain.SpaceSourceInput{jira}})
	if err != nil || len(again) != 1 {
		t.Fatalf("reenviar: %+v %v", again, err)
	}
	if again[0].ID != got[0].ID || again[0].LastSyncAt == nil || !again[0].LastSyncAt.Equal(at) || again[0].Note != jira.Note {
		t.Errorf("reenviar la lista perdió algo: %+v", again[0])
	}

	// El cursor no va al futuro ni es de otro espacio.
	if _, err := svc.MarkSynced("sp-1", got[0].ID, time.Now().Add(time.Hour)); !errors.Is(err, domain.ErrSourceSyncFuture) {
		t.Errorf("un cursor en el futuro: %v", err)
	}
	if _, err := svc.MarkSynced("sp-2", got[0].ID, at); !errors.Is(err, domain.ErrSourceNotFound) {
		t.Errorf("el cursor de una fuente de otro espacio: %v", err)
	}

	// La página de otra organización no puede ser el registro de ésta.
	ajena, err := repository.NewDocRepository(db).CreatePage("org-2", domain.DocOwnerList, "l-otro",
		domain.CreateDocPageRequest{Title: "Registro ajeno", Body: "## R-001 · x\n"}, "u-2")
	if err != nil {
		t.Fatal(err)
	}
	rules := func(system domain.SourceSystem, ref, pattern string) domain.SpaceSourceInput {
		return domain.SpaceSourceInput{Role: domain.SourceRules, System: system, Ref: ref, Pattern: pattern}
	}
	for _, c := range []struct {
		name string
		in   []domain.SpaceSourceInput
		want error
	}{
		{"papel inventado", []domain.SpaceSourceInput{{Role: "wiki", System: domain.SystemJira, Ref: "X"}}, domain.ErrSourceRole},
		{"sistema inventado", []domain.SpaceSourceInput{{Role: domain.SourceTracker, System: "trello", Ref: "X"}}, domain.ErrSourceSystem},
		{"sin ref", []domain.SpaceSourceInput{{Role: domain.SourceTracker, System: domain.SystemJira, Ref: "  "}}, domain.ErrSourceRef},
		{"registro fuera de cac", []domain.SpaceSourceInput{rules(domain.SystemNotion, "https://notion.so/reglas", `R-\d+`)}, domain.ErrSourceRulesInCac},
		{"registro sin patrón", []domain.SpaceSourceInput{rules(domain.SystemCac, ajena.ID, "")}, domain.ErrSourcePattern},
		{"patrón que casa con la nada", []domain.SpaceSourceInput{rules(domain.SystemCac, ajena.ID, `\d*`)}, domain.ErrSourcePattern},
		{"patrón que no compila", []domain.SpaceSourceInput{rules(domain.SystemCac, ajena.ID, `R-(\d+`)}, domain.ErrSourcePattern},
		{"página de otra organización", []domain.SpaceSourceInput{rules(domain.SystemCac, ajena.ID, `R-\d+`)}, domain.ErrSourcePage},
		{"la misma dos veces", []domain.SpaceSourceInput{jira, jira}, domain.ErrSourceRepeated},
		{"un id que no es de aquí", []domain.SpaceSourceInput{{ID: "no-existe", Role: domain.SourceTracker, System: domain.SystemJira, Ref: "X"}}, domain.ErrSourceNotFound},
	} {
		if _, err := svc.Replace(sp1, domain.SpaceSourcesRequest{Sources: c.in}); !errors.Is(err, c.want) {
			t.Errorf("%s: se esperaba %v y salió %v", c.name, c.want, err)
		}
	}
	// Ninguna de las malas tocó lo que había, y en su organización la página sí vale.
	if left, _ := svc.List("sp-1"); len(left) != 1 || left[0].ID != got[0].ID {
		t.Errorf("una lista rechazada cambió las fuentes: %+v", left)
	}
	if _, err := svc.Replace(sp2, domain.SpaceSourcesRequest{Sources: []domain.SpaceSourceInput{rules(domain.SystemCac, ajena.ID, `R-\d+`)}}); err != nil {
		t.Errorf("el registro de su propia organización: %v", err)
	}
	many := make([]domain.SpaceSourceInput, domain.SpaceSourcesMax+1)
	for i := range many {
		many[i] = domain.SpaceSourceInput{Role: domain.SourceTracker, System: domain.SystemJira, Ref: fmt.Sprintf("K%d", i)}
	}
	if _, err := svc.Replace(sp1, domain.SpaceSourcesRequest{Sources: many}); !errors.Is(err, domain.ErrSourceTooMany) {
		t.Errorf("más fuentes de las que caben: %v", err)
	}
}
