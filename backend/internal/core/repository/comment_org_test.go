package repository

import (
	"database/sql"
	"reflect"
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
		if _, err := BackfillCommentOrgs(db, commentOrgBatch); err != nil {
			t.Fatalf("el relleno, la vez %d: %v", i+1, err)
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

func withoutTriggers(t *testing.T, db *gorm.DB, stmt string) {
	t.Helper()
	// Como una restauración con `--disable-triggers`: el disparador no corre.
	if err := db.Transaction(func(tx *gorm.DB) error {
		if err := tx.Exec(`SET LOCAL session_replication_role = replica`).Error; err != nil {
			return err
		}
		return tx.Exec(stmt).Error
	}); err != nil {
		t.Fatal(err)
	}
}

// El relleno de los comentarios de antes de la columna va por tandas, hasta
// que no queda ninguno, y se puede correr dos veces: en producción la tabla
// entera en una sentencia podía pasar del healthcheck del arranque.
//
// Roturas que atrapa: parar tras la primera tanda, no contar lo rellenado,
// tocar lo que ya tenía org, o parar antes de tiempo con una tanda de
// comentarios sin tarea.
func TestCommentOrgsAreBackfilledInBatchesUntilNoneIsLeft(t *testing.T) {
	db := searchFTSDB(t)
	// Los huérfanos, primero en la tabla: la primera tanda los encuentra a
	// ellos antes que a nadie, y no tienen org que recibir.
	for _, id := range []string{"c-orphan", "c-orphan-2"} {
		withoutTriggers(t, db, `INSERT INTO item_comments (id, item_id, kind, body, created_at, updated_at)
			VALUES ('`+id+`', 'no-existe', 'user', 'huérfano', now(), now())`)
	}
	ftsTask(t, db, "t1", "org-1", "Una", "")
	ftsTask(t, db, "t2", "org-2", "Otra", "")
	for _, c := range [][2]string{{"c1", "t1"}, {"c2", "t1"}, {"c3", "t2"}, {"c4", "t2"}, {"c5", "t1"}} {
		ftsComment(t, db, c[0], c[1], "algo")
	}
	withoutTriggers(t, db, `UPDATE item_comments SET org_id = NULL WHERE id <> 'c5'`)
	// Uno que ya tenía org (aunque fuera vieja) no es del relleno.
	withoutTriggers(t, db, `UPDATE item_comments SET org_id = 'org-viejo' WHERE id = 'c5'`)

	commentOrgsBackfilled.Store(false)
	t.Cleanup(func() { commentOrgsBackfilled.Store(true) })
	n, err := BackfillCommentOrgs(db, 2)
	if err != nil {
		t.Fatal(err)
	}
	if !commentOrgsBackfilled.Load() {
		t.Error("al terminar el relleno la marca tenía que quedar en true; sin ella la búsqueda se queda en modo lento para siempre")
	}
	if n != 4 {
		t.Errorf("con tandas de 2 tenía que rellenar los 4 sin org, y dice %d", n)
	}
	for id, want := range map[string]string{"c1": "org-1", "c2": "org-1", "c3": "org-2", "c4": "org-2", "c5": "org-viejo"} {
		wantCommentOrg(t, db, id, want)
	}
	if got := commentOrg(t, db, "c-orphan"); got != nil {
		t.Errorf("un comentario sin tarea salió con org %q", *got)
	}
	if n, err := BackfillCommentOrgs(db, 2); err != nil || n != 0 {
		t.Errorf("la segunda vez no tenía nada que hacer: %d %v", n, err)
	}
}

// La valla de la org en el hilo es la de la tarea, no sólo la que lleva el
// comentario: ésa la pone un disparador, y con los disparadores apagados (una
// restauración) se queda vieja. Y mientras el relleno no ha terminado, un
// comentario que aún no tiene org sigue encontrando su tarea.
//
// Roturas que atrapa: quitar `t.org_id = ?` del join del hilo, y no aceptar
// los comentarios sin org mientras el relleno no ha terminado.
func TestAThreadIsFencedByItsTaskOrganization(t *testing.T) {
	db := searchFTSDB(t)
	ftsTask(t, db, "t-mine", "org-1", "Lunes", "")
	ftsTask(t, db, "t-theirs", "org-2", "Martes", "")
	ftsComment(t, db, "c-mine", "t-mine", "despliegue")
	ftsComment(t, db, "c-theirs", "t-theirs", "despliegue")
	// Se restauró con la org equivocada: el comentario ajeno dice ser de org-1.
	withoutTriggers(t, db, `UPDATE item_comments SET org_id = 'org-1' WHERE id = 'c-theirs'`)
	r := NewSearchRepository(db)

	hits, err := r.Tasks("despliegue", "org-1", 20)
	if err != nil {
		t.Fatal(err)
	}
	if got := hitIDs(hits); !reflect.DeepEqual(got, []string{"t-mine"}) {
		t.Errorf("la tarea de otra org salió por un comentario con la org vieja: %v", got)
	}

	// A medias del relleno: el comentario aún no tiene org.
	withoutTriggers(t, db, `UPDATE item_comments SET org_id = NULL WHERE id = 'c-mine'`)
	commentOrgsBackfilled.Store(false)
	t.Cleanup(func() { commentOrgsBackfilled.Store(true) })
	hits, err = r.Tasks("despliegue", "org-1", 20)
	if err != nil {
		t.Fatal(err)
	}
	if got := hitIDs(hits); !reflect.DeepEqual(got, []string{"t-mine"}) {
		t.Errorf("con el relleno a medias, el hilo sin org tenía que seguir encontrando su tarea: %v", got)
	}
}
