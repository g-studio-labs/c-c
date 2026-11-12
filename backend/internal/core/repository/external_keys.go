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
	keys := domain.ExternalKeys(t.Title, t.Description)
	q := db.Where("task_id = ?", taskID)
	if len(keys) > 0 {
		q = q.Where("key NOT IN ?", keys)
	}
	if err := q.Delete(&domain.TaskExternalKey{}).Error; err != nil {
		return
	}
	for _, k := range keys {
		db.Clauses(clause.OnConflict{DoNothing: true}).
			Create(&domain.TaskExternalKey{TaskID: taskID, Key: k, OrgID: t.OrgID})
	}
}

// FindByExternalKey: las tareas vivas de esas organizaciones que nombran la
// clave, las abiertas primero y luego la más reciente. Varias es legítimo: un
// ticket de Jira puede tener su tarea y otra que lo cita en **Referencias**.
func (r *TaskRepository) FindByExternalKey(orgIDs []string, key string) ([]domain.Task, error) {
	out := []domain.Task{}
	if len(orgIDs) == 0 || key == "" {
		return out, nil
	}
	err := r.db.Joins("JOIN task_external_keys k ON k.task_id = items.id").
		Where("k.org_id IN ? AND k.key = ? AND items.archived_at IS NULL", orgIDs, key).
		Order("CASE WHEN items.status IN ('resolved', 'closed') THEN 1 ELSE 0 END, items.updated_at DESC").
		Find(&out).Error
	return out, err
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
	var n int64
	if db.Model(&domain.TaskExternalKey{}).Count(&n); n > 0 {
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
