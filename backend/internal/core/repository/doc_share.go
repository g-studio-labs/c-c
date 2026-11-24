package repository

import (
	"errors"
	"time"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

var ErrDocShareNotFound = errors.New("doc share not found")

type DocShareRepository struct{ db *gorm.DB }

func NewDocShareRepository(db *gorm.DB) *DocShareRepository {
	return &DocShareRepository{db: db}
}

func (r *DocShareRepository) Create(s *domain.DocShare) error {
	return r.db.Create(s).Error
}

func (r *DocShareRepository) FindByID(id string) (*domain.DocShare, error) {
	var s domain.DocShare
	if err := r.db.First(&s, "id = ?", id).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, ErrDocShareNotFound
		}
		return nil, err
	}
	return &s, nil
}

// ListLive: los enlaces de una página que todavía abren algo, el más nuevo
// primero.
func (r *DocShareRepository) ListLive(docID, pageID string, now time.Time) ([]domain.DocShare, error) {
	out := []domain.DocShare{}
	err := r.db.Where("doc_id = ? AND page_id = ? AND revoked_at IS NULL AND expires_at > ?", docID, pageID, now).
		Order("created_at DESC").Find(&out).Error
	return out, err
}

// Revoke cierra el enlace. Idempotente: revocar dos veces deja la primera fecha.
func (r *DocShareRepository) Revoke(id string, now time.Time) error {
	return r.db.Model(&domain.DocShare{}).
		Where("id = ? AND revoked_at IS NULL", id).
		Update("revoked_at", now).Error
}

// sharedTreeCTE: la raíz y sus descendientes vivos. El `deleted_at IS NULL` va
// en cada paso y no sólo al final: una hija viva de una página tirada a la
// papelera tampoco se ve, igual que en la app. `UNION` y no `UNION ALL`: con
// un ciclo de madres en los datos la recursión no termina nunca, y un enlace
// público no puede colgar la base; `UNION` deja de bajar al repetir un id.
const sharedTreeCTE = `
	WITH RECURSIVE sub AS (
		SELECT id FROM doc_pages WHERE id = ? AND doc_id = ? AND deleted_at IS NULL
		UNION
		SELECT p.id FROM doc_pages p JOIN sub ON p.parent_id = sub.id
		WHERE p.deleted_at IS NULL AND p.doc_id = ?
	)`

// SharedTree: las páginas del árbol compartido, la raíz primero y cada nivel
// debajo de su madre, las hermanas por rango. Vacío si la raíz ya no está.
func (r *DocShareRepository) SharedTree(docID, rootID string) ([]domain.DocPage, error) {
	var rows []domain.DocPage
	// `rank` es NUMERIC: el ORDER BY es numérico y no depende de la colación.
	if err := r.db.Raw(sharedTreeCTE+`
		SELECT p.id, p.parent_id, p.title, p.body, p.rank::text AS rank
		FROM doc_pages p JOIN sub ON sub.id = p.id
		ORDER BY p.rank ASC, p.created_at ASC`, rootID, docID, docID).Scan(&rows).Error; err != nil {
		return nil, err
	}
	kids := map[string][]domain.DocPage{}
	var root *domain.DocPage
	for i := range rows {
		if rows[i].ID == rootID {
			root = &rows[i]
			continue
		}
		if rows[i].ParentID != nil {
			kids[*rows[i].ParentID] = append(kids[*rows[i].ParentID], rows[i])
		}
	}
	if root == nil {
		return []domain.DocPage{}, nil
	}
	out := make([]domain.DocPage, 0, len(rows))
	var walk func(p domain.DocPage)
	walk = func(p domain.DocPage) {
		out = append(out, p)
		for _, k := range kids[p.ID] {
			walk(k)
		}
	}
	walk(*root)
	return out, nil
}

// CitedInTree: si alguna página viva del árbol compartido cita el adjunto. Por
// el id dentro del cuerpo, como `DocRepository.AttachmentCited`: lo cita igual
// la ruta relativa (`/api/v1/docs/<doc>/attachments/<id>/raw`) que la absoluta.
// Quien llama ya comprobó que el adjunto existe y es del documento, así que el
// id es un uuid de la base y no lleva comodines de LIKE.
func (r *DocShareRepository) CitedInTree(docID, rootID, attachmentID string) (bool, error) {
	var n int64
	err := r.db.Raw(sharedTreeCTE+`
		SELECT count(*) FROM doc_pages p JOIN sub ON sub.id = p.id
		WHERE p.body LIKE ?`, rootID, docID, docID, "%"+attachmentID+"%").Scan(&n).Error
	return n > 0, err
}
