package repository

import (
	"fmt"
	"sync/atomic"

	"gorm.io/gorm"

	lg "github.com/guz-studio/cac/backend/internal/core/logger"
)

/*
ensureCommentOrgs le da a cada comentario la org de su tarea
(command-and-control-570), para que la búsqueda filtre la org **antes** de
cruzar con `items`. Sin ella, el GIN de los comentarios devolvía los de todas
las orgs —cada evento de Basecamp y cada commit es uno— y se tiraban después
del join: era la parte que más crecía.

**La mantiene un disparador, no el código.** Los comentarios se escriben por
muchos sitios (el servicio de tareas, la línea de estado, GitHub, Basecamp, la
migración de items con SQL crudo) y uno que se olvidara dejaría un comentario
invisible para la búsqueda sin que nada fallara. Por eso la columna tampoco
está en `domain.ItemComment`, como `search`: GORM no la escribe y el
disparador la pone siempre desde la tarea. Si una tarea cambiara de org, el
otro disparador se lo lleva a sus comentarios. Un comentario cuya tarea no
existe se queda sin org, y la búsqueda lo ignora, como antes.

**El índice es parcial y, si se puede, compuesto** `(org_id, search)` con
`btree_gin`, que es una extensión de confianza (no pide superusuario). Si no
se puede crear, el parcial sólo sobre `search`: la org se filtra en la fila,
pero antes del join. Se construye con `CONCURRENTLY` para no bloquear las
escrituras de los comentarios al arrancar; uno que quedó a medias (inválido)
se tira y se vuelve a construir.

Lo que ya había antes de la columna lo rellena `BackfillCommentOrgs`, aparte
y por tandas.

Idempotente: se llama al arrancar y desde las bases de prueba. Lo deshace el
script de vuelta anotado en command-and-control-570.
*/
func ensureCommentOrgs(db *gorm.DB) error {
	for _, s := range []string{
		`ALTER TABLE item_comments ADD COLUMN IF NOT EXISTS org_id varchar(36)`,
		`CREATE OR REPLACE FUNCTION item_comments_org_id() RETURNS trigger LANGUAGE plpgsql AS $$
		BEGIN
			NEW.org_id := (SELECT org_id FROM items WHERE id = NEW.item_id);
			RETURN NEW;
		END $$`,
		`DROP TRIGGER IF EXISTS trg_item_comments_org_id ON item_comments`,
		`CREATE TRIGGER trg_item_comments_org_id BEFORE INSERT OR UPDATE OF item_id, org_id ON item_comments
			FOR EACH ROW EXECUTE FUNCTION item_comments_org_id()`,
		// Lo que escribe este de abajo da igual (con `OLD.org_id` saldría lo
		// mismo): tocar `org_id` dispara `trg_item_comments_org_id`, que relee
		// la org de la tarea. Lo que importa es que toque las filas.
		`CREATE OR REPLACE FUNCTION items_org_id_to_comments() RETURNS trigger LANGUAGE plpgsql AS $$
		BEGIN
			UPDATE item_comments SET org_id = NEW.org_id WHERE item_id = NEW.id;
			RETURN NULL;
		END $$`,
		`DROP TRIGGER IF EXISTS trg_items_org_id_to_comments ON items`,
		`CREATE TRIGGER trg_items_org_id_to_comments AFTER UPDATE OF org_id ON items
			FOR EACH ROW WHEN (OLD.org_id IS DISTINCT FROM NEW.org_id) EXECUTE FUNCTION items_org_id_to_comments()`,
	} {
		if err := db.Exec(s).Error; err != nil {
			return err
		}
	}

	composite := db.Exec(`CREATE EXTENSION IF NOT EXISTS btree_gin`).Error == nil
	if !composite {
		var n int64
		db.Raw(`SELECT count(*) FROM pg_extension WHERE extname = 'btree_gin'`).Scan(&n)
		composite = n > 0
	}
	name, cols, other := "idx_item_comments_org_search", "org_id, search", "idx_item_comments_live_search"
	if !composite {
		lg.Warn("search: no btree_gin extension — comments are indexed by text only, and the organization is filtered on the row")
		name, cols, other = other, "search", name
	}
	// La misma condición que `notServerChatter`, escrita igual: Postgres sólo
	// usa un índice parcial si la consulta implica su `WHERE`.
	if err := ensureConcurrentIndex(db, name, `CREATE INDEX CONCURRENTLY IF NOT EXISTS `+name+`
		ON item_comments USING GIN (`+cols+`)
		WHERE deleted_at IS NULL AND NOT (kind = 'system' AND COALESCE(source_key, '') = '')`); err != nil {
		return err
	}
	// El de antes (todo `search`, sin condición) ya no lo usa nadie, y cada
	// comentario pagaba dos GIN al escribirse.
	for _, s := range []string{`DROP INDEX IF EXISTS ` + other, `DROP INDEX IF EXISTS idx_item_comments_search`} {
		if err := db.Exec(s).Error; err != nil {
			return err
		}
	}
	return nil
}

// ensureConcurrentIndex: un `CREATE INDEX CONCURRENTLY` que falla a medias
// deja el índice inválido, y `IF NOT EXISTS` lo daría por hecho para siempre.
func ensureConcurrentIndex(db *gorm.DB, name, create string) error {
	var invalid bool
	db.Raw(`SELECT EXISTS (SELECT 1 FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
		WHERE c.relname = ? AND NOT i.indisvalid)`, name).Scan(&invalid)
	if invalid {
		if err := db.Exec(`DROP INDEX IF EXISTS ` + name).Error; err != nil {
			return err
		}
	}
	return db.Exec(create).Error
}

// commentOrgBatch: cuántos comentarios rellena cada sentencia. Rellenar
// recalcula `search` (es STORED) en cada fila, y en una sola sentencia la
// tabla entera de producción podía pasar del healthcheck del arranque.
const commentOrgBatch = 5000

// commentOrgsBackfilled: si ya no queda ningún comentario de antes de la
// columna por rellenar. Mientras no, la búsqueda del hilo acepta también los
// que aún no tienen org (la valla es la org de la tarea, ver TasksTS); si no,
// un comentario viejo no saldría hasta que acabara el relleno.
var commentOrgsBackfilled atomic.Bool

/*
BackfillCommentOrgs le pone su org a los comentarios que no la tienen, de
`batch` en `batch` hasta que no queda ninguno, y lo dice en el log. Sólo
escribe la columna nueva, y sólo donde falta: se puede correr dos veces, y
cortarlo a medias no deja nada mal. Al arrancar corre en segundo plano, para
que el servidor escuche sin esperarlo.

Un comentario cuya tarea no existe no se toca (no hay org que darle), y por
eso la subconsulta cruza con `items`: sin eso, una tanda que fueran sólo
huérfanos no rellenaría nada y el bucle pararía con trabajo por hacer.
*/
func BackfillCommentOrgs(db *gorm.DB, batch int) (int64, error) {
	var total int64
	for {
		res := db.Exec(`UPDATE item_comments c SET org_id = t.org_id FROM items t
			WHERE t.id = c.item_id AND c.id IN (
				SELECT c2.id FROM item_comments c2 JOIN items t2 ON t2.id = c2.item_id
				WHERE c2.org_id IS NULL AND t2.org_id IS NOT NULL
				LIMIT ?)`, batch)
		if res.Error != nil {
			return total, res.Error
		}
		if res.RowsAffected == 0 {
			break
		}
		total += res.RowsAffected
		lg.Info(fmt.Sprintf("search: %d comments got their organization so far", total))
	}
	commentOrgsBackfilled.Store(true)
	return total, nil
}
