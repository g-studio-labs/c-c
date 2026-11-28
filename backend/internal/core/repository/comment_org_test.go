package repository

import (
	"database/sql"
	"testing"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

func commentOrg(t *testing.T, db *gorm.DB, id string) *string {
	t.Helper()
	var org sql.NullString
	if err := db.Raw(`SELECT org_id FROM item_comments WHERE id = ?`, id).Row().Scan(&org); err != nil {
		t.Fatalf("el comentario %s: %v", id, err)
	}
	if !org.Valid {
		return nil
	}
	return &org.String
}

func wantCommentOrg(t *testing.T, db *gorm.DB, id, want string) {
	t.Helper()
	if got := commentOrg(t, db, id); got == nil || *got != want {
		t.Errorf("el comentario %s tiene org %v, y su tarea es de %s", id, got, want)
	}
}

// Un comentario tiene la org de su tarea, lo escriba quien lo escriba
// (command-and-control-570): la búsqueda filtra por ella, y uno sin org sería
// invisible sin que nada fallara.
//
// Roturas que atrapa: que el disparador no rellene al insertar (por GORM, por
// SQL crudo como la migración de items, o con el ON CONFLICT de Basecamp y
// GitHub), que no siga a una tarea que cambia de org, que la columna se pueda
// escribir a mano con otra org, que el relleno de lo que ya había no corra o
// no sea idempotente, y que el índice no quede montado y válido.
func TestACommentTakesTheOrganizationOfItsTask(t *testing.T) {
	db := searchFTSDB(t)
	if err := EnsureGitHubIndexes(db); err != nil {
		t.Fatal(err)
	}
	ftsTask(t, db, "t1", "org-1", "Una", "")
	ftsTask(t, db, "t2", "org-2", "Otra", "")

	ftsComment(t, db, "c-gorm", "t1", "por la app")
	wantCommentOrg(t, db, "c-gorm", "org-1")

	if err := db.Exec(`INSERT INTO item_comments (id, item_id, kind, body, created_at, updated_at)
		VALUES ('c-raw', 't2', 'user', 'por SQL', now(), now())`).Error; err != nil {
		t.Fatal(err)
	}
	wantCommentOrg(t, db, "c-raw", "org-2")

	sourced := &domain.ItemComment{ItemID: "t2", Kind: "system", SourceKey: "bc:comment:1", Body: "desde Basecamp"}
	sourced.ID = "c-basecamp"
	if ok, err := NewBasecampRepository(db).AddSourcedComment(sourced); err != nil || !ok {
		t.Fatalf("no se escribió el de Basecamp: %v %v", ok, err)
	}
	wantCommentOrg(t, db, "c-basecamp", "org-2")

	// Escrito a mano con otra org, el disparador la devuelve a la de la tarea.
	if err := db.Exec(`UPDATE item_comments SET org_id = 'org-2' WHERE id = 'c-gorm'`).Error; err != nil {
		t.Fatal(err)
	}
	wantCommentOrg(t, db, "c-gorm", "org-1")

	// Una tarea que cambia de org se lleva su hilo.
	if err := db.Exec(`UPDATE items SET org_id = 'org-3' WHERE id = 't1'`).Error; err != nil {
		t.Fatal(err)
	}
	wantCommentOrg(t, db, "c-gorm", "org-3")

	// Lo que había antes de la columna: el relleno al arrancar, y dos veces.
	if err := db.Exec(`ALTER TABLE item_comments DISABLE TRIGGER trg_item_comments_org_id`).Error; err != nil {
		t.Fatal(err)
	}
	if err := db.Exec(`UPDATE item_comments SET org_id = NULL`).Error; err != nil {
		t.Fatal(err)
	}
	if err := db.Exec(`ALTER TABLE item_comments ENABLE TRIGGER trg_item_comments_org_id`).Error; err != nil {
		t.Fatal(err)
	}
	// Y lo que tiene producción de antes: el GIN entero de los comentarios, y
	// un índice nuevo que se quedó a medias en un arranque (inválido).
	if err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_item_comments_search ON item_comments USING GIN (search)`).Error; err != nil {
		t.Fatal(err)
	}
	if err := db.Exec(`UPDATE pg_index SET indisvalid = false WHERE indexrelid IN
		(SELECT oid FROM pg_class WHERE relname IN ('idx_item_comments_org_search', 'idx_item_comments_live_search'))`).Error; err != nil {
		t.Fatalf("marcar el índice como inválido pide superusuario: %v", err)
	}
	for i := 0; i < 2; i++ {
		if err := EnsureSearchIndexes(db); err != nil {
			t.Fatalf("la vez %d: %v", i+1, err)
		}
	}
	wantCommentOrg(t, db, "c-gorm", "org-3")
	wantCommentOrg(t, db, "c-raw", "org-2")

	// Sin tarea, sin org: la búsqueda lo ignora, como antes.
	if err := db.Exec(`INSERT INTO item_comments (id, item_id, kind, body, created_at, updated_at)
		VALUES ('c-orphan', 'no-existe', 'user', 'huérfano', now(), now())`).Error; err != nil {
		t.Fatal(err)
	}
	if got := commentOrg(t, db, "c-orphan"); got != nil {
		t.Errorf("un comentario sin tarea salió con org %q", *got)
	}

	var idx []struct {
		Name  string
		Valid bool
	}
	db.Raw(`SELECT c.relname AS name, i.indisvalid AS valid FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
		WHERE c.relname IN ('idx_item_comments_org_search', 'idx_item_comments_live_search', 'idx_item_comments_search')`).Scan(&idx)
	if len(idx) != 1 || !idx[0].Valid || idx[0].Name == "idx_item_comments_search" {
		t.Errorf("tenía que quedar un solo GIN de comentarios, el parcial y válido: %+v", idx)
	}
}
