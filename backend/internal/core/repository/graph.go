package repository

import (
	"fmt"
	"log"

	"github.com/google/uuid"
	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// GraphRepository: las aristas de `links` y la vecindad de cualquier cosa (ver
// domain/ontology.go, command-and-control-348).
type GraphRepository struct{ db *gorm.DB }

func NewGraphRepository(db *gorm.DB) *GraphRepository { return &GraphRepository{db: db} }

// SyncTextLinks deja en `links` lo que el texto de una cosa nombra —las refs de
// tareas (`proteus-12`) y los enlaces de la app a una tarea, página, decisión o
// incidente— y quita lo que ya no nombra. Se llama en cada escritura del
// texto: así «quién nombra esta tarea» es una consulta por índice y no un
// LIKE sobre todos los documentos.
//
// Sólo lo que existe y es de la misma org: una ref a una tarea que no está, o
// de otra org, no es una arista. Nunca falla hacia fuera: un enlace de menos
// no puede tirar un guardado.
func (r *GraphRepository) SyncTextLinks(orgID string, fromKind domain.EntityKind, fromID, text string) {
	if orgID == "" || fromID == "" {
		return
	}
	targets := map[domain.EntityRef]bool{}
	tasks := NewTaskRepository(r.db)
	for _, ref := range domain.ExtractTaskRefs(text, false) {
		if it, err := tasks.FindByRef([]string{orgID}, ref); err == nil {
			targets[domain.EntityRef{Kind: domain.EntityTask, ID: it.ID}] = true
		}
	}
	for _, e := range domain.ExtractEntityLinks(text) {
		if r.inOrg(orgID, e.Kind, e.ID) {
			targets[e] = true
		}
	}
	delete(targets, domain.EntityRef{Kind: fromKind, ID: fromID})
	err := r.db.Transaction(func(tx *gorm.DB) error {
		if err := tx.Where("from_kind = ? AND from_id = ? AND source = ?", fromKind, fromID, domain.LinkSourceText).
			Delete(&domain.GraphLink{}).Error; err != nil {
			return err
		}
		for t := range targets {
			if !domain.RelMentions.Allows(fromKind, t.Kind) {
				continue
			}
			if err := tx.Create(&domain.GraphLink{ID: uuid.NewString(), OrgID: orgID, FromKind: fromKind, FromID: fromID,
				Rel: domain.RelMentions, ToKind: t.Kind, ToID: t.ID, Source: domain.LinkSourceText}).Error; err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		log.Printf("graph: no se pudieron guardar los enlaces de %s %s: %v", fromKind, fromID, err)
	}
}

// AddLink guarda una arista puesta a mano (o por un agente). Repetirla no
// duplica.
func (r *GraphRepository) AddLink(orgID string, fromKind domain.EntityKind, fromID string, rel domain.Relation,
	toKind domain.EntityKind, toID, via string) error {
	if !rel.Allows(fromKind, toKind) {
		return fmt.Errorf("%s does not join a %s to a %s", rel, fromKind, toKind)
	}
	if !r.inOrg(orgID, fromKind, fromID) || !r.inOrg(orgID, toKind, toID) {
		return fmt.Errorf("both ends must exist in the same organization")
	}
	var n int64
	r.db.Model(&domain.GraphLink{}).Where("from_kind = ? AND from_id = ? AND rel = ? AND to_kind = ? AND to_id = ?",
		fromKind, fromID, rel, toKind, toID).Count(&n)
	if n > 0 {
		return nil
	}
	return r.db.Create(&domain.GraphLink{ID: uuid.NewString(), OrgID: orgID, FromKind: fromKind, FromID: fromID, Rel: rel,
		ToKind: toKind, ToID: toID, Source: domain.LinkSourceManual, Via: via}).Error
}

// OrgOf: la org de una cosa del grafo, o vacío si no existe.
func (r *GraphRepository) OrgOf(kind domain.EntityKind, id string) string {
	var org string
	switch kind {
	case domain.EntityTask:
		r.db.Raw(`SELECT org_id FROM items WHERE id = ? AND deleted_at IS NULL`, id).Scan(&org)
	case domain.EntityDecision, domain.EntityPage, domain.EntityTab:
		table := map[domain.EntityKind]string{domain.EntityDecision: "decisions", domain.EntityPage: "doc_pages", domain.EntityTab: "doc_tabs"}[kind]
		extra := ""
		if kind == domain.EntityPage {
			extra = " AND x.deleted_at IS NULL"
		}
		r.db.Raw(`SELECT d.org_id FROM `+table+` x JOIN docs d ON d.id = x.doc_id WHERE x.id = ?`+extra, id).Scan(&org)
	case domain.EntityIncident:
		r.db.Raw(`SELECT org_id FROM incidents WHERE id = ?`, id).Scan(&org)
	case domain.EntityServer:
		r.db.Raw(`SELECT COALESCE(org_id, '') FROM servers WHERE id = ?`, id).Scan(&org)
	case domain.EntitySection:
		r.db.Raw(`SELECT d.org_id FROM doc_sections s JOIN docs d ON d.id = s.doc_id WHERE s.id = ?`, id).Scan(&org)
	}
	return org
}

func (r *GraphRepository) inOrg(orgID string, kind domain.EntityKind, id string) bool {
	return orgID != "" && r.OrgOf(kind, id) == orgID
}

// edge: una arista cruda, antes de ponerle título.
type edge struct {
	Rel  domain.Relation
	Dir  string
	Kind domain.EntityKind
	ID   string
}

// Neighbors: todo lo que toca a una cosa, en los dos sentidos, juntando
// `links` con las tablas de aristas que ya existían. Cada vecino una vez por
// relación y sentido, con título, estado y ref; lo borrado no sale.
func (r *GraphRepository) Neighbors(orgID string, kind domain.EntityKind, id string) ([]domain.Neighbor, bool, error) {
	var edges []edge
	q := func(sql string, args ...any) error {
		var rows []edge
		if err := r.db.Raw(sql, args...).Scan(&rows).Error; err != nil {
			return err
		}
		edges = append(edges, rows...)
		return nil
	}
	// Las de `links`, en los dos sentidos.
	if err := q(`SELECT rel, 'out' AS dir, to_kind AS kind, to_id AS id FROM graph_links WHERE org_id = ? AND from_kind = ? AND from_id = ?
		UNION SELECT rel, 'in', from_kind, from_id FROM graph_links WHERE org_id = ? AND to_kind = ? AND to_id = ?`,
		orgID, kind, id, orgID, kind, id); err != nil {
		return nil, false, err
	}
	var err error
	switch kind {
	case domain.EntityTask:
		err = q(`SELECT 'applies_to' AS rel, 'in' AS dir, 'decision' AS kind, decision_id AS id FROM decision_links WHERE task_id = ? AND removed_at IS NULL
			UNION SELECT 'origin', 'in', 'incident', id FROM incidents WHERE origin_task_id = ?
			UNION SELECT 'blocks', 'out', 'task', item_id FROM item_blocks WHERE blocked_by_id = ?
			UNION SELECT 'blocks', 'in', 'task', blocked_by_id FROM item_blocks WHERE item_id = ?
			UNION SELECT 'parent_of', 'out', 'task', id FROM items WHERE parent_id = ? AND deleted_at IS NULL
			UNION SELECT 'parent_of', 'in', 'task', parent_id FROM items WHERE id = ? AND parent_id IS NOT NULL AND parent_id <> ''`,
			id, id, id, id, id, id)
	case domain.EntityDecision:
		err = q(`SELECT 'applies_to' AS rel, 'out' AS dir, CASE WHEN task_id IS NOT NULL THEN 'task' ELSE 'section' END AS kind,
				COALESCE(task_id, section_id) AS id FROM decision_links WHERE decision_id = ? AND removed_at IS NULL
			UNION SELECT 'supersedes', 'out', 'decision', supersedes_id FROM decisions WHERE id = ? AND supersedes_id IS NOT NULL AND supersedes_id <> ''
			UNION SELECT 'supersedes', 'in', 'decision', id FROM decisions WHERE supersedes_id = ?`, id, id, id)
	case domain.EntityIncident:
		err = q(`SELECT 'origin' AS rel, 'out' AS dir, 'task' AS kind, origin_task_id AS id FROM incidents WHERE id = ? AND origin_task_id IS NOT NULL AND origin_task_id <> ''
			UNION SELECT 'bit_on', 'out', 'server', server_id FROM incident_servers WHERE incident_id = ?
			UNION SELECT 'supersedes', 'out', 'incident', supersedes_id FROM incidents WHERE id = ? AND supersedes_id IS NOT NULL AND supersedes_id <> ''
			UNION SELECT 'supersedes', 'in', 'incident', id FROM incidents WHERE supersedes_id = ?`, id, id, id, id)
	case domain.EntityServer:
		err = q(`SELECT 'bit_on' AS rel, 'in' AS dir, 'incident' AS kind, incident_id AS id FROM incident_servers WHERE server_id = ?`, id)
	case domain.EntitySection:
		err = q(`SELECT 'applies_to' AS rel, 'in' AS dir, 'decision' AS kind, decision_id AS id FROM decision_links WHERE section_id = ? AND removed_at IS NULL`, id)
	}
	if err != nil {
		return nil, false, err
	}
	out, err := r.describe(edges)
	if err != nil {
		return nil, false, err
	}
	truncated := len(out) > domain.NeighborsMax
	if truncated {
		out = out[:domain.NeighborsMax]
	}
	return out, truncated, nil
}

// described: lo que se enseña de cada cosa.
type described struct {
	ID, Title, Status, Ref, Where string
}

// Describe: título, estado y ref de una cosa sola (la raíz de `Related`).
func (r *GraphRepository) Describe(kind domain.EntityKind, id string) (string, bool) {
	got, err := r.titles(kind, []string{id})
	if err != nil || len(got) == 0 {
		return "", false
	}
	return got[id].Title, true
}

// describe pone título a cada arista, por tipo y de una consulta por tipo, en
// el orden de domain.EntityKinds; lo que no existe (borrado) se cae.
func (r *GraphRepository) describe(edges []edge) ([]domain.Neighbor, error) {
	byKind := map[domain.EntityKind][]string{}
	seen := map[edge]bool{}
	var uniq []edge
	for _, e := range edges {
		if e.ID == "" || seen[e] {
			continue
		}
		seen[e] = true
		uniq = append(uniq, e)
		byKind[e.Kind] = append(byKind[e.Kind], e.ID)
	}
	info := map[domain.EntityKind]map[string]described{}
	for k, ids := range byKind {
		got, err := r.titles(k, ids)
		if err != nil {
			return nil, err
		}
		info[k] = got
	}
	out := []domain.Neighbor{}
	for _, k := range domain.EntityKinds {
		for _, e := range uniq {
			if e.Kind != k {
				continue
			}
			d, ok := info[k][e.ID]
			if !ok {
				continue
			}
			out = append(out, domain.Neighbor{Rel: e.Rel, Dir: e.Dir, Kind: e.Kind, ID: e.ID, Title: d.Title,
				Status: d.Status, Ref: d.Ref, Where: d.Where})
		}
	}
	return out, nil
}

// nodeName: el nombre del nodo dueño de un documento.
const nodeName = `COALESCE(sp.name, f.name, l.name, '')`
const nodeJoins = ` LEFT JOIN task_spaces sp ON d.owner_kind = 'space' AND sp.id = d.owner_id
	LEFT JOIN task_folders f ON d.owner_kind = 'folder' AND f.id = d.owner_id
	LEFT JOIN task_lists l ON d.owner_kind = 'list' AND l.id = d.owner_id`

func (r *GraphRepository) titles(kind domain.EntityKind, ids []string) (map[string]described, error) {
	var rows []described
	var err error
	switch kind {
	case domain.EntityTask:
		err = r.db.Raw(`SELECT i.id, i.title, i.status,
				CASE WHEN p.slug IS NOT NULL THEN p.slug || '-' || i.seq
				     WHEN s.key IS NOT NULL AND s.key <> '' THEN s.key || '-' || i.seq ELSE '' END AS ref
			FROM items i
			LEFT JOIN report_projects p ON p.id = NULLIF(i.project_id, '')
			LEFT JOIN task_spaces s ON s.id = i.space_id
			WHERE i.id IN ? AND i.deleted_at IS NULL`, ids).Scan(&rows).Error
	case domain.EntityPage:
		err = r.db.Raw(`SELECT x.id, x.title, `+nodeName+` AS "where" FROM doc_pages x JOIN docs d ON d.id = x.doc_id`+nodeJoins+`
			WHERE x.id IN ? AND x.deleted_at IS NULL`, ids).Scan(&rows).Error
	case domain.EntityTab:
		err = r.db.Raw(`SELECT x.id, x.key AS title, `+nodeName+` AS "where" FROM doc_tabs x JOIN docs d ON d.id = x.doc_id`+nodeJoins+`
			WHERE x.id IN ?`, ids).Scan(&rows).Error
	case domain.EntitySection:
		err = r.db.Raw(`SELECT x.id, x.heading AS title, CASE WHEN x.removed_at IS NULL THEN '' ELSE 'removed' END AS status,
				`+nodeName+` AS "where" FROM doc_sections x JOIN docs d ON d.id = x.doc_id`+nodeJoins+`
			WHERE x.id IN ?`, ids).Scan(&rows).Error
	case domain.EntityDecision:
		err = r.db.Raw(`SELECT x.id, x.title,
				CASE WHEN EXISTS (SELECT 1 FROM decisions n WHERE n.supersedes_id = x.id) THEN 'superseded' ELSE 'in_force' END AS status
			FROM decisions x WHERE x.id IN ?`, ids).Scan(&rows).Error
	case domain.EntityIncident:
		err = r.db.Raw(`SELECT x.id, x.title,
				CASE WHEN EXISTS (SELECT 1 FROM incidents n WHERE n.supersedes_id = x.id) THEN 'superseded' ELSE x.status END AS status
			FROM incidents x WHERE x.id IN ?`, ids).Scan(&rows).Error
	case domain.EntityServer:
		err = r.db.Raw(`SELECT id, name AS title FROM servers WHERE id IN ?`, ids).Scan(&rows).Error
	}
	out := map[string]described{}
	for _, d := range rows {
		out[d.ID] = d
	}
	return out, err
}
