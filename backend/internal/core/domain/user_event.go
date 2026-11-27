package domain

import (
	"encoding/json"
	"time"
)

// El diario de eventos de cada persona (command-and-control-558).
//
// Lo que cac le diría a alguien —un aviso de la campana, la actividad de
// Basecamp de su organización— queda además aquí, numerado **por persona y sin
// huecos**. Un token con `events:read` lo escucha por el stream y lo recupera
// desde un cursor, así que quien escucha desde una máquina (`cac-mcp listen`)
// no pierde lo que pasó mientras estaba desconectado.
//
// El número sale de una fila de cabecera bloqueada dentro de la transacción, y
// no de un `bigserial`: con una secuencia, dos inserciones concurrentes pueden
// confirmarse al revés de como tomaron su número, y quien ya leyó la 7 nunca
// verá la 6 que se confirmó después. Con el bloqueo, el orden del número es el
// orden de confirmación.

// Los tipos de evento del diario.
const (
	UserEventNotification = "notification:new"
	UserEventBasecamp     = "basecamp:event"
	// UserEventSkipped ocupa el número de un evento que quien escucha ya no
	// puede leer (es de una organización de la que salió): sin él, el cursor
	// vería un hueco y volvería a pedir lo mismo para siempre.
	UserEventSkipped = "cac:skipped"
	// UserEventGap: lo que había entre el cursor y lo que se guarda ya se
	// borró (retención). Ocupa el último número perdido, para que la cuenta
	// siga seguida, y pide un barrido a mano.
	UserEventGap = "cac:gap"
)

// Retención del diario.
const (
	UserEventRetention     = 30 * 24 * time.Hour
	UserEventKeepPerPerson = 5000
	// UserEventMaxData: lo que cabe en `data`. Un evento más grande se guarda
	// recortado (UserEventTruncated) en vez de no guardarse.
	UserEventMaxData = 32 * 1024
)

// UserEvent es una fila del diario.
type UserEvent struct {
	UserID string `gorm:"type:varchar(36);primaryKey;uniqueIndex:idx_user_events_key,priority:1"`
	Seq    int64  `gorm:"primaryKey;autoIncrement:false"`
	ID     string `gorm:"type:varchar(36);not null;uniqueIndex"`
	Type   string `gorm:"type:varchar(60);not null"`
	OrgID  string `gorm:"type:varchar(36);index"`
	// Key evita guardar dos veces lo mismo para la misma persona (un webhook
	// que Basecamp reintenta). Nil cuando no hay: Postgres no compara NULLs en
	// un índice único, así que los eventos sin clave no chocan entre sí.
	Key       *string         `gorm:"type:varchar(200);uniqueIndex:idx_user_events_key,priority:2"`
	Mine      bool            `gorm:"not null;default:false"`
	External  bool            `gorm:"not null;default:false"`
	Data      json.RawMessage `gorm:"type:jsonb"`
	CreatedAt time.Time       `gorm:"not null;index"`
}

// UserEventHead guarda el último número dado a cada persona. Es la fila que se
// bloquea al añadir.
type UserEventHead struct {
	UserID  string `gorm:"type:varchar(36);primaryKey"`
	LastSeq int64  `gorm:"not null;default:0"`
	// Epoch nombra esta vida del diario de la persona (ver JournalRestarted).
	// Nace con la fila: si la tabla se vacía, la fila nueva trae otra. Las
	// filas de antes de la columna la reciben en su siguiente Append.
	Epoch string `gorm:"type:varchar(36);not null;default:''"`
}

// GapReasonReset: la razón del cac:gap que dice que el diario volvió a empezar.
const GapReasonReset = "reset"

// JournalRestarted decide si quien lee desde `after` en la época `asked` lee
// otro diario que el que hay ahora (cabecera `head`, época `current`).
//
// Un diario vuelve a empezar cuando se vacía (`TRUNCATE`, una limpieza a
// mano) o se restaura de un respaldo. Hay dos señales, y hacen falta las dos:
//
//   - la cabecera por debajo del cursor: lo único que se ve tras restaurar un
//     respaldo (la época vuelve con la fila, la misma de entonces) o con un
//     cliente que no manda época;
//   - otra época: lo único que se ve si el diario nuevo ya pasó del cursor
//     antes de que quien escucha volviera a preguntar. Por la cabecera sola,
//     esos números se tomarían por los viejos y se saltarían.
//
// Una época vacía (cliente viejo, fila de antes de la columna, diario sin
// filas) no se compara: no dice nada.
//
// Tras restaurar un respaldo se lee desde el 1 lo que quien escucha ya tenía:
// no hay forma de saber qué números por debajo de la cabecera son del respaldo
// y cuáles se apuntaron después, así que se prefiere repetir a perder. Lo
// repetido vuelve con el mismo `id` (la fila restaurada lo conserva), y por él
// lo descarta quien lee (la skill `cac-listen`).
func JournalRestarted(after, head int64, asked, current string) bool {
	return after > head || (asked != "" && current != "" && asked != current)
}

// UserEventInput es lo que se le pide al diario que apunte.
type UserEventInput struct {
	UserID   string
	OrgID    string
	Type     string
	Key      string
	Mine     bool
	External bool
	Data     any
}

// EventEnvelope es el sobre de un evento: igual en el stream, en el backlog y
// en cada línea del fichero de `cac-mcp listen`. Es el contrato que comparten
// todas las piezas de la 558; no se cambia sin cambiarlas a todas.
type EventEnvelope struct {
	Seq      int64           `json:"seq"`
	ID       string          `json:"id"`
	Type     string          `json:"type"`
	At       time.Time       `json:"at"`
	OrgID    string          `json:"orgId"`
	Key      string          `json:"key"`
	Mine     bool            `json:"mine"`
	External bool            `json:"external"`
	Data     json.RawMessage `json:"data"`
}

// Envelope: el sobre de una fila.
func (e UserEvent) Envelope() EventEnvelope {
	key := ""
	if e.Key != nil {
		key = *e.Key
	}
	data := e.Data
	if len(data) == 0 {
		data = json.RawMessage("null")
	}
	return EventEnvelope{
		Seq: e.Seq, ID: e.ID, Type: e.Type, At: e.CreatedAt.UTC(), OrgID: e.OrgID,
		Key: key, Mine: e.Mine, External: e.External, Data: data,
	}
}

// Skipped: lo que recibe quien ya no puede leer un evento. Lleva el número y
// nada más: ni la organización, ni el tipo, ni el contenido.
func (e EventEnvelope) Skipped() EventEnvelope {
	return EventEnvelope{Seq: e.Seq, ID: e.ID, Type: UserEventSkipped, At: e.At, Data: json.RawMessage("null")}
}

// Visible: si quien tiene estas organizaciones puede leer el evento. Uno sin
// organización es suyo y nada más (un aviso personal).
func (e EventEnvelope) Visible(orgs map[string]bool) bool {
	return e.OrgID == "" || orgs[e.OrgID]
}

// GapEnvelope: el aviso de que se perdió lo que va de `from` a `to`, que ocupa
// el número `to`.
func GapEnvelope(from, to int64, at time.Time) EventEnvelope {
	data, _ := json.Marshal(map[string]int64{"from": from, "to": to})
	return EventEnvelope{Seq: to, ID: "gap", Type: UserEventGap, At: at.UTC(), Data: data}
}

// ResetEnvelope: el aviso de que el diario volvió a empezar. Lleva el `seq` 0,
// que no es de ningún evento: así no tapa el número de uno real, y quien lo
// recibe baja su cursor a 0 y lee el diario nuevo desde el 1. `previousCursor`
// es por dónde iba quien escuchaba; `epoch`, la época nueva.
func ResetEnvelope(previous int64, epoch string, at time.Time) EventEnvelope {
	data, _ := json.Marshal(map[string]any{"from": 0, "to": 0, "reason": GapReasonReset,
		"previousCursor": previous, "epoch": epoch})
	return EventEnvelope{Seq: 0, ID: "reset", Type: UserEventGap, At: at.UTC(), Data: data}
}

// IsReset: si el sobre es un ResetEnvelope.
func (e EventEnvelope) IsReset() bool {
	return e.Type == UserEventGap && e.Seq == 0 && e.ID == "reset"
}

// EventPage es una lectura del diario.
type EventPage struct {
	Events []EventEnvelope `json:"events"`
	// Head es el último número dado a esa persona.
	Head int64 `json:"head"`
	// Gap: lo que va desde el cursor pedido ya no está entero.
	Gap bool `json:"gap,omitempty"`
	// Epoch: la época del diario (ver JournalRestarted). Quien escucha la
	// guarda y la devuelve al preguntar (`?epoch=`).
	Epoch string `json:"epoch,omitempty"`
}
