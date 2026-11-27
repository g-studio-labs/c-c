package repository

import (
	"encoding/json"
	"errors"
	"time"

	"github.com/google/uuid"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// UserEventRepository guarda el diario de eventos de cada persona. Ver
// domain/user_event.go.
type UserEventRepository struct {
	db *gorm.DB
}

func NewUserEventRepository(db *gorm.DB) *UserEventRepository {
	return &UserEventRepository{db: db}
}

// Append apunta un evento y le da el siguiente número de esa persona. Devuelve
// la fila y `false` si la clave ya estaba (no apunta nada).
//
// Todo dentro de una transacción que bloquea la cabecera de la persona: el
// número se da y se confirma con la fila bloqueada, así que nadie confirma un
// número menor después de que otro confirmara uno mayor.
func (r *UserEventRepository) Append(in domain.UserEventInput, data json.RawMessage, now time.Time) (domain.UserEvent, bool, error) {
	var out domain.UserEvent
	created := false
	err := r.db.Transaction(func(tx *gorm.DB) error {
		if err := tx.Clauses(clause.OnConflict{DoNothing: true}).
			Create(&domain.UserEventHead{UserID: in.UserID}).Error; err != nil {
			return err
		}
		var head domain.UserEventHead
		if err := tx.Clauses(clause.Locking{Strength: "UPDATE"}).
			Where("user_id = ?", in.UserID).First(&head).Error; err != nil {
			return err
		}
		// La clave se mira con la cabecera ya bloqueada: dos entregas del mismo
		// webhook esperan una a la otra, y la segunda la encuentra.
		var key *string
		if in.Key != "" {
			k := in.Key
			key = &k
			var n int64
			if err := tx.Model(&domain.UserEvent{}).
				Where("user_id = ? AND key = ?", in.UserID, in.Key).Count(&n).Error; err != nil {
				return err
			}
			if n > 0 {
				return nil
			}
		}
		next := head.LastSeq + 1
		if err := tx.Model(&domain.UserEventHead{}).Where("user_id = ?", in.UserID).
			Update("last_seq", next).Error; err != nil {
			return err
		}
		out = domain.UserEvent{
			UserID: in.UserID, Seq: next, ID: uuid.NewString(), Type: in.Type, OrgID: in.OrgID,
			Key: key, Mine: in.Mine, External: in.External, Data: data, CreatedAt: now,
		}
		if err := tx.Create(&out).Error; err != nil {
			return err
		}
		created = true
		return nil
	})
	return out, created, err
}

// Head: el último número dado a esa persona (0 si nunca tuvo ninguno).
func (r *UserEventRepository) Head(userID string) (int64, error) {
	var head domain.UserEventHead
	err := r.db.Where("user_id = ?", userID).First(&head).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return 0, nil
	}
	return head.LastSeq, err
}

// After: hasta `limit` filas de esa persona con número mayor que `after`, en
// orden; y el menor número que todavía se guarda (0 si no queda ninguno).
func (r *UserEventRepository) After(userID string, after int64, limit int) ([]domain.UserEvent, int64, error) {
	var rows []domain.UserEvent
	if err := r.db.Where("user_id = ? AND seq > ?", userID, after).
		Order("seq ASC").Limit(limit).Find(&rows).Error; err != nil {
		return nil, 0, err
	}
	var oldest struct{ Min *int64 }
	if err := r.db.Model(&domain.UserEvent{}).Select("MIN(seq) AS min").
		Where("user_id = ?", userID).Scan(&oldest).Error; err != nil {
		return nil, 0, err
	}
	var min int64
	if oldest.Min != nil {
		min = *oldest.Min
	}
	return rows, min, nil
}

// Prune borra lo viejo: lo de antes de `before` y lo que pasa de `keep` por
// persona. La cabecera no se toca: el siguiente número sigue donde iba.
func (r *UserEventRepository) Prune(before time.Time, keep int64) (int64, error) {
	old := r.db.Where("created_at < ?", before).Delete(&domain.UserEvent{})
	if old.Error != nil {
		return 0, old.Error
	}
	extra := r.db.Exec(`DELETE FROM user_events e USING user_event_heads h
		WHERE e.user_id = h.user_id AND e.seq <= h.last_seq - ?`, keep)
	return old.RowsAffected + extra.RowsAffected, extra.Error
}
