package repository

import (
	"time"

	"gorm.io/gorm"
	"gorm.io/gorm/clause"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Los hilos de Basecamp y lo que hace falta para el evento completo
// (command-and-control-558, D). Ver domain/basecamp_event.go.

// ApplyToThread pone al día el hilo con un evento y lo devuelve como quedó.
//
// La fila se bloquea mientras se decide: dos entregas casi juntas del mismo
// hilo (el cliente comenta y alguien del equipo contesta en el mismo segundo)
// no se pisan, y el orden lo pone la hora de Basecamp, no cuál llegó antes.
func (r *BasecampRepository) ApplyToThread(w domain.BasecampThreadWrite) (*domain.BasecampThread, error) {
	var out domain.BasecampThread
	err := r.db.Transaction(func(tx *gorm.DB) error {
		seed := &domain.BasecampThread{OrgID: w.OrgID, RecordingID: w.RecordingID, ProjectID: w.ProjectID}
		if err := tx.Clauses(clause.OnConflict{
			Columns:   []clause.Column{{Name: "org_id"}, {Name: "recording_id"}},
			DoNothing: true,
		}).Create(seed).Error; err != nil {
			return err
		}
		if err := tx.Clauses(clause.Locking{Strength: "UPDATE"}).
			First(&out, "org_id = ? AND recording_id = ?", w.OrgID, w.RecordingID).Error; err != nil {
			return err
		}
		out.Apply(w)
		out.UpdatedAt = time.Now().UTC()
		return tx.Save(&out).Error
	})
	if err != nil {
		return nil, err
	}
	return &out, nil
}

// Awaiting: los hilos por contestar de los proyectos enlazados de la org, del
// que más lleva esperando al que menos.
func (r *BasecampRepository) Awaiting(orgID string) ([]domain.BasecampAwaiting, error) {
	type row struct {
		domain.BasecampThread
		ProjectName string
		SpaceID     string
		ItemID      string
		ItemSeq     int
		ItemProject string
		SpaceKey    string
		ProjectSlug string
	}
	var rows []row
	err := r.db.Raw(`SELECT t.*, p.name AS project_name, p.space_id AS space_id,
			COALESCE(i.id, '') AS item_id, COALESCE(i.seq, 0) AS item_seq,
			COALESCE(i.project_id, '') AS item_project, COALESCE(s.key, '') AS space_key
		FROM basecamp_threads t
		JOIN basecamp_projects p ON p.org_id = t.org_id AND p.project_id = t.project_id
		LEFT JOIN basecamp_links l ON l.org_id = t.org_id AND l.recording_id = t.recording_id
		LEFT JOIN items i ON i.id = l.item_id AND i.deleted_at IS NULL
		LEFT JOIN task_spaces s ON s.id = i.space_id
		WHERE t.org_id = ? AND t.awaiting_reply
		ORDER BY t.awaiting_since ASC NULLS LAST, t.recording_id ASC`, orgID).Scan(&rows).Error
	if err != nil {
		return nil, err
	}
	out := make([]domain.BasecampAwaiting, 0, len(rows))
	for _, x := range rows {
		a := domain.BasecampAwaiting{ProjectID: x.ProjectID, ProjectName: x.ProjectName, SpaceID: x.SpaceID,
			RecordingID: x.RecordingID, Type: x.Type, Title: x.Title, AppURL: domain.SafeBasecampURL(x.AppURL),
			AwaitingSince: x.AwaitingSince, TaskID: x.ItemID,
			LastExternal: domain.BasecampAwaitingExternal{Name: x.LastExternalName, Client: x.LastExternalClient,
				At: x.LastExternalAt, Excerpt: x.LastExternalExcerpt}}
		if x.ItemID != "" {
			a.TaskRef = r.taskRef(x.ItemProject, x.SpaceKey, x.ItemSeq)
		}
		out = append(out, a)
	}
	return out, nil
}

// CountAwaiting: cuántos hilos por contestar tiene la org (el estado).
func (r *BasecampRepository) CountAwaiting(orgID string) int64 {
	var n int64
	r.db.Raw(`SELECT COUNT(*) FROM basecamp_threads t
		JOIN basecamp_projects p ON p.org_id = t.org_id AND p.project_id = t.project_id
		WHERE t.org_id = ? AND t.awaiting_reply`, orgID).Scan(&n)
	return n
}

// taskRef: la misma regla que TaskService.Detail (el folio de un cliente o la
// clave del espacio y el número).
func (r *BasecampRepository) taskRef(projectID, spaceKey string, seq int) string {
	if projectID != "" {
		var slug string
		r.db.Raw(`SELECT slug FROM report_projects WHERE id = ?`, projectID).Scan(&slug)
		if slug == "" {
			return ""
		}
		return domain.Folio(slug, seq)
	}
	return domain.SpaceTaskRef(spaceKey, seq)
}

// TaskRefOf: la referencia de una tarea, para el evento.
func (r *BasecampRepository) TaskRefOf(item *domain.Item) string {
	if item == nil {
		return ""
	}
	var key string
	if item.ProjectID == "" {
		r.db.Raw(`SELECT key FROM task_spaces WHERE id = ?`, item.SpaceID).Scan(&key)
	}
	return r.taskRef(item.ProjectID, key, item.Seq)
}

// MemberIDs: quién es hoy de la org, con cualquier rol. Es a quien le llega
// la actividad de Basecamp: un superadmin que no es miembro no está aquí.
func (r *BasecampRepository) MemberIDs(orgID string) ([]string, error) {
	var ids []string
	err := r.db.Raw(`SELECT user_id FROM org_memberships WHERE org_id = ? ORDER BY user_id`, orgID).Scan(&ids).Error
	return ids, err
}

// OpenRequestCodes: los pedidos abiertos de la tarea.
func (r *BasecampRepository) OpenRequestCodes(itemID string) []string {
	var codes []string
	r.db.Raw(`SELECT code FROM requests WHERE task_id = ? AND status = ? ORDER BY created_at`,
		itemID, domain.RequestOpen).Scan(&codes)
	if codes == nil {
		codes = []string{}
	}
	return codes
}
