package domain

import (
	"errors"
	"regexp"
	"strings"
	"time"
)

// Las fuentes de un espacio: dónde vive fuera lo que cac tiene que traer, y
// dónde vive dentro lo que sus textos nombran (command-and-control-354,
// decisión 0caaffbe).
//
// cac es la fuente y lo de fuera es copia, pero la copia se mueve sola: el
// equipo de un cliente comenta en Jira y decide en Notion. Aquí se apunta, por
// espacio, qué tablero de Jira es el suyo, qué páginas de Notion hay que leer y
// hasta cuándo se leyó ya cada una (`LastSyncAt`): la puesta al día pregunta
// «qué hay nuevo desde entonces» y sólo mueve el cursor si terminó.
//
// Y una fuente es de dentro: el registro de reglas. Es una página de cac con
// una sección por regla (`## R-012 · …`); con su patrón apuntado aquí, `R-012`
// en cualquier texto del espacio pasa a ser una arista a esa sección, igual
// que `proteus-12` lo es a su tarea. El patrón es del espacio y no del código:
// otro cliente numera `BR-014`.

// SourceRole: para qué sirve una fuente.
type SourceRole string

const (
	// SourceTracker: el tablero de fuera donde se mueven los tickets.
	SourceTracker SourceRole = "tracker"
	// SourceRules: el registro de reglas, una página de cac.
	SourceRules SourceRole = "rules"
	// SourceRulesCopy: la copia del registro que lee quien no usa cac (una
	// página de Notion). Se regenera desde cac; no es de donde se lee.
	SourceRulesCopy SourceRole = "rules-copy"
	// SourceDecisions: donde el cliente apunta lo que se resolvió.
	SourceDecisions SourceRole = "decisions"
	// SourceOpenQuestions: lo que está por definir.
	SourceOpenQuestions SourceRole = "open-questions"
)

var SourceRoles = []SourceRole{SourceTracker, SourceRules, SourceRulesCopy, SourceDecisions, SourceOpenQuestions}

// SourceSystem: dónde vive.
type SourceSystem string

const (
	SystemJira   SourceSystem = "jira"
	SystemNotion SourceSystem = "notion"
	SystemCac    SourceSystem = "cac"
	SystemOther  SourceSystem = "other"
)

var SourceSystems = []SourceSystem{SystemJira, SystemNotion, SystemCac, SystemOther}

// SpaceSourcesMax: cuántas fuentes caben en un espacio. Son un puñado; más es
// un error de quien llama.
const SpaceSourcesMax = 20

// RulePatternMax: lo más largo que puede ser el patrón de una regla.
const RulePatternMax = 40

// SpaceSource: una fuente de un espacio.
type SpaceSource struct {
	ID      string       `gorm:"type:varchar(36);primaryKey" json:"id"`
	OrgID   string       `gorm:"type:varchar(36);not null;index" json:"-"`
	SpaceID string       `gorm:"type:varchar(36);not null;index" json:"spaceId"`
	Role    SourceRole   `gorm:"type:varchar(20);not null" json:"role"`
	System  SourceSystem `gorm:"type:varchar(20);not null" json:"system"`
	// Ref: cómo se nombra allí. La clave del proyecto de Jira (`PROTEUS`), la
	// URL de la página de Notion, o el id de la página de cac.
	Ref string `gorm:"type:text;not null" json:"ref"`
	// Pattern: sólo en `rules`: cómo se escribe una regla (`R-\d+`).
	Pattern string `gorm:"type:varchar(60)" json:"pattern,omitempty"`
	Note    string `gorm:"type:text" json:"note,omitempty"`
	// LastSyncAt: hasta cuándo se leyó ya. Nil: nunca.
	LastSyncAt *time.Time `json:"lastSyncAt,omitempty"`
	CreatedAt  time.Time  `json:"createdAt"`
	UpdatedAt  time.Time  `json:"updatedAt"`
}

func (SpaceSource) TableName() string { return "space_sources" }

// SpaceSourceInput: una fuente como se manda al guardar. Con `ID`, es la que
// ya estaba; sin él, se reconoce por papel, sistema y ref, para que volver a
// mandar la lista entera no le borre el cursor a ninguna.
type SpaceSourceInput struct {
	ID      string       `json:"id"`
	Role    SourceRole   `json:"role"`
	System  SourceSystem `json:"system"`
	Ref     string       `json:"ref"`
	Pattern string       `json:"pattern"`
	Note    string       `json:"note"`
}

// SpaceSourcesRequest: la lista entera de un espacio; lo que no venga se quita.
type SpaceSourcesRequest struct {
	Sources []SpaceSourceInput `json:"sources"`
}

// SourceSyncedRequest: hasta cuándo se leyó una fuente.
type SourceSyncedRequest struct {
	At time.Time `json:"at"`
}

var (
	ErrSourceRole       = errors.New("unknown-source-role")
	ErrSourceSystem     = errors.New("unknown-source-system")
	ErrSourceRef        = errors.New("source-ref-required")
	ErrSourcePattern    = errors.New("invalid-rule-pattern")
	ErrSourceRulesInCac = errors.New("rules-live-in-cac")
	ErrSourceTooMany    = errors.New("too-many-sources")
	ErrSourceRepeated   = errors.New("source-repeated")
	ErrSourceNotFound   = errors.New("source-not-found")
	ErrSourcePage       = errors.New("rules-page-not-found")
	ErrSourceSyncFuture = errors.New("sync-in-the-future")
)

// Normalize deja la fuente como se guarda, o dice por qué no vale.
func (in SpaceSourceInput) Normalize() (SpaceSourceInput, error) {
	in.Ref, in.Pattern, in.Note = strings.TrimSpace(in.Ref), strings.TrimSpace(in.Pattern), strings.TrimSpace(in.Note)
	if !validSourceRole(in.Role) {
		return in, ErrSourceRole
	}
	if !validSourceSystem(in.System) {
		return in, ErrSourceSystem
	}
	if in.Ref == "" {
		return in, ErrSourceRef
	}
	if in.Role != SourceRules {
		// Un patrón sin registro no nombra nada: se tira en vez de guardarlo.
		in.Pattern = ""
		return in, nil
	}
	// El registro vive en cac: es lo que lo hace una sección con identidad,
	// con sus decisiones y sus vecinos. Uno de fuera sería sólo un enlace.
	if in.System != SystemCac {
		return in, ErrSourceRulesInCac
	}
	if _, err := CompileRulePattern(in.Pattern); err != nil {
		return in, err
	}
	return in, nil
}

func validSourceRole(r SourceRole) bool {
	for _, x := range SourceRoles {
		if x == r {
			return true
		}
	}
	return false
}

func validSourceSystem(s SourceSystem) bool {
	for _, x := range SourceSystems {
		if x == s {
			return true
		}
	}
	return false
}

// CompileRulePattern: el patrón de un espacio, listo para buscar reglas
// enteras en un texto. Entre límites de palabra: `R-12` no sale de `PR-123`.
// Un patrón que casa con la nada (`\d*`) nombraría todo, y no se acepta.
func CompileRulePattern(pattern string) (*regexp.Regexp, error) {
	if len(pattern) > RulePatternMax {
		return nil, ErrSourcePattern
	}
	// El vacío cae aquí también: casa con la nada.
	whole, err := regexp.Compile(`^(?:` + pattern + `)$`)
	if err != nil || whole.MatchString("") {
		return nil, ErrSourcePattern
	}
	return regexp.Compile(`\b(?:` + pattern + `)\b`)
}

// ExtractRules: las reglas que un texto nombra, cada una una vez y en el
// orden en que salen.
func ExtractRules(re *regexp.Regexp, text string) []string {
	var out []string
	seen := map[string]bool{}
	for _, m := range re.FindAllString(text, -1) {
		if !seen[m] {
			seen[m] = true
			out = append(out, m)
		}
	}
	return out
}

// RuleOfHeading: la regla que **es** una sección: la que abre su título
// (`R-012 · Los pagos…`). Una sección que sólo la cita («Notas sobre R-012»)
// no es la regla, y no se queda con sus enlaces.
func RuleOfHeading(re *regexp.Regexp, heading string) string {
	heading = strings.TrimSpace(heading)
	loc := re.FindStringIndex(heading)
	if loc == nil || loc[0] != 0 {
		return ""
	}
	return heading[loc[0]:loc[1]]
}
