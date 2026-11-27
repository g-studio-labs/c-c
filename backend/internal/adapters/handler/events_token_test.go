package handler

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// El stream de un token personal (command-and-control-558). Corre contra un
// diario en memoria con la misma promesa que service.EventJournal —número por
// persona, sin huecos—, para poder tirar eventos del hub a propósito sin una
// base de datos.

type memJournal struct {
	mu     sync.Mutex
	hub    *events.Hub
	rows   map[string][]domain.EventEnvelope
	onRead func() // se llama después de leer, con el diario ya leído
}

func newMemJournal(hub *events.Hub) *memJournal {
	return &memJournal{hub: hub, rows: map[string][]domain.EventEnvelope{}}
}

// add apunta un evento; con publish, también lo manda al hub.
func (j *memJournal) add(userID, orgID string, publish bool) domain.EventEnvelope {
	j.mu.Lock()
	seq := int64(len(j.rows[userID]) + 1)
	env := domain.EventEnvelope{Seq: seq, ID: "e" + strconv.FormatInt(seq, 10), Type: domain.UserEventNotification,
		At: time.Now().UTC(), OrgID: orgID, Data: json.RawMessage(`{"title":"para ` + userID + `"}`)}
	j.rows[userID] = append(j.rows[userID], env)
	j.mu.Unlock()
	if publish {
		j.hub.Publish(events.Event{Type: env.Type, OrgID: orgID, UserID: userID, Seq: seq, Data: env})
	}
	return env
}

func (j *memJournal) Head(userID string) (int64, error) {
	j.mu.Lock()
	defer j.mu.Unlock()
	return int64(len(j.rows[userID])), nil
}

func (j *memJournal) Read(userID string, after int64, limit int, orgs map[string]bool) (domain.EventPage, error) {
	j.mu.Lock()
	page := domain.EventPage{Head: int64(len(j.rows[userID]))}
	for _, e := range j.rows[userID] {
		if e.Seq > after && len(page.Events) < limit {
			if !e.Visible(orgs) {
				e = e.Skipped()
			}
			page.Events = append(page.Events, e)
		}
	}
	hook := j.onRead
	j.onRead = nil
	j.mu.Unlock()
	if hook != nil {
		hook()
	}
	return page, nil
}

const (
	goodToken   = "cac_pat_ana"
	noScopeTok  = "cac_pat_sin_scope"
	tokenOwner  = "u-ana"
	otherPerson = "u-bea"
)

// tokenAuth es el autenticador de tokens de las pruebas: ana con events:read
// y superadmin (para comprobar que eso no le abre nada), y un token suyo sin
// el scope. `revoked` lo apaga; `scopeDropped` le quita events:read.
var scopeDropped atomic.Bool

func tokenAuth(revoked *atomic.Bool) func(string) (*domain.ClaimsJWT, error) {
	return func(tok string) (*domain.ClaimsJWT, error) {
		if revoked != nil && revoked.Load() {
			return nil, errors.New("revoked")
		}
		c := &domain.ClaimsJWT{UserID: tokenOwner, Username: "ana", Superadmin: true,
			Orgs: []domain.OrgMembershipClaim{{OrgID: "org-1", Role: domain.OrgRoleMember}}}
		switch tok {
		case goodToken:
			c.Scopes = []string{domain.ScopeEventsRead}
			if scopeDropped.Load() {
				c.Scopes = []string{domain.ScopeTasksWrite}
			}
		case noScopeTok:
			c.Scopes = []string{domain.ScopeTasksWrite}
		default:
			return nil, errors.New("unknown")
		}
		return c, nil
	}
}

// sseReader lee los eventos de un stream en marcha.
type sseReader struct {
	events chan sseEvent
	done   chan struct{}
	status int
}

type sseEvent struct {
	id, name string
	env      domain.EventEnvelope
}

// openStream abre el stream contra un servidor de verdad (httptest) para leer
// los eventos según llegan.
func openStream(t *testing.T, h EventsHandler, tok, lastEventID string) (*sseReader, context.CancelFunc) {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(h.Stream))
	t.Cleanup(srv.Close)
	ctx, cancel := context.WithCancel(context.Background())
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, srv.URL+"/api/v1/events", nil)
	req.Header.Set("Authorization", "Bearer "+tok)
	if lastEventID != "" {
		req.Header.Set("Last-Event-ID", lastEventID)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	r := &sseReader{events: make(chan sseEvent, 100), done: make(chan struct{}), status: resp.StatusCode}
	go func() {
		defer close(r.done)
		defer resp.Body.Close()
		sc := bufio.NewScanner(resp.Body)
		var cur sseEvent
		var data string
		for sc.Scan() {
			line := sc.Text()
			switch {
			case strings.HasPrefix(line, "id: "):
				cur.id = strings.TrimPrefix(line, "id: ")
			case strings.HasPrefix(line, "event: "):
				cur.name = strings.TrimPrefix(line, "event: ")
			case strings.HasPrefix(line, "data: "):
				data = strings.TrimPrefix(line, "data: ")
			case line == "":
				if cur.name != "" && cur.name != "ping" {
					_ = json.Unmarshal([]byte(data), &cur.env)
					r.events <- cur
				}
				cur, data = sseEvent{}, ""
			}
		}
	}()
	t.Cleanup(cancel)
	return r, cancel
}

func (r *sseReader) next(t *testing.T) sseEvent {
	t.Helper()
	select {
	case e := <-r.events:
		return e
	case <-time.After(2 * time.Second):
		t.Fatal("no llegó el evento esperado")
		return sseEvent{}
	}
}

func (r *sseReader) quiet(t *testing.T, d time.Duration) {
	t.Helper()
	select {
	case e := <-r.events:
		t.Fatalf("llegó lo que no debía: %+v", e)
	case <-time.After(d):
	}
}

func withTokenAuth(t *testing.T, revoked *atomic.Bool) {
	UsePATAuthenticator(tokenAuth(revoked))
	t.Cleanup(func() { UsePATAuthenticator(nil) })
}

// Un token sólo recibe el diario de su dueño: ni lo de su organización, ni un
// comunicado, ni lo de otra persona, ni lo de todas aunque sea superadmin.
func TestATokenStreamOnlyCarriesItsOwnersEvents(t *testing.T) {
	withTokenAuth(t, nil)
	hub := events.NewHub()
	j := newMemJournal(hub)
	r, _ := openStream(t, NewEventsHandler(hub, nil, j), goodToken, "")
	if r.status != http.StatusOK {
		t.Fatalf("status %d", r.status)
	}
	time.Sleep(50 * time.Millisecond)

	hub.Publish(events.Event{Type: "task:moved", OrgID: "org-1", Data: map[string]string{"x": "org"}})
	hub.Publish(events.Event{Type: "post:new", Broadcast: true, Data: map[string]string{"x": "todos"}})
	hub.Publish(events.Event{Type: "report:new", OrgID: "org-9", Data: map[string]string{"x": "otra org"}})
	hub.Publish(events.Event{Type: "dm:message", UserID: tokenOwner, Data: map[string]string{"x": "sin número"}})
	j.add(otherPerson, "org-1", true)
	mine := j.add(tokenOwner, "org-1", true)

	got := r.next(t)
	if got.env.Seq != mine.Seq || got.id != "1" || got.name != domain.UserEventNotification ||
		!strings.Contains(string(got.env.Data), tokenOwner) {
		t.Fatalf("se esperaba sólo su evento 1, llegó %+v", got)
	}
	r.quiet(t, 150*time.Millisecond)
}

// Sin events:read no se escucha: ni el stream ni el backlog.
func TestEventsReadIsNeededToListen(t *testing.T) {
	withTokenAuth(t, nil)
	hub := events.NewHub()
	h := NewEventsHandler(hub, nil, newMemJournal(hub))

	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/api/v1/events", nil)
	req.Header.Set("Authorization", "Bearer "+noScopeTok)
	h.Stream(rec, req)
	if rec.Code != http.StatusForbidden || !strings.Contains(rec.Body.String(), "missing-scope:events:read") {
		t.Errorf("stream sin el scope: %d %s", rec.Code, rec.Body.String())
	}

	backlog := func(scopes []string) *httptest.ResponseRecorder {
		rec := httptest.NewRecorder()
		claims := &domain.ClaimsJWT{UserID: tokenOwner, ViaToken: true, Scopes: scopes,
			Orgs: []domain.OrgMembershipClaim{{OrgID: "org-1"}}}
		req := httptest.NewRequest(http.MethodGet, "/api/v1/events/backlog?after=0", nil)
		h.Backlog(rec, req.WithContext(context.WithValue(req.Context(), repository.UserContextKey, claims)))
		return rec
	}
	if rec := backlog([]string{domain.ScopeTasksWrite}); rec.Code != http.StatusForbidden ||
		!strings.Contains(rec.Body.String(), "missing-scope:events:read") {
		t.Errorf("backlog sin el scope: %d %s", rec.Code, rec.Body.String())
	}
	if rec := backlog([]string{domain.ScopeEventsRead}); rec.Code != http.StatusOK {
		t.Errorf("backlog con el scope: %d %s", rec.Code, rec.Body.String())
	}

	r, _ := openStream(t, h, goodToken, "")
	if r.status != http.StatusOK {
		t.Errorf("stream con el scope: %d", r.status)
	}
}

// Un token personal no va en la URL: acaba en los logs.
func TestAPersonalTokenInTheURLIsRefused(t *testing.T) {
	withTokenAuth(t, nil)
	hub := events.NewHub()
	h := NewEventsHandler(hub, nil, newMemJournal(hub))
	rec := httptest.NewRecorder()
	h.Stream(rec, httptest.NewRequest(http.MethodGet, "/api/v1/events?token="+goodToken, nil))
	if rec.Code != http.StatusUnauthorized {
		t.Errorf("un token en ?token= → %d, se esperaba 401", rec.Code)
	}
}

// Escuchar con un token no pone a nadie «presente».
func TestATokenStreamIsNotPresence(t *testing.T) {
	withTokenAuth(t, nil)
	original := latido
	latido = 5 * time.Millisecond
	defer func() { latido = original }()

	var seen atomic.Int32
	hub := events.NewHub()
	h := NewEventsHandler(hub, func(string) { seen.Add(1) }, newMemJournal(hub))
	r, cancel := openStream(t, h, goodToken, "")
	if r.status != http.StatusOK {
		t.Fatalf("status %d", r.status)
	}
	time.Sleep(60 * time.Millisecond)
	cancel()
	if n := seen.Load(); n != 0 {
		t.Errorf("el stream de un token registró presencia %d veces", n)
	}
}

// Un token revocado (o sin el scope) deja de oír en el siguiente latido.
func TestARevokedTokenClosesItsStream(t *testing.T) {
	var revoked atomic.Bool
	withTokenAuth(t, &revoked)
	original := latido
	latido = 10 * time.Millisecond
	defer func() { latido = original }()

	hub := events.NewHub()
	r, _ := openStream(t, NewEventsHandler(hub, nil, newMemJournal(hub)), goodToken, "")
	if r.status != http.StatusOK {
		t.Fatalf("status %d", r.status)
	}
	time.Sleep(40 * time.Millisecond)
	select {
	case <-r.done:
		t.Fatal("colgó con el token todavía bueno")
	default:
	}
	revoked.Store(true)
	select {
	case <-r.done:
	case <-time.After(2 * time.Second):
		t.Fatal("el token revocado sigue escuchando")
	}
}

// Quitarle events:read a un token también lo deja de oír.
func TestATokenThatLosesTheScopeClosesItsStream(t *testing.T) {
	withTokenAuth(t, nil)
	defer scopeDropped.Store(false)
	original := latido
	latido = 10 * time.Millisecond
	defer func() { latido = original }()

	hub := events.NewHub()
	r, _ := openStream(t, NewEventsHandler(hub, nil, newMemJournal(hub)), goodToken, "")
	if r.status != http.StatusOK {
		t.Fatalf("status %d", r.status)
	}
	time.Sleep(40 * time.Millisecond)
	scopeDropped.Store(true)
	select {
	case <-r.done:
	case <-time.After(2 * time.Second):
		t.Fatal("un token sin events:read sigue escuchando")
	}
}

// Lo que el hub tiró (un consumidor lento, un bus caído) se recupera del
// diario: por el número que salta, y en el latido si no llega nada más.
func TestTheStreamRepairsWhatTheHubDropped(t *testing.T) {
	withTokenAuth(t, nil)
	original := latido
	latido = 50 * time.Millisecond
	defer func() { latido = original }()

	hub := events.NewHub()
	j := newMemJournal(hub)
	r, _ := openStream(t, NewEventsHandler(hub, nil, j), goodToken, "")
	time.Sleep(20 * time.Millisecond)

	j.add(tokenOwner, "org-1", false)
	j.add(tokenOwner, "org-1", false)
	j.add(tokenOwner, "org-1", true)
	for want := int64(1); want <= 3; want++ {
		if got := r.next(t); got.env.Seq != want {
			t.Fatalf("se esperaba el %d, llegó el %d", want, got.env.Seq)
		}
	}

	// El último, tirado y sin nada detrás: lo trae el latido.
	j.add(tokenOwner, "org-1", false)
	if got := r.next(t); got.env.Seq != 4 {
		t.Fatalf("el latido no recuperó el 4: %+v", got)
	}
	r.quiet(t, 120*time.Millisecond)
}

// Al reconectar con Last-Event-ID llega lo que faltaba y después lo nuevo,
// también lo que se publicó mientras se leía el diario.
func TestReconnectingWithLastEventIDLosesNothing(t *testing.T) {
	withTokenAuth(t, nil)
	original := latido
	latido = time.Hour // que no lo arregle el latido
	defer func() { latido = original }()

	hub := events.NewHub()
	j := newMemJournal(hub)
	for i := 0; i < 3; i++ {
		j.add(tokenOwner, "org-1", false)
	}
	// Llega uno nuevo justo después de leer el diario.
	j.onRead = func() { j.add(tokenOwner, "org-1", true) }

	r, _ := openStream(t, NewEventsHandler(hub, nil, j), goodToken, "1")
	for want := int64(2); want <= 4; want++ {
		if got := r.next(t); got.env.Seq != want || got.id != strconv.FormatInt(want, 10) {
			t.Fatalf("se esperaba el %d, llegó %+v", want, got)
		}
	}
	j.add(tokenOwner, "org-1", true)
	if got := r.next(t); got.env.Seq != 5 {
		t.Fatalf("después de ponerse al día no siguió en vivo: %+v", got)
	}
	r.quiet(t, 100*time.Millisecond)
}

// Lo de una organización de la que ya no es miembro llega sólo como su número.
func TestAnEventOfAnOrgYouLeftOnlyKeepsItsNumber(t *testing.T) {
	withTokenAuth(t, nil)
	hub := events.NewHub()
	j := newMemJournal(hub)
	r, _ := openStream(t, NewEventsHandler(hub, nil, j), goodToken, "")
	time.Sleep(20 * time.Millisecond)

	j.add(tokenOwner, "org-2", true)
	got := r.next(t)
	if got.env.Seq != 1 || got.name != domain.UserEventSkipped || got.env.OrgID != "" ||
		strings.Contains(string(got.env.Data), tokenOwner) {
		t.Fatalf("lo de org-2 llegó con contenido: %+v", got)
	}
}
