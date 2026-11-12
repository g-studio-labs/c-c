package domain

import (
	"regexp"
	"time"
)

// TaskExternalKey: una clave de fuera (`PROTEUS-165`, de Jira) que una tarea
// nombra en su título o su descripción (command-and-control-353).
//
// Antes sólo se calculaba al vuelo para las tareas en curso (`ContextTask.Keys`),
// así que «¿qué tarea es la de PROTEUS-165?» no tenía respuesta fuera del mapa:
// ni `get_task`, ni el tablero. Guardada, se pregunta por índice.
//
// La clave de fuera y la ref de cac numeran aparte: `PROTEUS-165` no es
// `proteus-165`. Por eso aquí sólo entra lo que va en mayúsculas
// (`ExternalKeys`), y la ref sigue su camino (`FindByRef`).
type TaskExternalKey struct {
	TaskID string `gorm:"type:varchar(36);primaryKey" json:"taskId"`
	Key    string `gorm:"type:varchar(40);primaryKey;index:idx_task_external_keys_org,priority:2" json:"key"`
	OrgID  string `gorm:"type:varchar(36);not null;index:idx_task_external_keys_org,priority:1" json:"-"`
	// Kind: cómo la nombra (ver ExternalKeyKind). Nombrar un ticket no es ser
	// su tarea.
	Kind      ExternalKeyKind `gorm:"type:varchar(10);not null;default:''" json:"kind"`
	CreatedAt time.Time       `json:"-"`
}

func (TaskExternalKey) TableName() string { return "task_external_keys" }

// ExternalKeyKind: cómo nombra una tarea una clave de fuera.
//
// El primer día en producción (12-nov-2026) `PROTEUS-165` lo nombraban cuatro
// tareas: la que es ese ticket y tres que lo citaban de pasada, dos de ellas
// las de este mismo trabajo, que lo ponían de ejemplo. Preguntar «¿qué tarea
// es PROTEUS-165?» contestaba «varias». Lo que distingue a la tarea del ticket
// es dónde lo dice: en su línea **Jira:** (la convención de `cac-work`), y si
// no hay ninguna así, en el título.
type ExternalKeyKind string

const (
	ExternalKeyTicket  ExternalKeyKind = "ticket"  // en la línea **Jira:** de la descripción
	ExternalKeyTitle   ExternalKeyKind = "title"   // en el título
	ExternalKeyMention ExternalKeyKind = "mention" // en cualquier otro sitio del texto
)

// ExternalKeyKinds: de la que más dice «soy ese ticket» a la que menos.
var ExternalKeyKinds = []ExternalKeyKind{ExternalKeyTicket, ExternalKeyTitle, ExternalKeyMention}

// trackerLine: lo que sigue a **Jira:** hasta la siguiente negrita o el fin de
// la línea. `**Jira:** PROTEUS-165 · **Referencias:** PROTEUS-12` da sólo el
// primero: lo de **Referencias** es una cita.
var trackerLine = regexp.MustCompile(`(?i)\*\*\s*jira\s*:?\s*\*\*\s*:?([^*\n]*)`)

// ClassifyExternalKeys: las claves de fuera de una tarea (las de ExternalKeys,
// en su orden y con su tope) y cómo nombra cada una.
func ClassifyExternalKeys(title, description string) []TaskExternalKey {
	ticket := map[string]bool{}
	for _, m := range trackerLine.FindAllStringSubmatch(description, -1) {
		for _, k := range externalKey.FindAllString(m[1], -1) {
			ticket[k] = true
		}
	}
	inTitle := map[string]bool{}
	for _, k := range externalKey.FindAllString(title, -1) {
		inTitle[k] = true
	}
	var out []TaskExternalKey
	for _, k := range ExternalKeys(title, description) {
		kind := ExternalKeyMention
		if ticket[k] {
			kind = ExternalKeyTicket
		} else if inTitle[k] {
			kind = ExternalKeyTitle
		}
		out = append(out, TaskExternalKey{Key: k, Kind: kind})
	}
	return out
}

// IsExternalKey: el texto entero es una clave de fuera, tal cual se escribe.
func IsExternalKey(s string) bool {
	loc := externalKey.FindStringIndex(s)
	return loc != nil && loc[0] == 0 && loc[1] == len(s)
}
