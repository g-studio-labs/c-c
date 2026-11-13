package repository

import (
	"regexp"
	"time"

	"github.com/google/uuid"
	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// SpaceSourceRepository: las fuentes de cada espacio y lo que de ellas sale al
// grafo (ver domain/space_source.go, command-and-control-354).
type SpaceSourceRepository struct{ db *gorm.DB }

func NewSpaceSourceRepository(db *gorm.DB) *SpaceSourceRepository {
	return &SpaceSourceRepository{db: db}
}

// List: las fuentes de un espacio, en el orden en que se apuntaron.
func (r *SpaceSourceRepository) List(spaceID string) ([]domain.SpaceSource, error) {
	out := []domain.SpaceSource{}
	err := r.db.Where("space_id = ?", spaceID).Order("created_at ASC, id ASC").Find(&out).Error
	return out, err
}

// PageInOrg dice si esa página de documentación existe en la organización.
func (r *SpaceSourceRepository) PageInOrg(orgID, pageID string) bool {
	var n int64
	r.db.Model(&domain.DocPage{}).Where("id = ? AND org_id = ?", pageID, orgID).Count(&n)
	return n > 0
}

// Replace deja en el espacio exactamente esas fuentes. La que ya estaba —por
// su id, o por su papel, sistema y ref— conserva su cursor: mandar la lista
// entera otra vez no hace releer nada. Después recalcula lo que los textos del
// espacio nombran, porque puede haber cambiado el registro o su patrón.
func (r *SpaceSourceRepository) Replace(orgID, spaceID string, in []domain.SpaceSourceInput) ([]domain.SpaceSource, error) {
	err := r.db.Transaction(func(tx *gorm.DB) error {
		var had []domain.SpaceSource
		if err := tx.Where("space_id = ?", spaceID).Find(&had).Error; err != nil {
			return err
		}
		kept := map[string]bool{}
		now := time.Now().UTC()
		for i, x := range in {
			var old *domain.SpaceSource
			for j := range had {
				h := &had[j]
				if kept[h.ID] {
					continue
				}
				if (x.ID != "" && h.ID == x.ID) || (x.ID == "" && h.Role == x.Role && h.System == x.System && h.Ref == x.Ref) {
					old = h
					break
				}
			}
			if x.ID != "" && old == nil {
				return domain.ErrSourceNotFound
			}
			if old != nil {
				kept[old.ID] = true
				if err := tx.Model(&domain.SpaceSource{}).Where("id = ?", old.ID).Updates(map[string]any{
					"role": x.Role, "system": x.System, "ref": x.Ref, "pattern": x.Pattern, "note": x.Note, "updated_at": now,
				}).Error; err != nil {
					return err
				}
				continue
			}
			// La hora de alta ordena la lista: una por fuente, en el orden dado.
			at := now.Add(time.Duration(i) * time.Microsecond)
			if err := tx.Create(&domain.SpaceSource{ID: uuid.NewString(), OrgID: orgID, SpaceID: spaceID, Role: x.Role,
				System: x.System, Ref: x.Ref, Pattern: x.Pattern, Note: x.Note, CreatedAt: at, UpdatedAt: at}).Error; err != nil {
				return err
			}
		}
		for _, h := range had {
			if !kept[h.ID] {
				if err := tx.Delete(&domain.SpaceSource{}, "id = ?", h.ID).Error; err != nil {
					return err
				}
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	ResyncSpaceRules(r.db, spaceID)
	return r.List(spaceID)
}

// MarkSynced mueve el cursor de una fuente del espacio.
func (r *SpaceSourceRepository) MarkSynced(spaceID, sourceID string, at time.Time) (*domain.SpaceSource, error) {
	res := r.db.Model(&domain.SpaceSource{}).Where("id = ? AND space_id = ?", sourceID, spaceID).
		Updates(map[string]any{"last_sync_at": at.UTC(), "updated_at": time.Now().UTC()})
	if res.Error != nil {
		return nil, res.Error
	}
	if res.RowsAffected == 0 {
		return nil, domain.ErrSourceNotFound
	}
	var s domain.SpaceSource
	if err := r.db.First(&s, "id = ?", sourceID).Error; err != nil {
		return nil, err
	}
	return &s, nil
}

// ruleBook: un registro de reglas listo para buscar: su patrón y a qué
// sección lleva cada regla.
type ruleBook struct {
	PageID   string
	Pattern  string
	re       *regexp.Regexp
	sections map[string]string
}

// ruleBooks: los registros de un espacio. Un patrón que ya no compila (se
// guardó con otra versión) se salta: no nombra nada, pero no tira un guardado.
//
// Todo dentro de `isolated`: quien llama suele estar en la transacción de un
// guardado, y en Postgres una consulta que falla ahí la deja inservible.
func ruleBooks(db *gorm.DB, spaceID string) []ruleBook {
	if spaceID == "" {
		return nil
	}
	var out []ruleBook
	isolated(db, func(tx *gorm.DB) error {
		var err error
		out, err = readRuleBooks(tx, spaceID)
		return err
	})
	return out
}

// isolated corre unas lecturas en su propia transacción (un punto de
// guardado, si ya había una): si alguna falla, se deshace hasta ahí y la de
// fuera sigue viva. Un enlace de menos no puede tirar un guardado.
func isolated(db *gorm.DB, fn func(tx *gorm.DB) error) {
	_ = db.Transaction(fn)
}

func readRuleBooks(db *gorm.DB, spaceID string) ([]ruleBook, error) {
	var rows []domain.SpaceSource
	if err := db.Where("space_id = ? AND role = ?", spaceID, domain.SourceRules).Order("created_at ASC, id ASC").
		Find(&rows).Error; err != nil {
		return nil, err
	}
	var out []ruleBook
	for _, s := range rows {
		re, err := domain.CompileRulePattern(s.Pattern)
		if err != nil {
			continue
		}
		b := ruleBook{PageID: s.Ref, Pattern: s.Pattern, re: re, sections: map[string]string{}}
		var secs []domain.DocSection
		if err := db.Where("page_id = ? AND removed_at IS NULL", s.Ref).Order("ordinal ASC").Find(&secs).Error; err != nil {
			return nil, err
		}
		for _, sec := range secs {
			// La primera sección que abre con la regla es la regla: una
			// repetida más abajo no le quita los enlaces.
			if rule := domain.RuleOfHeading(re, sec.Heading); rule != "" && b.sections[rule] == "" {
				b.sections[rule] = sec.ID
			}
		}
		out = append(out, b)
	}
	return out, nil
}

// ruleSections: las secciones del registro que nombra un texto del espacio.
// El propio registro no se nombra a sí mismo: «Reemplaza a R-005» dentro de
// R-012 es prosa de la página, no una arista de la página a su sección.
func ruleSections(db *gorm.DB, spaceID string, fromKind domain.EntityKind, fromID, text string) []string {
	var out []string
	for _, b := range ruleBooks(db, spaceID) {
		if fromKind == domain.EntityPage && fromID == b.PageID {
			continue
		}
		for _, rule := range domain.ExtractRules(b.re, text) {
			if id := b.sections[rule]; id != "" {
				out = append(out, id)
			}
		}
	}
	return out
}

// docsOfSpace: los documentos que cuelgan de un espacio, de sus carpetas o de
// sus listas.
const docsOfSpace = `SELECT d.id FROM docs d WHERE (d.owner_kind = 'space' AND d.owner_id = @space)
	OR (d.owner_kind = 'folder' AND d.owner_id IN (SELECT id FROM task_folders WHERE space_id = @space))
	OR (d.owner_kind = 'list' AND d.owner_id IN (SELECT id FROM task_lists WHERE space_id = @space))`

// SpaceOf: el espacio al que pertenece una cosa del grafo, o vacío.
func (r *GraphRepository) SpaceOf(kind domain.EntityKind, id string) string {
	var space string
	const ofDoc = `SELECT COALESCE(CASE d.owner_kind WHEN 'space' THEN d.owner_id
			WHEN 'folder' THEN (SELECT space_id FROM task_folders WHERE id = d.owner_id)
			WHEN 'list' THEN (SELECT space_id FROM task_lists WHERE id = d.owner_id) END, '')
		FROM docs d JOIN `
	switch kind {
	case domain.EntityTask:
		r.db.Raw(`SELECT COALESCE(space_id, '') FROM items WHERE id = ?`, id).Scan(&space)
	case domain.EntityPage:
		r.db.Raw(ofDoc+`doc_pages x ON x.doc_id = d.id WHERE x.id = ?`, id).Scan(&space)
	case domain.EntityTab:
		r.db.Raw(ofDoc+`doc_tabs x ON x.doc_id = d.id WHERE x.id = ?`, id).Scan(&space)
	case domain.EntityDecision:
		r.db.Raw(ofDoc+`decisions x ON x.doc_id = d.id WHERE x.id = ?`, id).Scan(&space)
	case domain.EntityIncident:
		r.db.Raw(ofDoc+`incidents x ON x.doc_id = d.id WHERE x.id = ?`, id).Scan(&space)
	}
	return space
}

// ResyncSpaceRules recalcula lo que nombran los textos de un espacio que
// pueden nombrar una regla: los que casan con algún patrón del espacio, y los
// que ya tenían una arista a una sección (el patrón cambió, o el registro se
// quitó). Se llama al guardar las fuentes y al guardar el registro: una regla
// citada en un comentario antes de existir su sección se enlaza cuando la
// sección llega. Como todo lo del grafo, no falla hacia fuera.
func ResyncSpaceRules(db *gorm.DB, spaceID string) {
	if spaceID == "" {
		return
	}
	type ent struct {
		Kind domain.EntityKind
		ID   string
	}
	todo := map[ent]bool{}
	// Lo que ya apuntaba a una sección desde este espacio.
	var linked []ent
	isolated(db, func(tx *gorm.DB) error {
		return tx.Raw(`SELECT from_kind AS kind, from_id AS id FROM graph_links WHERE to_kind = ? AND source = ?
			AND org_id = (SELECT org_id FROM task_spaces WHERE id = ?)`,
			domain.EntitySection, domain.LinkSourceText, spaceID).Scan(&linked).Error
	})
	g := NewGraphRepository(db)
	for _, e := range linked {
		if g.SpaceOf(e.Kind, e.ID) == spaceID {
			todo[e] = true
		}
	}
	for _, b := range ruleBooks(db, spaceID) {
		// El filtro lo hace Postgres con el patrón tal cual, sin los límites de
		// palabra: trae de más, y quien decide es `ruleSections`.
		args := map[string]any{"space": spaceID, "re": b.Pattern}
		for _, q := range []struct {
			kind domain.EntityKind
			sql  string
		}{
			{domain.EntityTask, `SELECT i.id FROM items i WHERE i.space_id = @space AND i.deleted_at IS NULL
				AND (COALESCE(i.description, '') ~ @re OR EXISTS (SELECT 1 FROM item_comments c
					WHERE c.item_id = i.id AND c.deleted_at IS NULL AND c.body ~ @re))`},
			{domain.EntityPage, `SELECT x.id FROM doc_pages x WHERE x.deleted_at IS NULL AND COALESCE(x.body, '') ~ @re AND x.doc_id IN (` + docsOfSpace + `)`},
			{domain.EntityTab, `SELECT x.id FROM doc_tabs x WHERE COALESCE(x.body, '') ~ @re AND x.doc_id IN (` + docsOfSpace + `)`},
			{domain.EntityDecision, `SELECT x.id FROM decisions x WHERE (x.title || ' ' || COALESCE(x.body, '')) ~ @re AND x.doc_id IN (` + docsOfSpace + `)`},
			{domain.EntityIncident, `SELECT x.id FROM incidents x WHERE (x.title || ' ' || COALESCE(x.body, '')) ~ @re AND x.doc_id IN (` + docsOfSpace + `)`},
		} {
			// Un patrón que Go acepta y Postgres no falla aquí: se salta.
			var ids []string
			isolated(db, func(tx *gorm.DB) error { return tx.Raw(q.sql, args).Scan(&ids).Error })
			for _, id := range ids {
				todo[ent{q.kind, id}] = true
			}
		}
	}
	for e := range todo {
		syncText(db, e.Kind, e.ID)
	}
}

// resyncRulesOfPage: si la página es el registro de algún espacio, recalcula
// esos espacios. Se llama al guardarla: sus secciones acaban de cambiar.
func resyncRulesOfPage(db *gorm.DB, pageID string) {
	if pageID == "" {
		return
	}
	var spaces []string
	isolated(db, func(tx *gorm.DB) error {
		return tx.Raw(`SELECT DISTINCT space_id FROM space_sources WHERE role = ? AND ref = ?`,
			domain.SourceRules, pageID).Scan(&spaces).Error
	})
	for _, sp := range spaces {
		ResyncSpaceRules(db, sp)
	}
}

// FindRule: las secciones a las que lleva una regla en esas organizaciones
// (todas, con `any`). Una por registro que la tenga: lo normal es una.
func (r *SpaceSourceRepository) FindRule(orgs []string, any bool, rule string) []string {
	q := r.db.Model(&domain.SpaceSource{}).Where("role = ?", domain.SourceRules)
	if !any {
		if len(orgs) == 0 {
			return nil
		}
		q = q.Where("org_id IN ?", orgs)
	}
	var spaces []string
	if err := q.Distinct().Pluck("space_id", &spaces).Error; err != nil {
		return nil
	}
	var out []string
	seen := map[string]bool{}
	for _, sp := range spaces {
		for _, b := range ruleBooks(r.db, sp) {
			if id := b.sections[rule]; id != "" && !seen[id] {
				seen[id] = true
				out = append(out, id)
			}
		}
	}
	return out
}
