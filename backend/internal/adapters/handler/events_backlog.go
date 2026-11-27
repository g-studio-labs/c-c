package handler

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	lg "github.com/guz-studio/cac/backend/internal/core/logger"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// El diario de eventos por el stream y por el backlog, para un token personal
// con `events:read` (command-and-control-558).
//
// Lo que se promete a quien escucha: cada evento de su dueño, en orden de
// `seq`, sin huecos y sin repetir dentro de una conexión, y al reconectar con
// `Last-Event-ID` lo que se perdió mientras tanto. «Exactamente una vez» lo
// cierra el cliente confirmando por `seq`; aquí es «al menos una vez, en
// orden».

// streamBatch: cuántos eventos se leen del diario de una vez al ponerse al día.
const streamBatch = 500

// streamToken es el stream de un token personal. Lo que lo separa del de una
// sesión, y por qué:
//
//   - se suscribe con hub.SubscribePersonal: sólo el diario de su dueño, ni lo
//     de sus organizaciones, ni los comunicados, ni todo aunque sea superadmin;
//   - se suscribe **antes** de leer el diario: al revés, lo que se publicara
//     entre la lectura y la suscripción no llegaría por ningún lado;
//   - un número que salta lee el hueco del diario, y en cada latido mira el
//     diario por si el hub tiró algo (un consumidor lento, un bus caído);
//   - en cada latido vuelve a autenticar el token: revocado, caducado o sin el
//     scope, cuelga;
//   - no llama a `seen`: un token no es una persona presente.
func (h *eventsHandler) streamToken(w http.ResponseWriter, r *http.Request, claims *domain.ClaimsJWT, tok string) {
	if h.journal == nil || patAuth == nil {
		SendErrorResponse(w, http.StatusServiceUnavailable, "Event journal unavailable", "no-journal")
		return
	}
	var cursor int64
	resume := r.Header.Get("Last-Event-ID")
	if resume != "" {
		n, err := strconv.ParseInt(resume, 10, 64)
		if err != nil || n < 0 {
			SendErrorResponse(w, http.StatusBadRequest, "Bad Last-Event-ID", "bad-last-event-id")
			return
		}
		cursor = n
	}
	// La época del diario que conoce quien reconecta (ver
	// domain.JournalRestarted). Sin ella, sólo la cabecera delata un reinicio.
	epoch := r.URL.Query().Get("epoch")
	flusher, ok := w.(http.Flusher)
	if !ok {
		SendErrorResponse(w, http.StatusInternalServerError, "Streaming unsupported", "no-flusher")
		return
	}

	ch, unsubscribe := h.hub.SubscribePersonal(claims.UserID)
	defer unsubscribe()

	// Sin cursor, se escucha desde ahora: la cabecera se lee ya suscrito, así
	// que lo que llegue a partir de aquí tiene un número mayor.
	if resume == "" {
		head, current, err := h.journal.Head(claims.UserID)
		if err != nil {
			SendErrorResponse(w, http.StatusInternalServerError, "Event journal unavailable", "journal-read")
			return
		}
		cursor, epoch = head, current
	}

	if err := http.NewResponseController(w).SetWriteDeadline(time.Time{}); err != nil {
		lg.Warn("SSE: could not clear write deadline (WriteTimeout will kill the stream): " + err.Error())
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)

	write := func(s string) bool {
		if _, err := fmt.Fprint(w, s); err != nil {
			return false
		}
		flusher.Flush()
		return true
	}
	frame := func(env domain.EventEnvelope) bool {
		payload, err := json.Marshal(env)
		if err != nil {
			return false
		}
		return write(fmt.Sprintf("id: %d\nevent: %s\ndata: %s\n\n", env.Seq, env.Type, payload))
	}
	orgs := service.OrgSet(claims)
	// send entrega un sobre si es el siguiente; lo que ya se entregó se salta.
	// El reset es la excepción: su `seq` 0 está por detrás de todo cursor a
	// propósito, se entrega siempre y baja el cursor a 0, para que el diario
	// nuevo llegue desde el 1 (ver EventJournal.Read).
	send := func(env domain.EventEnvelope) bool {
		if env.IsReset() {
			if !frame(env) {
				return false
			}
			cursor = 0
			return true
		}
		if env.Seq <= cursor {
			return true
		}
		if !frame(env) {
			return false
		}
		cursor = env.Seq
		return true
	}
	// catchUp lee del diario todo lo que hay después del cursor. También es
	// lo que ve un reinicio del diario con la conexión viva: Read lo compara
	// con la época y el cursor de esta conexión, y lo dice con el reset.
	catchUp := func() bool {
		for {
			page, err := h.journal.Read(claims.UserID, cursor, streamBatch, orgs, epoch)
			if err != nil {
				return true // el siguiente latido lo vuelve a intentar
			}
			// Read ya comparó la época que se le dio: la de la página es la de
			// ahora, y es la que vale para la siguiente pregunta, **también
			// vacía**. Tras un TRUNCATE no hay cabecera, y el reset sale sin
			// época hasta el siguiente Append. Quedarse con la vieja hacía que
			// el primer evento del diario nuevo pasara en vivo y el latido
			// siguiente, al ver la época nueva, anunciara el reinicio otra vez
			// y repitiera el diario desde el 1. Una época vacía no se compara
			// (domain.JournalRestarted): hasta que una página traiga la nueva,
			// sólo la cabecera puede delatar otro reinicio.
			epoch = page.Epoch
			if len(page.Events) == 0 {
				return true
			}
			for _, env := range page.Events {
				if !send(env) {
					return false
				}
			}
			if cursor >= page.Head {
				return true
			}
		}
	}

	if !write(": connected\n\n") || !catchUp() {
		return
	}

	heartbeat := time.NewTicker(latido)
	defer heartbeat.Stop()
	for {
		select {
		case <-r.Context().Done():
			return
		case <-heartbeat.C:
			fresh, err := patAuth(tok)
			if err != nil || !fresh.HasScope(domain.ScopeEventsRead) {
				write("event: token:revoked\ndata: {}\n\n")
				return
			}
			// Las membresías de hoy: lo de una organización que dejó no se
			// le entrega (sólo su número, ver EventEnvelope.Skipped).
			orgs = service.OrgSet(fresh)
			if !write(fmt.Sprintf("event: ping\ndata: {\"ts\":%d}\n\n", time.Now().Unix())) {
				return
			}
			if !catchUp() {
				return
			}
		case ev, open := <-ch:
			if !open {
				return
			}
			// Un número que no es el siguiente se mira en el diario: por
			// detrás del cursor suele ser uno repetido, pero también es lo
			// primero que llega de un diario que volvió a empezar, y callarlo
			// dejaría lo nuevo sin entregar hasta que pasara del cursor.
			if ev.Seq != cursor+1 {
				if !catchUp() {
					return
				}
				continue
			}
			env, ok := envelopeOf(ev.Data)
			if !ok || env.Seq != ev.Seq {
				if !catchUp() {
					return
				}
				continue
			}
			if !env.Visible(orgs) {
				env = env.Skipped()
			}
			if !send(env) {
				return
			}
		}
	}
}

// envelopeOf: el sobre que viaja en Data. En este pod es un
// domain.EventEnvelope; llegado por el bus, un mapa decodificado.
func envelopeOf(data any) (domain.EventEnvelope, bool) {
	if env, ok := data.(domain.EventEnvelope); ok {
		return env, true
	}
	raw, err := json.Marshal(data)
	if err != nil {
		return domain.EventEnvelope{}, false
	}
	var env domain.EventEnvelope
	if err := json.Unmarshal(raw, &env); err != nil || env.Seq <= 0 {
		return domain.EventEnvelope{}, false
	}
	return env, true
}

// Backlog: `GET /api/v1/events/backlog?after=<seq>&limit=`, lo que el diario
// guarda de quien pregunta después de `after`, tal como lo puede leer hoy
// (sólo de las organizaciones de las que es miembro). Contesta
// `{events, head, gap, epoch}`; con `?epoch=` (la que contestó la última
// vez), un diario que volvió a empezar se lee desde el 1 tras un reset.
//
// Va bajo AuthMiddleware, que deja pasar cualquier GET de un token: el scope
// se mira aquí, porque los GET no pasan por la tabla de rutas de un token.
func (h *eventsHandler) Backlog(w http.ResponseWriter, r *http.Request) {
	claims, ok := r.Context().Value(repository.UserContextKey).(*domain.ClaimsJWT)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "invalid-token")
		return
	}
	if claims.ViaToken && !claims.HasScope(domain.ScopeEventsRead) {
		SendErrorResponse(w, http.StatusForbidden, "Token is missing the "+domain.ScopeEventsRead+" scope",
			"missing-scope:"+domain.ScopeEventsRead)
		return
	}
	if h.journal == nil {
		SendErrorResponse(w, http.StatusServiceUnavailable, "Event journal unavailable", "no-journal")
		return
	}
	var after int64
	if q := r.URL.Query().Get("after"); q != "" {
		n, err := strconv.ParseInt(q, 10, 64)
		if err != nil || n < 0 {
			SendErrorResponse(w, http.StatusBadRequest, "Bad cursor", "bad-after")
			return
		}
		after = n
	}
	limit := 200
	if q := r.URL.Query().Get("limit"); q != "" {
		if n, err := strconv.Atoi(q); err == nil && n > 0 {
			limit = n
		}
	}
	page, err := h.journal.Read(claims.UserID, after, limit, service.OrgSet(claims), r.URL.Query().Get("epoch"))
	if err != nil {
		SendErrorResponse(w, http.StatusInternalServerError, "Event journal unavailable", "journal-read")
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[domain.EventPage]{Success: true, Data: page})
}
