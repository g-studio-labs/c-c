package repository

import (
	"gorm.io/gorm"
	"gorm.io/gorm/clause"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// syncTaskKeys recalcula las claves de fuera que nombra una tarea, leyendo su
// título y su descripción de la base. Como `syncText`: con el mismo manejador
// que el guardado, y sin tirar el guardado si falla.
func syncTaskKeys(db *gorm.DB, taskID string) {
	if taskID == "" {
		return
	}
	var t struct{ OrgID, Title, Description string }
	if err := db.Raw(`SELECT org_id, title, COALESCE(description, '') AS description FROM items
		WHERE id = ? AND deleted_at IS NULL`, taskID).Scan(&t).Error; err != nil || t.OrgID == "" {
		return
	}
	// Se reescriben todas: cambiar de sitio la clave (de una cita a la línea
	// **Jira:**) cambia cómo la nombra sin cambiar la clave.
	if err := db.Where("task_id = ?", taskID).Delete(&domain.TaskExternalKey{}).Error; err != nil {
		return
	}
	for _, k := range domain.ClassifyExternalKeys(t.Title, t.Description) {
		k.TaskID, k.OrgID = taskID, t.OrgID
		db.Clauses(clause.OnConflict{DoNothing: true}).Create(&k)
	}
}

// FindByExternalKey: las tareas vivas de esas organizaciones a las que lleva
// la clave. Las que la tienen en su línea **Jira:**; si no hay ninguna, las
// que la llevan en el título; y si tampoco, las que la citan. Dentro de cada
// grupo, las abiertas primero. Varias en el mismo grupo es legítimo (una tarea
// y la que la cierra en Jira): quien pregunta elige.
func (r *TaskRepository) FindByExternalKey(orgIDs []string, key string) ([]domain.Task, error) {
	out := []domain.Task{}
	if len(orgIDs) == 0 || key == "" {
		return out, nil
	}
	for _, kind := range domain.ExternalKeyKinds {
		err := r.db.Joins("JOIN task_external_keys k ON k.task_id = items.id").
			Where("k.org_id IN ? AND k.key = ? AND k.kind = ? AND items.archived_at IS NULL", orgIDs, key, kind).
			Order("CASE WHEN items.status IN ('resolved', 'closed') THEN 1 ELSE 0 END, items.updated_at DESC").
			Find(&out).Error
		if err != nil || len(out) > 0 {
			return out, err
		}
	}
	return out, nil
}

// ExternalKeysOf: las claves de cada tarea, para ponerlas en una lista de
// tarjetas con una sola consulta.
func (r *TaskRepository) ExternalKeysOf(taskIDs []string) map[string][]string {
	out := map[string][]string{}
	if len(taskIDs) == 0 {
		return out
	}
	var rows []domain.TaskExternalKey
	r.db.Where("task_id IN ?", taskIDs).Order("created_at ASC, key ASC").Find(&rows)
	for _, x := range rows {
		out[x.TaskID] = append(out[x.TaskID], x.Key)
	}
	return out
}

// BackfillTaskExternalKeys rellena una vez las claves de las tareas que ya
// existían. Sólo con la tabla vacía: después, cada guardado se ocupa.
func BackfillTaskExternalKeys(db *gorm.DB) {
	// «Vacía» es «sin ninguna fila que diga cómo nombra la clave»: las del
	// primer despliegue, de antes de `Kind`, se rehacen.
	var n int64
	if db.Model(&domain.TaskExternalKey{}).Where("kind <> ''").Count(&n); n > 0 {
		return
	}
	var ids []string
	if err := db.Raw(`SELECT id FROM items WHERE deleted_at IS NULL
		AND (title ~ '[A-Z][A-Z0-9]+-[0-9]' OR COALESCE(description, '') ~ '[A-Z][A-Z0-9]+-[0-9]')`).Scan(&ids).Error; err != nil {
		return
	}
	for _, id := range ids {
		syncTaskKeys(db, id)
	}
}
