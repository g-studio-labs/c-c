package repository

import (
	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// Lo que el texto de cada cosa nombra se guarda al escribirla (ver
// GraphRepository.SyncTextLinks). Los guardados de abajo envuelven a los de
// siempre (`…Raw`) y, si salen bien, recalculan los enlaces con el **mismo**
// manejador de base: dentro de una transacción ve sus propias filas.
// Un enlace de menos no tira un guardado: SyncTextLinks nunca falla hacia fuera.

// syncText recalcula lo que nombra una cosa, leyendo su texto de la base.
func syncText(db *gorm.DB, kind domain.EntityKind, id string) {
	if id == "" {
		return
	}
	var text string
	switch kind {
	case domain.EntityTask:
		// La descripción y lo que dice el hilo (sin las notas de commits de
		// GitHub, que nombran la tarea a sí misma): para cac, lo que el hilo de
		// una tarea nombra lo nombra la tarea.
		db.Raw(`SELECT COALESCE(i.description, '') || ' ' || COALESCE((SELECT string_agg(c.body, ' ') FROM item_comments c
			WHERE c.item_id = i.id AND c.deleted_at IS NULL AND COALESCE(c.source_key, '') = ''), '')
			FROM items i WHERE i.id = ?`, id).Scan(&text)
	case domain.EntityPage:
		db.Raw(`SELECT COALESCE(body, '') FROM doc_pages WHERE id = ?`, id).Scan(&text)
	case domain.EntityTab:
		db.Raw(`SELECT COALESCE(body, '') FROM doc_tabs WHERE id = ?`, id).Scan(&text)
	case domain.EntityDecision:
		db.Raw(`SELECT title || ' ' || COALESCE(body, '') FROM decisions WHERE id = ?`, id).Scan(&text)
	case domain.EntityIncident:
		db.Raw(`SELECT title || ' ' || COALESCE(body, '') FROM incidents WHERE id = ?`, id).Scan(&text)
	default:
		return
	}
	g := NewGraphRepository(db)
	g.SyncTextLinks(g.OrgOf(kind, id), kind, id, text)
}

func syncTabText(db *gorm.DB, d *domain.Doc, key domain.DocTabKey) {
	if d == nil {
		return
	}
	var id string
	db.Raw(`SELECT id FROM doc_tabs WHERE doc_id = ? AND key = ?`, d.ID, key).Scan(&id)
	syncText(db, domain.EntityTab, id)
}

func (r *DocRepository) SaveTabRenaming(orgID string, kind domain.DocOwnerKind, ownerID string,
	key domain.DocTabKey, body, userID string, renames map[string]string) (*domain.Doc, error) {
	d, err := r.saveTabRenamingRaw(orgID, kind, ownerID, key, body, userID, renames)
	if err == nil {
		syncTabText(r.db, d, key)
	}
	return d, err
}

func (r *DocRepository) AppendTab(orgID string, kind domain.DocOwnerKind, ownerID string,
	key domain.DocTabKey, texto, userID string) (*domain.Doc, error) {
	d, err := r.appendTabRaw(orgID, kind, ownerID, key, texto, userID)
	if err == nil {
		syncTabText(r.db, d, key)
	}
	return d, err
}

func (r *DocRepository) AppendTabSection(orgID string, kind domain.DocOwnerKind, ownerID string,
	key domain.DocTabKey, section, texto, userID string) (*domain.Doc, error) {
	d, err := r.appendTabSectionRaw(orgID, kind, ownerID, key, section, texto, userID)
	if err == nil {
		syncTabText(r.db, d, key)
	}
	return d, err
}

func (r *DocRepository) AddDecision(d *domain.Decision) (*domain.Decision, error) {
	out, err := r.addDecisionRaw(d)
	if err == nil && out != nil {
		syncText(r.db, domain.EntityDecision, out.ID)
	}
	return out, err
}

func (r *DocRepository) CreatePage(orgID string, kind domain.DocOwnerKind, ownerID string,
	req domain.CreateDocPageRequest, userID string) (*domain.DocPage, error) {
	p, err := r.createPageRaw(orgID, kind, ownerID, req, userID)
	if err == nil && p != nil {
		syncText(r.db, domain.EntityPage, p.ID)
	}
	return p, err
}

func (r *DocRepository) SavePageRenaming(p *domain.DocPage, title, body *string, userID string, renames map[string]string) error {
	err := r.savePageRenamingRaw(p, title, body, userID, renames)
	if err == nil {
		syncText(r.db, domain.EntityPage, p.ID)
		// Si es el registro de reglas de un espacio, sus secciones cambiaron.
		resyncRulesOfPage(r.db, p.ID)
	}
	return err
}

func (r *DocRepository) AppendPage(pageID, texto, userID string) error {
	err := r.appendPageRaw(pageID, texto, userID)
	if err == nil {
		syncText(r.db, domain.EntityPage, pageID)
		resyncRulesOfPage(r.db, pageID)
	}
	return err
}

func (r *IncidentRepository) Create(in *domain.Incident, serverIDs, areas []string,
	commits []domain.IncidentCommit, guards []string) error {
	err := r.createRaw(in, serverIDs, areas, commits, guards)
	if err == nil {
		syncText(r.db, domain.EntityIncident, in.ID)
	}
	return err
}

func (r *TaskRepository) CreateTask(t *domain.Task, spaceID string) error {
	err := r.createTaskRaw(t, spaceID)
	if err == nil {
		syncText(r.db, domain.EntityTask, t.ID)
		syncTaskKeys(r.db, t.ID)
	}
	return err
}

func (r *TaskRepository) UpdateTask(id string, fields map[string]any) error {
	err := r.updateTaskRaw(id, fields)
	_, body := fields["description"]
	_, title := fields["title"]
	if err == nil && body {
		syncText(r.db, domain.EntityTask, id)
	}
	// Las claves de fuera salen del título y de la descripción.
	if err == nil && (body || title) {
		syncTaskKeys(r.db, id)
	}
	return err
}

func (r *TaskRepository) CreateComment(c *domain.TaskComment) error {
	err := r.createCommentRaw(c)
	if err == nil {
		syncText(r.db, domain.EntityTask, c.ItemID)
	}
	return err
}

func (r *TaskRepository) UpdateComment(id, body string) error {
	err := r.updateCommentRaw(id, body)
	if err == nil {
		var item string
		r.db.Raw(`SELECT item_id FROM item_comments WHERE id = ?`, id).Scan(&item)
		syncText(r.db, domain.EntityTask, item)
	}
	return err
}

// BackfillGraphLinks recorre una vez los textos que ya existían cuando nació
// `graph_links` y guarda lo que nombran. Sólo con la tabla vacía de enlaces de
// texto: después, cada escritura se ocupa de lo suyo.
func BackfillGraphLinks(db *gorm.DB) {
	var n int64
	if db.Model(&domain.GraphLink{}).Where("source = ?", domain.LinkSourceText).Count(&n); n > 0 {
		return
	}
	for _, x := range []struct {
		kind domain.EntityKind
		sql  string
	}{
		{domain.EntityTask, `SELECT id FROM items WHERE deleted_at IS NULL AND (COALESCE(description, '') <> '' OR EXISTS (SELECT 1 FROM item_comments c WHERE c.item_id = items.id))`},
		{domain.EntityPage, `SELECT id FROM doc_pages WHERE deleted_at IS NULL AND COALESCE(body, '') <> ''`},
		{domain.EntityTab, `SELECT id FROM doc_tabs WHERE COALESCE(body, '') <> ''`},
		{domain.EntityDecision, `SELECT id FROM decisions`},
		{domain.EntityIncident, `SELECT id FROM incidents`},
	} {
		var ids []string
		if err := db.Raw(x.sql).Scan(&ids).Error; err != nil {
			continue
		}
		for _, id := range ids {
			syncText(db, x.kind, id)
		}
	}
}
