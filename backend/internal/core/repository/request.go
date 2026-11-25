package repository

import (
	"errors"
	"strings"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

var ErrRequestNotFound = errors.New("request not found")

type RequestRepository struct{ db *gorm.DB }

func NewRequestRepository(db *gorm.DB) *RequestRepository {
	return &RequestRepository{db: db}
}

// Create guarda el pedido con un ID corto nuevo. Seis letras de 31 dan para
// casi novecientos millones: chocar es rarísimo, pero si pasa se prueba otro
// en vez de devolver un 500 a quien sólo quería pedir algo.
func (r *RequestRepository) Create(q *domain.Request) error {
	var err error
	for range 5 {
		q.Code = domain.NewRequestCode()
		if err = r.db.Create(q).Error; err == nil {
			return nil
		}
		if !strings.Contains(err.Error(), "duplicate key") || !strings.Contains(err.Error(), "code") {
			return err
		}
		q.ID = ""
	}
	return err
}

// Find por uuid o por ID corto. El corto, sin distinguir mayúsculas: se dicta.
func (r *RequestRepository) Find(idOrCode string) (*domain.Request, error) {
	var q domain.Request
	key := strings.TrimSpace(idOrCode)
	tx := r.db
	if domain.LooksLikeRequestCode(key) {
		tx = tx.Where("code = ?", strings.ToLower(key))
	} else {
		tx = tx.Where("id = ?", key)
	}
	if err := tx.First(&q).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, ErrRequestNotFound
		}
		return nil, err
	}
	return &q, nil
}

// Reissue guarda la versión vigente en el historial y escribe la nueva, en una
// transacción: un historial sin la versión que se pisó no es historial.
func (r *RequestRepository) Reissue(prev domain.Request, fields map[string]any) error {
	return r.db.Transaction(func(tx *gorm.DB) error {
		old := domain.RequestVersion{
			RequestID: prev.ID, Version: prev.Version, Kind: prev.Kind,
			Summary: prev.Summary, Body: prev.Body, ByID: prev.RequestedByID,
		}
		if err := tx.Create(&old).Error; err != nil {
			return err
		}
		fields["version"] = prev.Version + 1
		res := tx.Model(&domain.Request{}).Where("id = ? AND status = ? AND version = ?", prev.ID, domain.RequestOpen, prev.Version).Updates(fields)
		if res.Error != nil {
			return res.Error
		}
		if res.RowsAffected == 0 {
			return ErrRequestNotOpen
		}
		return nil
	})
}

// ErrRequestNotOpen: el pedido ya se contestó o se retiró (o lo rehízo otro
// mientras tanto).
var ErrRequestNotOpen = errors.New("request-not-open")

// Settle cierra un pedido abierto. Condicionado a que siga abierto: dos
// respuestas a la vez (el botón y la terminal) no se pisan, gana la primera.
func (r *RequestRepository) Settle(id string, fields map[string]any) error {
	res := r.db.Model(&domain.Request{}).Where("id = ? AND status = ?", id, domain.RequestOpen).Updates(fields)
	if res.Error != nil {
		return res.Error
	}
	if res.RowsAffected == 0 {
		return ErrRequestNotOpen
	}
	return nil
}

func (r *RequestRepository) SetPublished(id, url string) error {
	return r.db.Model(&domain.Request{}).Where("id = ?", id).Update("published_url", url).Error
}

func (r *RequestRepository) History(id string) ([]domain.RequestVersion, error) {
	out := []domain.RequestVersion{}
	err := r.db.Where("request_id = ?", id).Order("version ASC").Find(&out).Error
	return out, err
}

// RequestFilter: qué pedidos lista la bandeja.
type RequestFilter struct {
	OrgIDs []string
	// RequestedOf: sólo los pedidos a esta persona. Vacío: los de todos.
	RequestedOf string
	TaskID      string
	Statuses    []domain.RequestStatus
	Limit       int
}

// List: por urgencia de la tarea y, a igual urgencia, el más viejo primero (lo
// que lleva más esperando). Los cerrados, el más reciente primero.
//
// La prioridad se pregunta por el nombre **guardado** (`medium`, no `normal`):
// ordenar por el de la API dejaba casi todo debajo de lo de prioridad baja (#83).
func (r *RequestRepository) List(f RequestFilter) ([]domain.Request, error) {
	out := []domain.Request{}
	if len(f.OrgIDs) == 0 {
		return out, nil
	}
	tx := r.db.Table("requests").Select("requests.*").
		Joins("JOIN items i ON i.id = requests.task_id AND i.deleted_at IS NULL").
		Where("requests.org_id IN ?", f.OrgIDs)
	if f.RequestedOf != "" {
		tx = tx.Where("requests.requested_of_id = ?", f.RequestedOf)
	}
	if f.TaskID != "" {
		tx = tx.Where("requests.task_id = ?", f.TaskID)
	}
	if len(f.Statuses) > 0 {
		tx = tx.Where("requests.status IN ?", f.Statuses)
	}
	limit := f.Limit
	if limit <= 0 || limit > 200 {
		limit = 200
	}
	err := tx.Order(`CASE WHEN requests.status = 'open' THEN 0 ELSE 1 END,
		CASE WHEN requests.status = 'open' THEN
			CASE i.priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END
		ELSE 0 END,
		CASE WHEN requests.status = 'open' THEN requests.created_at END ASC,
		COALESCE(requests.answered_at, requests.withdrawn_at, requests.updated_at) DESC`).
		Limit(limit).Scan(&out).Error
	return out, err
}

// OpenOfTasks: los pedidos abiertos de cada tarea, para marcarla «espera a…».
func (r *RequestRepository) OpenOfTasks(taskIDs []string) (map[string][]domain.Request, error) {
	out := map[string][]domain.Request{}
	if len(taskIDs) == 0 {
		return out, nil
	}
	var rows []domain.Request
	if err := r.db.Where("task_id IN ? AND status = ?", taskIDs, domain.RequestOpen).
		Order("created_at ASC").Find(&rows).Error; err != nil {
		return out, err
	}
	for _, q := range rows {
		out[q.TaskID] = append(out[q.TaskID], q)
	}
	return out, nil
}

// DisplayName por `nombreVisible`, como en todo lo que se enseña.
func (r *RequestRepository) DisplayName(userID string) string {
	if userID == "" {
		return ""
	}
	var name string
	r.db.Table("users").Select(nombreVisible).Where("id = ?", userID).Scan(&name)
	return name
}

// IsMember: si alguien es de la organización (a quien se le puede pedir algo).
func (r *RequestRepository) IsMember(orgID, userID string) bool {
	var n int64
	r.db.Table("org_memberships").Where("org_id = ? AND user_id = ?", orgID, userID).Count(&n)
	return n > 0
}
