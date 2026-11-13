package domain

import (
	"regexp"
	"time"
)

// La ontología de cac: qué cosas hay y cómo se relacionan (decisión 7b3cc169,
// command-and-control-348).
//
// cac ya era un grafo —una decisión se aplica a tareas y secciones, un
// incidente muerde en servidores, una tarea bloquea a otra—, pero cada arista
// vivía en su tabla y se leía desde un solo lado. Aquí se nombran todas en un
// sitio y en un conjunto cerrado: `Neighbors` las junta, en los dos sentidos,
// y `GraphLink` guarda las que no tenían tabla (lo que un texto nombra, lo que se
// enlaza a mano). Un tipo o una relación nueva se añade aquí, y la prueba
// `TestEveryRelationIsDeclared` obliga a decir qué une.

// EntityKind: un tipo de cosa del grafo.
type EntityKind string

const (
	EntityTask     EntityKind = "task"
	EntityPage     EntityKind = "page"
	EntityTab      EntityKind = "tab" // una pestaña de la portada de un nodo (overview, runbook…)
	EntityDecision EntityKind = "decision"
	EntityIncident EntityKind = "incident"
	EntityServer   EntityKind = "server"
	EntitySection  EntityKind = "section"
)

// EntityKinds: todos, en el orden en que se enseñan.
var EntityKinds = []EntityKind{EntityTask, EntityDecision, EntityIncident, EntityPage, EntityTab, EntitySection, EntityServer}

func ValidEntityKind(k string) bool {
	for _, x := range EntityKinds {
		if string(x) == k {
			return true
		}
	}
	return false
}

// Relation: un tipo de arista. Se lee «origen REL destino».
type Relation string

const (
	// RelMentions: el texto del origen nombra al destino (una ref, un enlace,
	// o la regla del registro de su espacio).
	// Se recalcula en cada escritura del origen (`LinkSourceText`).
	RelMentions Relation = "mentions"
	// RelRelatesTo: alguien dijo que van juntas, sin más.
	RelRelatesTo Relation = "relates_to"
	// RelAppliesTo: una decisión se aplica a una tarea o a una sección
	// (`decision_links`).
	RelAppliesTo Relation = "applies_to"
	// RelOrigin: un incidente salió de una tarea (`incidents.origin_task_id`).
	RelOrigin Relation = "origin"
	// RelBlocks: el origen tiene que terminar antes que el destino (`item_blocks`).
	RelBlocks Relation = "blocks"
	// RelParentOf: una tarea y su subtarea (`items.parent_id`).
	RelParentOf Relation = "parent_of"
	// RelSupersedes: una decisión o un incidente que reemplaza a otro.
	RelSupersedes Relation = "supersedes"
	// RelBitOn: un incidente mordió en un servidor (`incident_servers`).
	RelBitOn Relation = "bit_on"
)

// RelationEnds: qué une cada relación (origen → destinos). Cerrado: una arista
// que no está aquí no se guarda ni se enseña.
var RelationEnds = map[Relation]struct{ From, To []EntityKind }{
	// Una sección se nombra por su regla (`R-012`): ver space_source.go.
	RelMentions:   {From: []EntityKind{EntityTask, EntityPage, EntityTab, EntityDecision, EntityIncident}, To: []EntityKind{EntityTask, EntityPage, EntityDecision, EntityIncident, EntitySection}},
	RelRelatesTo:  {From: EntityKinds, To: EntityKinds},
	RelAppliesTo:  {From: []EntityKind{EntityDecision}, To: []EntityKind{EntityTask, EntitySection, EntityIncident, EntityServer}},
	RelOrigin:     {From: []EntityKind{EntityIncident}, To: []EntityKind{EntityTask}},
	RelBlocks:     {From: []EntityKind{EntityTask}, To: []EntityKind{EntityTask}},
	RelParentOf:   {From: []EntityKind{EntityTask}, To: []EntityKind{EntityTask}},
	RelSupersedes: {From: []EntityKind{EntityDecision, EntityIncident}, To: []EntityKind{EntityDecision, EntityIncident}},
	RelBitOn:      {From: []EntityKind{EntityIncident}, To: []EntityKind{EntityServer}},
}

// Allows dice si la relación puede unir esos dos tipos.
func (r Relation) Allows(from, to EntityKind) bool {
	ends, ok := RelationEnds[r]
	if !ok {
		return false
	}
	has := func(l []EntityKind, k EntityKind) bool {
		for _, x := range l {
			if x == k {
				return true
			}
		}
		return false
	}
	return has(ends.From, from) && has(ends.To, to)
}

// De dónde salió una arista de `links`.
const (
	LinkSourceText   = "text"   // la sacó cac del texto del origen
	LinkSourceManual = "manual" // la puso alguien (una persona o un agente)
)

// GraphLink: una arista que no tiene tabla propia (`graph_links`). Las que sí la tienen
// (`decision_links`, `item_blocks`…) se quedan donde están y `Neighbors` las
// une: migrarlas no le daba nada a nadie y cada una tiene sus reglas y sus
// pruebas.
type GraphLink struct {
	ID        string     `gorm:"type:varchar(36);primaryKey" json:"id"`
	OrgID     string     `gorm:"type:varchar(36);not null;index" json:"-"`
	FromKind  EntityKind `gorm:"type:varchar(20);not null;index:idx_links_from" json:"fromKind"`
	FromID    string     `gorm:"type:varchar(36);not null;index:idx_links_from" json:"fromId"`
	Rel       Relation   `gorm:"type:varchar(20);not null" json:"rel"`
	ToKind    EntityKind `gorm:"type:varchar(20);not null;index:idx_links_to" json:"toKind"`
	ToID      string     `gorm:"type:varchar(36);not null;index:idx_links_to" json:"toId"`
	Source    string     `gorm:"type:varchar(10);not null" json:"source"`
	Via       string     `gorm:"type:varchar(20)" json:"via,omitempty"`
	CreatedAt time.Time  `json:"createdAt"`
}

func (GraphLink) TableName() string { return "graph_links" }

// Neighbor: algo relacionado con otra cosa, como se enseña: qué relación, en
// qué sentido (`out`: la cosa → el vecino; `in`: el vecino → la cosa), y lo
// justo para decidir si se abre.
type Neighbor struct {
	Rel    Relation   `json:"rel"`
	Dir    string     `json:"dir"`
	Kind   EntityKind `json:"kind"`
	ID     string     `json:"id"`
	Title  string     `json:"title"`
	Status string     `json:"status,omitempty"`
	Ref    string     `json:"ref,omitempty"`
	// Where: el nodo de una página, una pestaña o una sección.
	Where string `json:"where,omitempty"`
}

const (
	NeighborOut = "out"
	NeighborIn  = "in"
)

// NeighborsMax: lo más que se devuelve de una cosa; lo demás se recorta y se
// dice.
const NeighborsMax = 60

// Related: la respuesta de `GET /graph/{kind}/{id}`.
type Related struct {
	Kind      EntityKind `json:"kind"`
	ID        string     `json:"id"`
	Title     string     `json:"title"`
	Neighbors []Neighbor `json:"neighbors"`
	Truncated bool       `json:"truncated,omitempty"`
}

// Lo que el texto puede nombrar: un enlace de la app a una cosa por su id.
// Las refs (`proteus-12`) las saca ExtractTaskRefs.
var textEntityLink = regexp.MustCompile(`[?&](task|page|decision|incident)=([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})`)

// EntityRef: una cosa nombrada en un texto por su id.
type EntityRef struct {
	Kind EntityKind
	ID   string
}

// ExtractEntityLinks: las cosas que un texto enlaza por su id, cada una una vez.
func ExtractEntityLinks(text string) []EntityRef {
	var out []EntityRef
	seen := map[EntityRef]bool{}
	for _, m := range textEntityLink.FindAllStringSubmatch(text, -1) {
		r := EntityRef{Kind: EntityKind(m[1]), ID: m[2]}
		if !seen[r] {
			seen[r] = true
			out = append(out, r)
		}
	}
	return out
}

// SimilarTitleMin: desde cuánto parecido dos títulos se enseñan como «quizá
// es la misma» al apuntar una decisión o un incidente (command-and-control-351).
// Bajo a propósito: es una pregunta para el agente, no una fusión.
const SimilarTitleMin = 0.25

// SimilarMax: cuántos parecidos se enseñan, como mucho.
const SimilarMax = 3

// TitleSimilarity: cuánto se parecen dos títulos por sus palabras con
// contenido (cuatro letras o más: «de», «la», «que» no dicen de qué va), sin
// mayúsculas ni acentos. De 0 a 1.
func TitleSimilarity(a, b string) float64 {
	set := func(s string) map[string]bool {
		out := map[string]bool{}
		for _, w := range foldWords(s) {
			if len([]rune(w)) >= 4 {
				out[w] = true
			}
		}
		return out
	}
	wa, wb := set(a), set(b)
	if len(wa) == 0 || len(wb) == 0 {
		return 0
	}
	both := 0
	for w := range wa {
		if wb[w] {
			both++
		}
	}
	return float64(both) / float64(len(wa)+len(wb)-both)
}

// SimilarEntry: una decisión o un incidente vigente que se parece a otro.
type SimilarEntry struct {
	ID    string  `json:"id"`
	Title string  `json:"title"`
	Where string  `json:"where,omitempty"`
	Score float64 `json:"score"`
}
