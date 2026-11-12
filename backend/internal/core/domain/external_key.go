package domain

import "time"

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
	TaskID    string    `gorm:"type:varchar(36);primaryKey" json:"taskId"`
	Key       string    `gorm:"type:varchar(40);primaryKey;index:idx_task_external_keys_org,priority:2" json:"key"`
	OrgID     string    `gorm:"type:varchar(36);not null;index:idx_task_external_keys_org,priority:1" json:"-"`
	CreatedAt time.Time `json:"-"`
}

func (TaskExternalKey) TableName() string { return "task_external_keys" }

// IsExternalKey: el texto entero es una clave de fuera, tal cual se escribe.
func IsExternalKey(s string) bool {
	loc := externalKey.FindStringIndex(s)
	return loc != nil && loc[0] == 0 && loc[1] == len(s)
}
