package service

import (
	"encoding/json"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	lg "github.com/guz-studio/cac/backend/internal/core/logger"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// EventJournal es el diario de eventos de cada persona (command-and-control-558):
// apunta, publica en el hub con su número y lee desde un cursor. Ver
// domain/user_event.go.
type EventJournal struct {
	repo *repository.UserEventRepository
	hub  *events.Hub
	now  func() time.Time
}

func NewEventJournal(repo *repository.UserEventRepository, hub *events.Hub) *EventJournal {
	return &EventJournal{repo: repo, hub: hub, now: time.Now}
}

// journalOut es el diario del proceso, como pushOut: lo usa Notify, que se
// construye en una docena de sitios y no tiene por qué saber de él. Sin
// diario (las pruebas que no lo ponen) no se apunta nada.
var journalOut *EventJournal

// SetEventJournal: ver journalOut.
func SetEventJournal(j *EventJournal) { journalOut = j }

// Append apunta un evento en el diario de una persona y, una vez confirmado,
// lo publica. Devuelve el sobre y si se apuntó (`false`: la clave ya estaba).
//
// Publicar va **después** de confirmar: quien recibe el número N por el hub
// tiene que poder leer N del diario, o no podría rellenar un hueco.
func (j *EventJournal) Append(in domain.UserEventInput) (domain.EventEnvelope, bool, error) {
	data, err := json.Marshal(in.Data)
	if err != nil {
		return domain.EventEnvelope{}, false, err
	}
	if len(data) > domain.UserEventMaxData {
		data, _ = json.Marshal(map[string]any{"truncated": true, "bytes": len(data)})
	}
	row, created, err := j.repo.Append(in, data, j.now)
	if err != nil || !created {
		return domain.EventEnvelope{}, false, err
	}
	env := row.Envelope()
	if j.hub != nil {
		j.hub.Publish(events.Event{Type: env.Type, OrgID: env.OrgID, UserID: in.UserID, Seq: env.Seq, Data: env})
	}
	return env, true, nil
}

// Head: el último número de esa persona.
func (j *EventJournal) Head(userID string) (int64, error) { return j.repo.Head(userID) }

// Read: hasta `limit` eventos de esa persona después de `after`, tal como los
// puede leer hoy quien es miembro de `orgs`.
//
// Lo de una organización de la que ya no es miembro no se le enseña, pero su
// número sí (EventEnvelope.Skipped): el cursor de quien escucha sigue seguido.
// Y si lo que va desde `after` ya se borró, el primer sobre es un
// domain.GapEnvelope que ocupa el último número perdido, por lo mismo.
func (j *EventJournal) Read(userID string, after int64, limit int, orgs map[string]bool) (domain.EventPage, error) {
	if limit <= 0 || limit > 500 {
		limit = 500
	}
	head, err := j.repo.Head(userID)
	if err != nil {
		return domain.EventPage{}, err
	}
	page := domain.EventPage{Events: []domain.EventEnvelope{}, Head: head}
	if after > head {
		// Un cursor por delante de la cabecera (un diario reiniciado): no se
		// acepta en silencio, es un hueco y se sigue desde la cabecera.
		page.Gap = true
		page.Events = append(page.Events, domain.GapEnvelope(head, head, j.now()))
		return page, nil
	}
	if after == head {
		return page, nil
	}
	rows, oldest, err := j.repo.After(userID, after, limit)
	if err != nil {
		return domain.EventPage{}, err
	}
	// Lo primero que se guarda está más allá del siguiente: se perdió algo.
	// Sin filas pero con cabecera por delante, se perdió todo lo que faltaba.
	firstKept := head + 1
	if oldest > 0 {
		firstKept = oldest
	}
	if after+1 < firstKept {
		page.Gap = true
		page.Events = append(page.Events, domain.GapEnvelope(after+1, firstKept-1, j.now()))
	}
	for _, row := range rows {
		env := row.Envelope()
		if !env.Visible(orgs) {
			env = env.Skipped()
		}
		page.Events = append(page.Events, env)
	}
	return page, nil
}

// Prune: la retención del diario, 30 días y 5 000 eventos por persona.
func (j *EventJournal) Prune(now time.Time) (int64, error) {
	return j.repo.Prune(now.Add(-domain.UserEventRetention), domain.UserEventKeepPerPerson)
}

// StartPruning: el reloj diario de la retención.
func (j *EventJournal) StartPruning() {
	go func() {
		tick := time.NewTicker(24 * time.Hour)
		defer tick.Stop()
		for {
			if _, err := j.Prune(time.Now()); err != nil {
				lg.Error("event journal prune failed: " + err.Error())
			}
			<-tick.C
		}
	}()
}

// OrgSet: las organizaciones de unos claims, para Read. Sólo las membresías:
// ser superadmin no abre el diario de nadie a otras organizaciones.
func OrgSet(c *domain.ClaimsJWT) map[string]bool {
	set := make(map[string]bool, len(c.Orgs))
	for _, o := range c.Orgs {
		set[o.OrgID] = true
	}
	return set
}
