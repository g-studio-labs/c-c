package repository

import (
	"reflect"
	"testing"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// El orden de la búsqueda, escrito tal como salía el 27-nov-2026, antes de
// que command-and-control-570 la acotara para que bajara de 1,5–3 s. Acotar
// (quedarse con unas candidatas antes de puntuar, filtrar los comentarios por
// la org antes del join) sólo puede cambiar el tiempo, no lo que sale ni en
// qué orden. Esta prueba es lo que lo garantiza: se escribió y pasó sobre la
// consulta de antes.
//
// Si un cambio la pone en rojo, el cambio movió el ranking: o se arregla el
// cambio, o se decide moverlo y se reescribe el orden aquí, diciendo por qué.

func goldenAge(t *testing.T, db *gorm.DB, table, id string, daysAgo int) {
	t.Helper()
	if err := db.Exec(`UPDATE `+table+` SET updated_at = now() - make_interval(days => ?) WHERE id = ?`, daysAgo, id).Error; err != nil {
		t.Fatal(err)
	}
}

// Roturas que atrapa: puntuar con otra cosa que ts_rank_cd, dejar de poner
// primero lo que casa en el título, perder el desempate por fecha, contar un
// comentario borrado, uno de la charla del servidor o el de otra org, o caer
// sin decirlo a la búsqueda por LIKE (que no da fragmento ni este orden).
func TestTaskRankingStaysAsItWas(t *testing.T) {
	db := searchFTSDB(t)
	type task struct {
		id, org, title, desc string
		days                 int
	}
	for _, x := range []task{
		{"t-title-old", "org-1", "Despliegue de koa", "", 10},
		{"t-title-new", "org-1", "Despliegue nocturno", "El despliegue nocturno, otro despliegue.", 1},
		{"t-desc-many", "org-1", "Mantenimiento", "despliegue despliegue despliegue despliegue despliegue", 2},
		{"t-desc-one", "org-1", "Cosas", "Un despliegue más.", 3},
		{"t-desc-one-b", "org-1", "Otras cosas", "Un despliegue más.", 5},
		{"t-comments", "org-1", "Lunes", "Nada que ver.", 4},
		{"t-both", "org-1", "Martes", "Hubo un despliegue.", 6},
		{"t-sub", "org-1", "Subtarea", "Probar el despliegue nocturno.", 7},
		{"t-chatter", "org-1", "Miércoles", "", 1},
		{"t-deleted-comment", "org-1", "Jueves", "", 1},
		{"t-other-org", "org-2", "Despliegue ajeno", "despliegue despliegue", 1},
		{"t-host-other", "org-2", "Ajena con hilo", "", 1},
	} {
		ftsTask(t, db, x.id, x.org, x.title, x.desc)
	}
	if err := db.Exec(`UPDATE items SET parent_id = 't-title-old' WHERE id = 't-sub'`).Error; err != nil {
		t.Fatal(err)
	}
	ftsComment(t, db, "c1", "t-comments", "El despliegue se hizo a mano.")
	ftsComment(t, db, "c2", "t-comments", "Otro despliegue, despliegue y despliegue.")
	ftsComment(t, db, "c3", "t-both", "despliegue despliegue despliegue despliegue despliegue despliegue")
	ftsComment(t, db, "c4", "t-deleted-comment", "despliegue retirado")
	if err := db.Delete(&domain.ItemComment{}, "id = ?", "c4").Error; err != nil {
		t.Fatal(err)
	}
	chatter := &domain.ItemComment{ItemID: "t-chatter", Kind: "system", Body: "status: despliegue → in_progress"}
	chatter.ID = "c5"
	if err := db.Create(chatter).Error; err != nil {
		t.Fatal(err)
	}
	ftsComment(t, db, "c6", "t-host-other", "despliegue en el hilo de otra org")
	for _, x := range []struct {
		id   string
		days int
	}{
		{"t-title-old", 10}, {"t-title-new", 1}, {"t-desc-many", 2}, {"t-desc-one", 3}, {"t-desc-one-b", 5},
		{"t-comments", 4}, {"t-both", 6}, {"t-sub", 7}, {"t-chatter", 1}, {"t-deleted-comment", 1},
	} {
		goldenAge(t, db, "items", x.id, x.days)
	}
	r := NewSearchRepository(db)

	cases := []struct {
		query string
		limit int
		want  []string
	}{
		{"despliegue", 20, []string{
			"t-title-new", "t-title-old",
			"t-desc-many", "t-both", "t-comments", "t-desc-one", "t-desc-one-b", "t-sub",
		}},
		{"despliegue", 3, []string{"t-title-new", "t-title-old", "t-desc-many"}},
		{"despliegue nocturno", 20, []string{"t-title-new", "t-sub"}},
	}
	for _, c := range cases {
		hits, err := r.Tasks(c.query, "org-1", c.limit)
		if err != nil {
			t.Fatal(err)
		}
		if got := hitIDs(hits); !reflect.DeepEqual(got, c.want) {
			t.Errorf("%q (límite %d): el orden cambió: got %v, want %v", c.query, c.limit, got, c.want)
		}
		for _, h := range hits {
			switch h.ID {
			case "t-desc-many", "t-desc-one", "t-title-new":
				if h.Snippet == "" {
					t.Errorf("%q: %s casa en la descripción y salió sin fragmento (¿cayó al LIKE?)", c.query, h.ID)
				}
			case "t-comments":
				if h.Snippet != "" {
					t.Errorf("%q: un comentario no da fragmento: %q", c.query, h.Snippet)
				}
			case "t-sub":
				if h.Where == "Lista" {
					t.Errorf("%q: la subtarea perdió de quién cuelga: %q", c.query, h.Where)
				}
			}
		}
	}
}

// Roturas que atrapa: otro orden entre pestañas, páginas, .md y decisiones;
// perder el peso del título de una página; perder el desempate por fecha;
// sacar una página borrada o las migas de una subpágina.
func TestDocRankingStaysAsItWas(t *testing.T) {
	db := searchFTSDB(t)
	must := func(v any) {
		t.Helper()
		if err := db.Create(v).Error; err != nil {
			t.Fatal(err)
		}
	}
	sp := &domain.TaskSpace{OrgID: "org-1", Name: "Espacio", Rank: "0.5"}
	sp.ID = "s"
	must(sp)
	must(&domain.GitHubRepo{BaseModel: domain.BaseModel{ID: "gh-1"}, RepoID: 1, FullName: "dwit/handover", OrgID: "org-1", SpaceID: "s", ShowDocs: true})
	must(&domain.RepoDoc{BaseModel: domain.BaseModel{ID: "rd-1"}, OrgID: "org-1", RepoID: 1, Path: "docs/soltar.md", Title: "Soltar", BlobSha: "x",
		Body: "# Soltar\n\nCómo se corta una release y qué release va antes."})
	must(&domain.Doc{BaseModel: domain.BaseModel{ID: "doc-s"}, OrgID: "org-1", OwnerKind: domain.DocOwnerSpace, OwnerID: "s"})
	must(&domain.Doc{BaseModel: domain.BaseModel{ID: "doc-l"}, OrgID: "org-1", OwnerKind: domain.DocOwnerList, OwnerID: "l"})
	must(&domain.Doc{BaseModel: domain.BaseModel{ID: "doc-x"}, OrgID: "org-2", OwnerKind: domain.DocOwnerList, OwnerID: "l-x"})
	must(&domain.DocTab{BaseModel: domain.BaseModel{ID: "tab-s"}, DocID: "doc-s", Key: "runbook", Body: "## Release\n\nUna release sale después del backend."})
	must(&domain.DocTab{BaseModel: domain.BaseModel{ID: "tab-l"}, DocID: "doc-l", Key: "overview", Body: "Habla de una release, una vez."})
	must(&domain.DocPage{BaseModel: domain.BaseModel{ID: "p-title"}, DocID: "doc-l", OrgID: "org-1", Title: "Release", Rank: "0.5", Body: "Lo que hay que saber."})
	parent := "p-title"
	must(&domain.DocPage{BaseModel: domain.BaseModel{ID: "p-child"}, DocID: "doc-l", OrgID: "org-1", ParentID: &parent, Title: "Detalle", Rank: "0.5",
		Body: "release release release"})
	must(&domain.DocPage{BaseModel: domain.BaseModel{ID: "p-once-new"}, DocID: "doc-s", OrgID: "org-1", Title: "Notas", Rank: "0.6", Body: "Una release."})
	must(&domain.DocPage{BaseModel: domain.BaseModel{ID: "p-once-old"}, DocID: "doc-s", OrgID: "org-1", Title: "Más notas", Rank: "0.7", Body: "Una release."})
	must(&domain.DocPage{BaseModel: domain.BaseModel{ID: "p-deleted"}, DocID: "doc-s", OrgID: "org-1", Title: "Release borrada", Rank: "0.8", Body: "release"})
	if err := db.Delete(&domain.DocPage{}, "id = ?", "p-deleted").Error; err != nil {
		t.Fatal(err)
	}
	must(&domain.DocPage{BaseModel: domain.BaseModel{ID: "p-other-org"}, DocID: "doc-x", OrgID: "org-2", Title: "Release", Rank: "0.5", Body: "release"})
	must(&domain.Decision{BaseModel: domain.BaseModel{ID: "dec-1"}, DocID: "doc-l", Title: "Las release se juntan", Body: "Una por arreglo gastaba números.", Origin: "doc"})
	for _, x := range []struct {
		table, id string
		days      int
	}{
		{"doc_tabs", "tab-s", 3}, {"doc_tabs", "tab-l", 1}, {"doc_pages", "p-title", 5}, {"doc_pages", "p-child", 2},
		{"doc_pages", "p-once-new", 1}, {"doc_pages", "p-once-old", 9}, {"repo_docs", "rd-1", 4},
	} {
		goldenAge(t, db, x.table, x.id, x.days)
	}
	if err := db.Exec(`UPDATE decisions SET decided_at = now() - interval '6 days' WHERE id = 'dec-1'`).Error; err != nil {
		t.Fatal(err)
	}
	r := NewSearchRepository(db)

	key := func(h domain.SearchHit) string {
		if h.Kind == domain.SearchDoc && h.PageID == "" {
			return "tab:" + h.Tab + "@" + h.OwnerID
		}
		return h.ID
	}
	cases := []struct {
		query string
		limit int
		want  []string
	}{
		{"release", 20, []string{"p-child", "p-title", "dec-1", "tab:runbook@s", "rd-1", "p-once-new", "tab:overview@l", "p-once-old"}},
		{"release", 3, []string{"p-child", "p-title", "dec-1"}},
	}
	for _, c := range cases {
		hits, err := r.Docs(c.query, "org-1", c.limit)
		if err != nil {
			t.Fatal(err)
		}
		got := []string{}
		for _, h := range hits {
			got = append(got, key(h))
			if h.Snippet == "" && h.Kind != domain.SearchDoc {
				t.Errorf("%q: %s salió sin fragmento (¿cayó al LIKE?)", c.query, h.ID)
			}
			if h.ID == "p-child" && h.Where != "Lista › Release" {
				t.Errorf("%q: las migas de la subpágina cambiaron: %q", c.query, h.Where)
			}
		}
		if !reflect.DeepEqual(got, c.want) {
			t.Errorf("%q (límite %d): el orden cambió: got %v, want %v", c.query, c.limit, got, c.want)
		}
	}
}

