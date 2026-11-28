package repository

import (
	"encoding/base64"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"time"

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
	Kinds       []domain.RequestKind
	// Text busca en el código, el resumen y el título de la tarea; TextTaskIDs
	// son las tareas que nombra (su ref), que el servicio ya resolvió.
	Text        string
	TextTaskIDs []string
	Limit       int
}

// MaxRequestPage: el tope de una página con cursor. Sin cursor la lista sigue
// con el suyo de siempre (200), que es lo que esperan la web y el MCP.
const MaxRequestPage = 100

// requestPriorityRank: la urgencia de la tarea, por el nombre **guardado**
// (`medium`, no `normal`): ordenar por el de la API dejaba casi todo debajo
// de lo de prioridad baja (#83).
const requestPriorityRank = `CASE i.priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END`

// requestClosedAt: cuándo se cerró. Es la expresión de los índices
// `idx_requests_resolved_*`: si cambia aquí, cambia allí, o deja de usarlos.
const requestClosedAt = `COALESCE(requests.answered_at, requests.withdrawn_at, requests.updated_at)`

func (r *RequestRepository) filtered(f RequestFilter) *gorm.DB {
	tx := r.db.Table("requests").
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
	if len(f.Kinds) > 0 {
		tx = tx.Where("requests.kind IN ?", f.Kinds)
	}
	if text := strings.TrimSpace(f.Text); text != "" {
		like := "%" + escapeLike(strings.ToLower(text)) + "%"
		cond := `(requests.code LIKE ? ESCAPE '\' OR LOWER(requests.summary) LIKE ? ESCAPE '\' OR LOWER(i.title) LIKE ? ESCAPE '\'`
		args := []any{like, like, like}
		if len(f.TextTaskIDs) > 0 {
			cond += ` OR requests.task_id IN ?`
			args = append(args, f.TextTaskIDs)
		}
		tx = tx.Where(cond+")", args...)
	}
	return tx
}

// List: por urgencia de la tarea y, a igual urgencia, el más viejo primero (lo
// que lleva más esperando). Los cerrados, el más reciente primero.
func (r *RequestRepository) List(f RequestFilter) ([]domain.Request, error) {
	out := []domain.Request{}
	if len(f.OrgIDs) == 0 {
		return out, nil
	}
	limit := f.Limit
	if limit <= 0 || limit > 200 {
		limit = 200
	}
	err := r.filtered(f).Select("requests.*").Order(`CASE WHEN requests.status = 'open' THEN 0 ELSE 1 END,
		CASE WHEN requests.status = 'open' THEN ` + requestPriorityRank + ` ELSE 0 END,
		CASE WHEN requests.status = 'open' THEN requests.created_at END ASC,
		` + requestClosedAt + ` DESC`).
		Limit(limit).Scan(&out).Error
	return out, err
}

// requestCursor: dónde acabó la página, con las claves del orden. Los abiertos
// van por (urgencia, creado, id) hacia arriba; los cerrados, por (cerrado, id)
// hacia abajo. El id desempata: sin él, dos pedidos con la misma fecha se
// repiten o se saltan entre páginas.
type requestCursor struct {
	open bool
	rank int
	at   time.Time
	id   string
}

func (c requestCursor) encode() string {
	seg := "c"
	if c.open {
		seg = "o"
	}
	raw := fmt.Sprintf("%s|%d|%s|%s", seg, c.rank, c.at.UTC().Format(time.RFC3339Nano), c.id)
	return base64.RawURLEncoding.EncodeToString([]byte(raw))
}

func decodeRequestCursor(s string) (requestCursor, error) {
	raw, err := base64.RawURLEncoding.DecodeString(s)
	if err != nil {
		return requestCursor{}, ErrBadCursor
	}
	parts := strings.SplitN(string(raw), "|", 4)
	if len(parts) != 4 || (parts[0] != "o" && parts[0] != "c") || parts[3] == "" {
		return requestCursor{}, ErrBadCursor
	}
	rank, err := strconv.Atoi(parts[1])
	if err != nil {
		return requestCursor{}, ErrBadCursor
	}
	at, err := time.Parse(time.RFC3339Nano, parts[2])
	if err != nil {
		return requestCursor{}, ErrBadCursor
	}
	return requestCursor{open: parts[0] == "o", rank: rank, at: at, id: parts[3]}, nil
}

// Page: una página de la bandeja, en el orden de List con el id de desempate,
// y el cursor de la siguiente ("" si no hay más). `cursor` vacío: la primera.
func (r *RequestRepository) Page(f RequestFilter, cursor string) ([]domain.Request, string, error) {
	out := []domain.Request{}
	if len(f.OrgIDs) == 0 {
		return out, "", nil
	}
	limit := f.Limit
	if limit <= 0 {
		limit = 50
	}
	if limit > MaxRequestPage {
		limit = MaxRequestPage
	}
	tx := r.filtered(f)
	if cursor != "" {
		c, err := decodeRequestCursor(cursor)
		if err != nil {
			return out, "", err
		}
		if c.open {
			tx = tx.Where(`(requests.status = 'open' AND (`+requestPriorityRank+`, requests.created_at, requests.id) > (?, ?, ?))
				OR requests.status <> 'open'`, c.rank, c.at, c.id)
		} else {
			tx = tx.Where(`requests.status <> 'open' AND (`+requestClosedAt+`, requests.id) < (?, ?)`, c.at, c.id)
		}
	}
	type row struct {
		domain.Request
		SortRank     int       `gorm:"column:sort_rank"`
		SortClosedAt time.Time `gorm:"column:sort_closed_at"`
	}
	order := `CASE WHEN requests.status = 'open' THEN 0 ELSE 1 END,
		CASE WHEN requests.status = 'open' THEN ` + requestPriorityRank + ` ELSE 0 END,
		CASE WHEN requests.status = 'open' THEN requests.created_at END ASC,
		CASE WHEN requests.status = 'open' THEN requests.id END ASC,
		CASE WHEN requests.status <> 'open' THEN ` + requestClosedAt + ` END DESC,
		CASE WHEN requests.status <> 'open' THEN requests.id END DESC`
	// Sólo resueltos: el mismo orden dicho sin CASE, que es el de los índices
	// `idx_requests_resolved_*`: Postgres lo lee ya ordenado y para en la página.
	if onlyClosed(f.Statuses) {
		order = requestClosedAt + ` DESC, requests.id DESC`
	}
	var rows []row
	err := tx.Select("requests.*, " + requestPriorityRank + " AS sort_rank, " + requestClosedAt + " AS sort_closed_at").
		Order(order).Limit(limit + 1).Scan(&rows).Error
	if err != nil {
		return out, "", err
	}
	next := ""
	if len(rows) > limit {
		rows = rows[:limit]
		last := rows[limit-1]
		c := requestCursor{open: last.Status == domain.RequestOpen, id: last.ID}
		if c.open {
			c.rank, c.at = last.SortRank, last.CreatedAt
		} else {
			c.at = last.SortClosedAt
		}
		next = c.encode()
	}
	for _, x := range rows {
		out = append(out, x.Request)
	}
	return out, next, nil
}

func onlyClosed(statuses []domain.RequestStatus) bool {
	if len(statuses) == 0 {
		return false
	}
	for _, st := range statuses {
		if st == domain.RequestOpen {
			return false
		}
	}
	return true
}

// EnsureRequestIndexes: los resueltos se leen de lo más reciente atrás, de
// una persona («Míos») o de una organización («Equipo»), y se paginan con
// cursor: sin índice sobre ese orden, cada página recorre todos los cerrados.
func EnsureRequestIndexes(db *gorm.DB) error {
	for _, stmt := range []string{
		`CREATE INDEX IF NOT EXISTS idx_requests_resolved_of
			ON requests (requested_of_id, (COALESCE(answered_at, withdrawn_at, updated_at)) DESC, id DESC) WHERE status <> 'open'`,
		`CREATE INDEX IF NOT EXISTS idx_requests_resolved_org
			ON requests (org_id, (COALESCE(answered_at, withdrawn_at, updated_at)) DESC, id DESC) WHERE status <> 'open'`,
	} {
		if err := db.Exec(stmt).Error; err != nil {
			return err
		}
	}
	return nil
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
