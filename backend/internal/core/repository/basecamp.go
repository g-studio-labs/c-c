package repository

import (
	"crypto/subtle"
	"errors"
	"strconv"
	"strings"
	"time"

	"github.com/google/uuid"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// BasecampRepository: la conexión, los proyectos, las personas, los enlaces y
// los eventos vistos de Basecamp (ver domain/basecamp.go).
type BasecampRepository struct{ db *gorm.DB }

func NewBasecampRepository(db *gorm.DB) *BasecampRepository { return &BasecampRepository{db: db} }

// ─── La conexión ─────────────────────────────────────────────────────────────

func (r *BasecampRepository) Connection(orgID string) (*domain.BasecampConnection, error) {
	var c domain.BasecampConnection
	if err := r.db.First(&c, "org_id = ?", orgID).Error; err != nil {
		return nil, err
	}
	return &c, nil
}

// SaveConnection: conectar otra vez reemplaza la cuenta de la org.
func (r *BasecampRepository) SaveConnection(c *domain.BasecampConnection) error {
	if c.ID == "" {
		c.ID = uuid.NewString()
	}
	return r.db.Clauses(clause.OnConflict{
		Columns: []clause.Column{{Name: "org_id"}},
		DoUpdates: clause.AssignmentColumns([]string{"account_id", "account_name", "identity_name", "connected_by_id",
			"access_token", "refresh_token", "expires_at", "refresh_failed_at", "updated_at"}),
	}).Create(c).Error
}

// SaveTokens: una renovación que salió bien. Limpia el fallo.
func (r *BasecampRepository) SaveTokens(orgID string, access, refresh []byte, expires time.Time) error {
	fields := map[string]any{"access_token": access, "expires_at": expires, "refresh_failed_at": nil, "updated_at": time.Now().UTC()}
	if refresh != nil {
		fields["refresh_token"] = refresh
	}
	return r.db.Model(&domain.BasecampConnection{}).Where("org_id = ?", orgID).Updates(fields).Error
}

// MarkRefreshFailed apunta el fallo y dice si es nuevo: sólo el primero de
// una racha avisa a los admins.
func (r *BasecampRepository) MarkRefreshFailed(orgID string, at time.Time) (bool, error) {
	res := r.db.Model(&domain.BasecampConnection{}).
		Where("org_id = ? AND refresh_failed_at IS NULL", orgID).
		Updates(map[string]any{"refresh_failed_at": at, "updated_at": at})
	return res.RowsAffected == 1, res.Error
}

// ExpiringConnections: las que caducan antes de `before`, para renovarlas
// antes de que nadie las necesite.
func (r *BasecampRepository) ExpiringConnections(before time.Time) ([]domain.BasecampConnection, error) {
	var out []domain.BasecampConnection
	err := r.db.Where("expires_at < ?", before).Find(&out).Error
	return out, err
}

// DeleteConnection: la org deja Basecamp. Se lleva sus proyectos y fuentes;
// las personas y los enlaces se quedan, por si vuelve a conectar.
func (r *BasecampRepository) DeleteConnection(orgID string) error {
	return r.db.Transaction(func(tx *gorm.DB) error {
		if err := deleteProjects(tx, orgID); err != nil {
			return err
		}
		return tx.Where("org_id = ?", orgID).Delete(&domain.BasecampConnection{}).Error
	})
}

// DeleteProjects: los proyectos de la org y sus fuentes. Al conectar otra
// cuenta, los de la vieja ya no son de nadie.
func (r *BasecampRepository) DeleteProjects(orgID string) error {
	return r.db.Transaction(func(tx *gorm.DB) error { return deleteProjects(tx, orgID) })
}

func deleteProjects(tx *gorm.DB, orgID string) error {
	var projects []domain.BasecampProject
	if err := tx.Where("org_id = ?", orgID).Find(&projects).Error; err != nil {
		return err
	}
	for _, p := range projects {
		if err := deleteProjectSource(tx, &p); err != nil {
			return err
		}
	}
	return tx.Where("org_id = ?", orgID).Delete(&domain.BasecampProject{}).Error
}

// SaveState apunta un enlace de conectar recién dado, y de paso barre los
// caducados.
func (r *BasecampRepository) SaveState(st *domain.BasecampOAuthState, now time.Time) error {
	r.db.Where("expires_at < ?", now).Delete(&domain.BasecampOAuthState{})
	return r.db.Create(st).Error
}

// ConsumeState borra el nonce y dice si estaba (y vivo): sólo la primera
// vuelta con ese state lo encuentra.
func (r *BasecampRepository) ConsumeState(nonce, orgID string, now time.Time) (bool, error) {
	res := r.db.Where("nonce = ? AND org_id = ? AND expires_at >= ?", nonce, orgID, now).
		Delete(&domain.BasecampOAuthState{})
	return res.RowsAffected == 1, res.Error
}

// ─── Proyectos ───────────────────────────────────────────────────────────────

func (r *BasecampRepository) Projects(orgID string) ([]domain.BasecampProject, error) {
	out := []domain.BasecampProject{}
	err := r.db.Where("org_id = ?", orgID).Order("name ASC, project_id ASC").Find(&out).Error
	return out, err
}

func (r *BasecampRepository) Project(orgID string, projectID int64) (*domain.BasecampProject, error) {
	var p domain.BasecampProject
	if err := r.db.First(&p, "org_id = ? AND project_id = ?", orgID, projectID).Error; err != nil {
		return nil, err
	}
	return &p, nil
}

// ProjectBySecret: el proyecto cuyo webhook lleva ese secreto.
//
// Se busca por el hash, no por el secreto: quien prueba secretos no controla
// qué hash le toca, así que el tiempo del índice no le dice nada. Y el último
// paso es una comparación en tiempo constante.
func (r *BasecampRepository) ProjectBySecret(secret string) (*domain.BasecampProject, error) {
	if len(secret) < 20 || len(secret) > 200 {
		return nil, gorm.ErrRecordNotFound
	}
	want := domain.BasecampSecretHash(secret)
	var p domain.BasecampProject
	if err := r.db.First(&p, "secret_hash = ?", want).Error; err != nil {
		return nil, err
	}
	if subtle.ConstantTimeCompare([]byte(p.SecretHash), []byte(want)) != 1 {
		return nil, gorm.ErrRecordNotFound
	}
	return &p, nil
}

// SaveProject enlaza el proyecto al espacio y apunta la fuente del espacio
// (`basecamp`, `tracker`) en la misma transacción.
func (r *BasecampRepository) SaveProject(p *domain.BasecampProject) error {
	if p.ID == "" {
		p.ID = uuid.NewString()
	}
	return r.db.Transaction(func(tx *gorm.DB) error {
		var old domain.BasecampProject
		if err := tx.First(&old, "org_id = ? AND project_id = ?", p.OrgID, p.ProjectID).Error; err == nil {
			if err := deleteProjectSource(tx, &old); err != nil {
				return err
			}
			if err := tx.Delete(&old).Error; err != nil {
				return err
			}
		} else if !errors.Is(err, gorm.ErrRecordNotFound) {
			return err
		}
		if err := tx.Create(p).Error; err != nil {
			return err
		}
		now := time.Now().UTC()
		return tx.Create(&domain.SpaceSource{ID: uuid.NewString(), OrgID: p.OrgID, SpaceID: p.SpaceID,
			Role: domain.SourceTracker, System: domain.SystemBasecamp, Ref: projectSourceRef(p),
			Note: p.Name, CreatedAt: now, UpdatedAt: now}).Error
	})
}

func (r *BasecampRepository) DeleteProject(p *domain.BasecampProject) error {
	return r.db.Transaction(func(tx *gorm.DB) error {
		if err := deleteProjectSource(tx, p); err != nil {
			return err
		}
		return tx.Delete(&domain.BasecampProject{}, "id = ?", p.ID).Error
	})
}

// projectSourceRef: cómo se nombra el proyecto entre las fuentes del espacio.
// La URL si la hay (la lee la puesta al día); si no, su id.
func projectSourceRef(p *domain.BasecampProject) string {
	if p.AppURL != "" {
		return p.AppURL
	}
	return strconv.FormatInt(p.ProjectID, 10)
}

func deleteProjectSource(tx *gorm.DB, p *domain.BasecampProject) error {
	return tx.Where("space_id = ? AND system = ? AND ref = ?", p.SpaceID, domain.SystemBasecamp, projectSourceRef(p)).
		Delete(&domain.SpaceSource{}).Error
}

// SpaceInOrg: el espacio es de la organización.
func (r *BasecampRepository) SpaceInOrg(spaceID, orgID string) bool {
	var n int64
	r.db.Model(&domain.TaskSpace{}).Where("id = ? AND org_id = ?", spaceID, orgID).Count(&n)
	return n > 0
}

// ─── Personas ────────────────────────────────────────────────────────────────

func (r *BasecampRepository) People(orgID string) ([]domain.BasecampPerson, error) {
	out := []domain.BasecampPerson{}
	err := r.db.Where("org_id = ?", orgID).Order("user_id <> '' ASC, name ASC").Find(&out).Error
	return out, err
}

func (r *BasecampRepository) CountUnmatched(orgID string) int64 {
	var n int64
	r.db.Model(&domain.BasecampPerson{}).Where("org_id = ? AND (user_id = '' OR user_id IS NULL)", orgID).Count(&n)
	return n
}

// UpsertPerson apunta (o pone al día) a una persona de Basecamp y la casa por
// correo con un miembro de la org si aún no tiene usuario. Una persona casada
// a mano no la mueve nadie más que un admin.
func (r *BasecampRepository) UpsertPerson(orgID string, w domain.BasecampPersonWire) (*domain.BasecampPerson, error) {
	if w.ID == 0 {
		return nil, gorm.ErrRecordNotFound
	}
	email := strings.ToLower(strings.TrimSpace(w.EmailAddress))
	var p domain.BasecampPerson
	err := r.db.First(&p, "org_id = ? AND person_id = ?", orgID, w.ID).Error
	switch {
	case errors.Is(err, gorm.ErrRecordNotFound):
		p = domain.BasecampPerson{OrgID: orgID, PersonID: w.ID}
		p.ID = uuid.NewString()
	case err != nil:
		return nil, err
	}
	if w.Name != "" {
		p.Name = w.Name
	}
	if email != "" {
		p.Email = email
	}
	if w.AttachableSGID != "" {
		p.SGID = w.AttachableSGID
	}
	p.Client = w.Client
	if p.UserID == "" && p.Email != "" {
		if uid := r.memberByEmail(orgID, p.Email); uid != "" {
			p.UserID, p.MatchedBy = uid, "email"
		}
	}
	if err := r.db.Clauses(clause.OnConflict{
		Columns:   []clause.Column{{Name: "org_id"}, {Name: "person_id"}},
		DoUpdates: clause.AssignmentColumns([]string{"name", "email", "sgid", "client", "user_id", "matched_by", "updated_at"}),
	}).Create(&p).Error; err != nil {
		return nil, err
	}
	return &p, nil
}

// memberByEmail: el miembro de la org con ese correo, o "" si no hay
// exactamente uno.
func (r *BasecampRepository) memberByEmail(orgID, email string) string {
	var ids []string
	r.db.Raw(`SELECT u.id FROM users u JOIN org_memberships m ON m.user_id = u.id
		WHERE m.org_id = ? AND LOWER(u.email) = ?`, orgID, email).Scan(&ids)
	if len(ids) != 1 {
		return ""
	}
	return ids[0]
}

// IsMember: el usuario es de la org.
func (r *BasecampRepository) IsMember(orgID, userID string) bool {
	var n int64
	r.db.Raw(`SELECT COUNT(*) FROM org_memberships WHERE org_id = ? AND user_id = ?`, orgID, userID).Scan(&n)
	return n > 0
}

// MatchPerson: un admin casa (o suelta, con "") a una persona.
func (r *BasecampRepository) MatchPerson(orgID string, personID int64, userID string) (*domain.BasecampPerson, error) {
	matched := ""
	if userID != "" {
		matched = "manual"
	}
	res := r.db.Model(&domain.BasecampPerson{}).Where("org_id = ? AND person_id = ?", orgID, personID).
		Updates(map[string]any{"user_id": userID, "matched_by": matched, "updated_at": time.Now().UTC()})
	if res.Error != nil {
		return nil, res.Error
	}
	if res.RowsAffected == 0 {
		return nil, gorm.ErrRecordNotFound
	}
	var p domain.BasecampPerson
	err := r.db.First(&p, "org_id = ? AND person_id = ?", orgID, personID).Error
	return &p, err
}

// PersonForMention: a quién nombra una mención, por su sgid o por el id que
// el sgid deja leer.
func (r *BasecampRepository) PersonForMention(orgID string, m domain.BasecampMention) (*domain.BasecampPerson, error) {
	var p domain.BasecampPerson
	q := r.db.Where("org_id = ?", orgID)
	if m.PersonID != 0 {
		q = q.Where("sgid = ? OR person_id = ?", m.SGID, m.PersonID)
	} else {
		q = q.Where("sgid = ?", m.SGID)
	}
	if err := q.First(&p).Error; err != nil {
		return nil, err
	}
	return &p, nil
}

// ─── Enlaces a tareas ────────────────────────────────────────────────────────

func (r *BasecampRepository) LinkFor(orgID string, recordingID int64) (*domain.BasecampLink, error) {
	var l domain.BasecampLink
	if err := r.db.First(&l, "org_id = ? AND recording_id = ?", orgID, recordingID).Error; err != nil {
		return nil, err
	}
	return &l, nil
}

func (r *BasecampRepository) LinksOfTask(itemID string) ([]domain.BasecampLink, error) {
	out := []domain.BasecampLink{}
	err := r.db.Where("item_id = ?", itemID).Order("created_at ASC").Find(&out).Error
	return out, err
}

// CreateLink: un recording ya enlazado a otra tarea es ErrBasecampLinkTaken;
// a la misma, no cambia nada.
func (r *BasecampRepository) CreateLink(l *domain.BasecampLink) error {
	if old, err := r.LinkFor(l.OrgID, l.RecordingID); err == nil {
		if old.ItemID != l.ItemID {
			return domain.ErrBasecampLinkTaken
		}
		*l = *old
		return nil
	}
	if l.ID == "" {
		l.ID = uuid.NewString()
	}
	return r.db.Create(l).Error
}

func (r *BasecampRepository) DeleteLink(itemID string, recordingID int64) error {
	res := r.db.Where("item_id = ? AND recording_id = ?", itemID, recordingID).Delete(&domain.BasecampLink{})
	if res.Error == nil && res.RowsAffected == 0 {
		return gorm.ErrRecordNotFound
	}
	return res.Error
}

// Assignees: los responsables de una tarea.
func (r *BasecampRepository) Assignees(itemID string) ([]string, error) {
	var ids []string
	err := r.db.Raw(`SELECT user_id FROM item_assignees WHERE item_id = ? ORDER BY user_id`, itemID).Scan(&ids).Error
	return ids, err
}

// Item: la tarea enlazada (sin borrar).
func (r *BasecampRepository) Item(id string) (*domain.Item, error) {
	var it domain.Item
	if err := r.db.First(&it, "id = ?", id).Error; err != nil {
		return nil, err
	}
	return &it, nil
}

// AddSourcedComment: como el de GitHub, una línea por tarea y procedencia
// (índice `idx_item_comments_source`).
func (r *BasecampRepository) AddSourcedComment(c *domain.ItemComment) (bool, error) {
	res := r.db.Clauses(clause.OnConflict{
		Columns:     []clause.Column{{Name: "item_id"}, {Name: "source_key"}},
		TargetWhere: clause.Where{Exprs: []clause.Expression{clause.Expr{SQL: "source_key <> ''"}}},
		DoNothing:   true,
	}).Create(c)
	return res.RowsAffected == 1, res.Error
}

// ─── Eventos vistos ──────────────────────────────────────────────────────────

// ClaimEvent apunta el evento y dice si es la primera vez que llega.
func (r *BasecampRepository) ClaimEvent(orgID string, eventID int64) (bool, error) {
	res := r.db.Clauses(clause.OnConflict{DoNothing: true}).
		Create(&domain.BasecampEvent{OrgID: orgID, EventID: eventID, ReceivedAt: time.Now().UTC()})
	return res.RowsAffected == 1, res.Error
}

// ReleaseEvent: el evento no se pudo atender; que el reintento de Basecamp
// vuelva a entrar.
func (r *BasecampRepository) ReleaseEvent(orgID string, eventID int64) error {
	return r.db.Where("org_id = ? AND event_id = ?", orgID, eventID).Delete(&domain.BasecampEvent{}).Error
}

// AdminIDs: los admins de la org, para cuando la conexión deja de renovarse.
func (r *BasecampRepository) AdminIDs(orgID string) ([]string, error) {
	var ids []string
	err := r.db.Raw(`SELECT user_id FROM org_memberships WHERE org_id = ? AND role = ? ORDER BY user_id`,
		orgID, domain.OrgRoleAdmin).Scan(&ids).Error
	return ids, err
}

// ─── Tareas que trae Basecamp (fase 3, command-and-control-557) ──────────────

// FirstListOf: la lista donde caen las tareas que trae Basecamp a un espacio:
// la primera del espacio, las de fuera de carpetas antes. "" si no tiene.
func (r *BasecampRepository) FirstListOf(spaceID string) string {
	var ids []string
	r.db.Raw(`SELECT id FROM task_lists WHERE space_id = ?
		ORDER BY (folder_id IS NULL) DESC, rank ASC, created_at ASC, id ASC LIMIT 1`, spaceID).Scan(&ids)
	if len(ids) == 0 {
		return ""
	}
	return ids[0]
}

// CreateLinkedTask crea la tarea y su enlace al recording en una sola
// transacción, o devuelve la que ya estaba enlazada (created=false).
//
// Quien llama ya miró el enlace; lo que queda es la carrera: dos entregas a
// la vez del mismo to-do (el alta y la asignación llegan casi juntas), o
// alguien que lo enlaza a mano entre medias. El índice único de
// basecamp_links tumba la segunda transacción entera, tarea incluida, y se
// contesta con la tarea que ganó.
func (r *BasecampRepository) CreateLinkedTask(t *domain.Item, l *domain.BasecampLink) (*domain.Item, bool, error) {
	err := r.db.Transaction(func(tx *gorm.DB) error {
		if t.ID == "" {
			t.ID = uuid.NewString()
		}
		if err := (&TaskRepository{db: tx}).createTaskRaw(t, t.SpaceID); err != nil {
			return err
		}
		l.ItemID = t.ID
		if l.ID == "" {
			l.ID = uuid.NewString()
		}
		return tx.Create(l).Error
	})
	if err != nil {
		if old, e := r.LinkFor(l.OrgID, l.RecordingID); e == nil {
			it, e := r.Item(old.ItemID)
			if errors.Is(e, gorm.ErrRecordNotFound) {
				// La que ganó ya se borró: sin tarea, como si no hubiera.
				return nil, false, nil
			}
			return it, false, e
		}
		return nil, false, err
	}
	// Lo que hace CreateTask después de crear: la búsqueda y las claves de
	// fuera. Fuera de la transacción, como allí.
	syncText(r.db, domain.EntityTask, t.ID)
	syncTaskKeys(r.db, t.ID)
	return t, true, nil
}

// AddBasecampAssignee pone al usuario de responsable y apunta que lo puso
// Basecamp. Si ya lo era (lo puso el equipo), no apunta nada: así Basecamp no
// lo quita nunca. Dice si entró ahora.
func (r *BasecampRepository) AddBasecampAssignee(orgID, itemID, userID string) (bool, error) {
	added := false
	err := r.db.Transaction(func(tx *gorm.DB) error {
		res := tx.Clauses(clause.OnConflict{DoNothing: true}).Create(&domain.ItemAssignee{ItemID: itemID, UserID: userID})
		if res.Error != nil {
			return res.Error
		}
		if res.RowsAffected == 0 {
			return nil
		}
		added = true
		return tx.Clauses(clause.OnConflict{DoNothing: true}).Create(&domain.BasecampAssignment{
			ItemID: itemID, UserID: userID, OrgID: orgID, CreatedAt: time.Now().UTC()}).Error
	})
	return added, err
}

// BasecampAssignees: los responsables de la tarea que puso Basecamp.
func (r *BasecampRepository) BasecampAssignees(itemID string) ([]string, error) {
	var ids []string
	err := r.db.Model(&domain.BasecampAssignment{}).Where("item_id = ?", itemID).Order("user_id").Pluck("user_id", &ids).Error
	return ids, err
}

// RemoveBasecampAssignee quita al responsable sólo si lo puso Basecamp.
func (r *BasecampRepository) RemoveBasecampAssignee(itemID, userID string) (bool, error) {
	removed := false
	err := r.db.Transaction(func(tx *gorm.DB) error {
		res := tx.Where("item_id = ? AND user_id = ?", itemID, userID).Delete(&domain.BasecampAssignment{})
		if res.Error != nil || res.RowsAffected == 0 {
			return res.Error
		}
		res = tx.Where("item_id = ? AND user_id = ?", itemID, userID).Delete(&domain.ItemAssignee{})
		removed = res.RowsAffected > 0
		return res.Error
	})
	return removed, err
}

// SetWebhookTypes apunta con qué tipos quedó el webhook del proyecto.
func (r *BasecampRepository) SetWebhookTypes(id, types string) error {
	return r.db.Model(&domain.BasecampProject{}).Where("id = ?", id).
		Updates(map[string]any{"webhook_types": types, "updated_at": time.Now().UTC()}).Error
}

// AllProjects: los proyectos enlazados de todas las orgs, para el reloj.
func (r *BasecampRepository) AllProjects() ([]domain.BasecampProject, error) {
	var out []domain.BasecampProject
	err := r.db.Order("org_id, project_id").Find(&out).Error
	return out, err
}
