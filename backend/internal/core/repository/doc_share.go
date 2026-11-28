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

// OrgShareRow: un enlace vivo de la organización con lo que hace falta para
// pintarlo sin otra consulta: la página, el nodo del que cuelga su documento,
// el espacio de ese nodo y quién lo abrió.
type OrgShareRow struct {
	domain.DocShare
	PageTitle     string
	DocKind       domain.DocOwnerKind
	OwnerID       string
	OwnerName     string
	SpaceID       string
	SpaceName     string
	CreatedByName string
}

// ListLiveForOrg: los enlaces que todavía abren algo en toda la organización,
// los que caducan antes primero. Con spaceID, sólo los de ese espacio.
//
// El espacio sale del dueño del documento en la misma consulta: una carpeta y
// una lista guardan su `space_id` aunque estén anidadas, así que basta un join
// por clase y no hay que subir el árbol. LEFT JOIN a propósito: un enlace cuyo
// nodo ya no está sigue abriendo la página (Public no mira el nodo), y una
// pantalla para tener los enlaces controlados no puede esconderlo; sale sin
// espacio. La página en la papelera sí lo saca: no abre nada, y la lista por
// página tampoco lo enseña.
func (r *DocShareRepository) ListLiveForOrg(orgID, spaceID string, now time.Time) ([]OrgShareRow, error) {
	q := `
		SELECT s.*,
			p.title AS page_title,
			d.owner_kind AS doc_kind,
			d.owner_id AS owner_id,
			COALESCE(sp.name, f.name, l.name, '') AS owner_name,
			COALESCE(sp.id, f.space_id, l.space_id, '') AS space_id,
			COALESCE(sp.name, fs.name, ls.name, '') AS space_name,
			(SELECT ` + nombreVisible + ` FROM users WHERE users.id = s.created_by) AS created_by_name
		FROM doc_shares s
		JOIN docs d ON d.id = s.doc_id
		JOIN doc_pages p ON p.id = s.page_id AND p.doc_id = s.doc_id AND p.deleted_at IS NULL
		LEFT JOIN task_spaces sp ON d.owner_kind = 'space' AND sp.id = d.owner_id
		LEFT JOIN task_folders f ON d.owner_kind = 'folder' AND f.id = d.owner_id
		LEFT JOIN task_spaces fs ON fs.id = f.space_id
		LEFT JOIN task_lists l ON d.owner_kind = 'list' AND l.id = d.owner_id
		LEFT JOIN task_spaces ls ON ls.id = l.space_id
		WHERE s.org_id = ? AND s.revoked_at IS NULL AND s.expires_at > ?`
	args := []any{orgID, now}
	if spaceID != "" {
		q += ` AND COALESCE(sp.id, f.space_id, l.space_id) = ?`
		args = append(args, spaceID)
	}
	q += ` ORDER BY s.expires_at ASC, s.created_at ASC`
	out := []OrgShareRow{}
	err := r.db.Raw(q, args...).Scan(&out).Error
	return out, err
}
