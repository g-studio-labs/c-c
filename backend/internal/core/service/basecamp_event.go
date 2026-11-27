package service

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"sort"
	"strings"
	"time"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	lg "github.com/guz-studio/cac/backend/internal/core/logger"
)

// Basecamp «por contestar» y el evento completo (command-and-control-558, D).
//
// Cada evento de un proyecto enlazado pone al día su hilo
// (domain.BasecampThread) y deja un `basecamp:event` en el diario de cada
// miembro de la organización del proyecto, con el texto entero, el hilo y lo
// que le toca a esa persona. Va **antes** de los avisos, y cada aviso lleva su
// `key` como SourceKey: quien escucha ve primero el evento y puede casar el
// aviso con él.
//
// Nada de esto tumba un webhook: un error aquí se apunta en el log y el evento
// sigue. Basecamp desactiva el webhook entero tras diez fallos, y con él se
// perdería todo lo demás.

const (
	// basecampEnrichTimeout: lo que se espera a Basecamp para completar el
	// evento (los últimos comentarios, la lista del to-do). Sin respuesta, el
	// evento sale igual con `enriched: false`.
	basecampEnrichTimeout = 5 * time.Second
	// basecampRecent: los comentarios del hilo que viajan en el evento.
	basecampRecent = 5
	// basecampRecentPages: las páginas de comentarios que se leen para llegar
	// a los últimos.
	basecampRecentPages = 4
)

// WithJournal: otro diario que el del proceso, para las pruebas.
func (s *BasecampService) WithJournal(j *EventJournal) *BasecampService {
	s.journal = j
	return s
}

// WithEnrichTimeout: otro plazo para completar el evento, para las pruebas.
func (s *BasecampService) WithEnrichTimeout(d time.Duration) *BasecampService {
	s.enrichTimeout = d
	return s
}

func (s *BasecampService) eventJournal() *EventJournal {
	if s.journal != nil {
		return s.journal
	}
	return journalOut
}

// basecampEventKey: la clave del evento en el diario. La misma procedencia
// que la línea de la tarea (otra entrega del mismo comentario no es otro
// evento); un cambio de asignados lleva además el id del evento, porque cada
// uno es otra cosa que pasó en el mismo to-do.
func basecampEventKey(ev domain.BasecampWebhookEvent) string {
	if ev.Kind == domain.BasecampTodoAssignment || ev.Kind == domain.BasecampCardAssignment {
		return fmt.Sprintf("bc:%s:%d:e%d", ev.Kind, ev.Recording.ID, ev.ID)
	}
	return basecampLineKey(ev)
}

// basecampAuthor: quién escribió el evento y si es del equipo.
type basecampAuthor struct {
	wire   domain.BasecampPersonWire
	person *domain.BasecampPerson
	// team: casado con un miembro de la org y no marcado cliente. Todo lo
	// demás es de fuera, también una persona sin casar.
	team bool
}

func (a basecampAuthor) userID() string {
	if a.person == nil || !a.team {
		return ""
	}
	return a.person.UserID
}

func (s *BasecampService) authorOf(orgID string, wire domain.BasecampPersonWire, person *domain.BasecampPerson) basecampAuthor {
	a := basecampAuthor{wire: wire, person: person}
	a.team = person != nil && person.UserID != "" && !person.Client && !wire.Client && s.repo.IsMember(orgID, person.UserID)
	return a
}

// trackThread pone al día el hilo del evento. Sin hilo (un error), el evento
// sale igual, sin el estado del hilo.
func (s *BasecampService) trackThread(proj *domain.BasecampProject, ev domain.BasecampWebhookEvent, author basecampAuthor) *domain.BasecampThread {
	targetID, title, appURL := ev.Target()
	if targetID == 0 {
		return nil
	}
	typ := ev.Recording.Type
	if ev.Kind == domain.BasecampCommentCreated && ev.Recording.Parent != nil {
		typ = ev.Recording.Parent.Type
	}
	at := ev.CreatedAt
	if at.IsZero() {
		at = s.now()
	}
	said := (ev.Kind == domain.BasecampCommentCreated || ev.Kind == domain.BasecampMessageCreated) && author.wire.ID != 0
	w := domain.BasecampThreadWrite{OrgID: proj.OrgID, RecordingID: targetID, ProjectID: proj.ProjectID,
		Title: clipRunes(strings.TrimSpace(title), 300), AppURL: domain.SafeBasecampURL(appURL), Type: typ,
		Said: said, At: at.UTC(), External: !author.team, AuthorName: strings.TrimSpace(author.wire.Name),
		AuthorClient: author.wire.Client || (author.person != nil && author.person.Client),
		AuthorUserID: author.userID(),
		Excerpt:      domain.BasecampPlainText(ev.Recording.Content, basecampExcerpt)}
	thread, err := s.repo.ApplyToThread(w)
	if err != nil {
		lg.Warn(fmt.Sprintf("basecamp: updating thread %d of org %s: %v", targetID, proj.OrgID, err))
		return nil
	}
	return thread
}

func clipRunes(s string, max int) string {
	if r := []rune(s); len(r) > max {
		return string(r[:max])
	}
	return s
}

// basecampFull: lo que hace falta para el evento completo.
type basecampFull struct {
	proj   *domain.BasecampProject
	ev     domain.BasecampWebhookEvent
	key    string
	author basecampAuthor
	thread *domain.BasecampThread
	item   *domain.Item
	// todo: el to-do o la tarjeta, si ya se leyó (los eventos que hacen
	// tarea). Si no, se pide al completar.
	todo *domain.BasecampAssignable
	// textHTML: el texto del evento; vacío, el del recording.
	textHTML string
}

type bcPersonOut struct {
	PersonID int64  `json:"personId"`
	Name     string `json:"name"`
	Client   bool   `json:"client"`
	UserID   string `json:"userId"`
}

// publishEvent deja el `basecamp:event` en el diario de cada miembro de la
// org. No falla nunca: lo que no puede completar sale sin completar.
func (s *BasecampService) publishEvent(f basecampFull) {
	j := s.eventJournal()
	if j == nil {
		return
	}
	members, err := s.repo.MemberIDs(f.proj.OrgID)
	if err != nil {
		lg.Warn("basecamp: listing the members for an event: " + err.Error())
		return
	}
	if len(members) == 0 {
		return
	}
	targetID, _, _ := f.ev.Target()
	timeout := s.enrichTimeout
	if timeout <= 0 {
		timeout = basecampEnrichTimeout
	}
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	enriched := true
	todo := f.todo
	if todo == nil {
		if path := todoPath(f.ev, f.proj.ProjectID, targetID); path != "" {
			var rec domain.BasecampAssignable
			if err := s.apiCtx(ctx, f.proj.OrgID, path, &rec); err == nil && rec.ID == targetID {
				todo = &rec
			} else if err != nil && !errors.Is(err, gorm.ErrRecordNotFound) {
				enriched = false
			}
		}
	}
	recent, ok := s.recentComments(ctx, f.proj, targetID)
	enriched = enriched && ok

	data := s.eventData(f, todo, recent, enriched)
	mentioned := map[string]bool{}
	for _, m := range data["mentions"].([]bcPersonOut) {
		if m.UserID != "" {
			mentioned[m.UserID] = true
		}
	}
	assigned := map[string]bool{}
	if t, ok := data["todo"].(map[string]any); ok {
		for _, a := range t["assignees"].([]bcPersonOut) {
			if a.UserID != "" {
				assigned[a.UserID] = true
			}
		}
	}
	for _, uid := range members {
		you := map[string]any{"mentioned": mentioned[uid], "assigned": assigned[uid],
			"answeredAfter": f.thread.AnsweredBy(uid)}
		per := make(map[string]any, len(data)+1)
		for k, v := range data {
			per[k] = v
		}
		per["you"] = you
		if _, _, err := j.Append(domain.UserEventInput{UserID: uid, OrgID: f.proj.OrgID, Type: domain.UserEventBasecamp,
			Key: f.key, Mine: mentioned[uid] || assigned[uid], External: !f.author.team, Data: per}); err != nil {
			lg.Warn("basecamp: appending an event to a journal: " + err.Error())
		}
	}
}

// todoPath: dónde se lee el to-do o la tarjeta del hilo, si el hilo es uno.
func todoPath(ev domain.BasecampWebhookEvent, projectID, targetID int64) string {
	typ := ev.Recording.Type
	if ev.Kind == domain.BasecampCommentCreated && ev.Recording.Parent != nil {
		typ = ev.Recording.Parent.Type
	}
	switch typ {
	case "Todo":
		return fmt.Sprintf("/buckets/%d/todos/%d.json", projectID, targetID)
	case "Kanban::Card":
		return fmt.Sprintf("/buckets/%d/card_tables/cards/%d.json", projectID, targetID)
	}
	return ""
}

// apiCtx: una lectura de la cuenta de la org con el plazo de ctx.
func (s *BasecampService) apiCtx(ctx context.Context, orgID, path string, out any) error {
	conn, token, err := s.accessToken(orgID)
	if err != nil {
		return err
	}
	_, err = s.doCtx(ctx, http.MethodGet, s.accountURL(conn, path), token, nil, out)
	return err
}

type basecampCommentWire struct {
	ID        int64                      `json:"id"`
	CreatedAt time.Time                  `json:"created_at"`
	Content   string                     `json:"content"`
	AppURL    string                     `json:"app_url"`
	Creator   *domain.BasecampPersonWire `json:"creator"`
}

// recentComments: los últimos comentarios del hilo, del más viejo al más
// nuevo. `false` si Basecamp no contestó.
func (s *BasecampService) recentComments(ctx context.Context, proj *domain.BasecampProject, targetID int64) ([]map[string]any, bool) {
	conn, token, err := s.accessToken(proj.OrgID)
	if err != nil {
		return []map[string]any{}, false
	}
	var all []basecampCommentWire
	next := s.accountURL(conn, fmt.Sprintf("/buckets/%d/recordings/%d/comments.json", proj.ProjectID, targetID))
	for page := 0; next != "" && page < basecampRecentPages; page++ {
		var batch []basecampCommentWire
		h, err := s.doCtx(ctx, http.MethodGet, next, token, nil, &batch)
		if errors.Is(err, gorm.ErrRecordNotFound) {
			break
		}
		if err != nil {
			return []map[string]any{}, false
		}
		all = append(all, batch...)
		next = ""
		if m := reNextLink.FindStringSubmatch(h.Get("Link")); m != nil && strings.HasPrefix(m[1], s.cfg.APIURL+"/") {
			next = m[1]
		}
	}
	sort.SliceStable(all, func(i, k int) bool { return all[i].CreatedAt.Before(all[k].CreatedAt) })
	if len(all) > basecampRecent {
		all = all[len(all)-basecampRecent:]
	}
	out := make([]map[string]any, 0, len(all))
	for _, c := range all {
		author := bcPersonOut{}
		if c.Creator != nil {
			author = s.personOut(proj.OrgID, *c.Creator)
		}
		out = append(out, map[string]any{"id": c.ID, "at": c.CreatedAt.UTC(), "author": author,
			"text": domain.BasecampPlainText(c.Content, basecampExcerpt), "appUrl": domain.SafeBasecampURL(c.AppURL)})
	}
	return out, true
}

// personOut: una persona de Basecamp con su usuario de cac, si casó con un
// miembro de la org.
func (s *BasecampService) personOut(orgID string, w domain.BasecampPersonWire) bcPersonOut {
	out := bcPersonOut{PersonID: w.ID, Name: strings.TrimSpace(w.Name), Client: w.Client}
	if w.ID == 0 {
		return out
	}
	p, err := s.repo.UpsertPerson(orgID, w)
	if err != nil {
		return out
	}
	out.Client = out.Client || p.Client
	if p.UserID != "" && s.repo.IsMember(orgID, p.UserID) {
		out.UserID = p.UserID
	}
	return out
}

// eventData: el `data` del evento, igual para todos los miembros salvo `you`.
func (s *BasecampService) eventData(f basecampFull, todo *domain.BasecampAssignable, recent []map[string]any, enriched bool) map[string]any {
	ev, proj := f.ev, f.proj
	targetID, targetTitle, targetURL := ev.Target()

	var parent any
	if p := ev.Recording.Parent; p != nil {
		parent = map[string]any{"id": p.ID, "title": p.Title, "type": p.Type, "appUrl": domain.SafeBasecampURL(p.AppURL)}
	}
	data := map[string]any{
		"kind":      ev.Kind,
		"eventId":   ev.ID,
		"createdAt": ev.CreatedAt.UTC(),
		"project":   map[string]any{"id": proj.ProjectID, "name": proj.Name, "spaceId": proj.SpaceID},
		"recording": map[string]any{"id": ev.Recording.ID, "type": ev.Recording.Type, "title": ev.Recording.Title,
			"appUrl": domain.SafeBasecampURL(ev.Recording.AppURL), "parent": parent},
		"enriched": enriched,
	}

	author := bcPersonOut{PersonID: f.author.wire.ID, Name: strings.TrimSpace(f.author.wire.Name),
		Client: f.author.wire.Client || (f.author.person != nil && f.author.person.Client), UserID: f.author.userID()}
	data["author"] = author
	data["team"] = f.author.team

	content := f.textHTML
	if content == "" {
		content = ev.Recording.Content
	}
	putText(data, "text", domain.BasecampMarkdown(content))
	mentions := []bcPersonOut{}
	seen := map[string]bool{}
	for _, m := range domain.ExtractBasecampMentions(content) {
		key := m.SGID
		if seen[key] {
			continue
		}
		seen[key] = true
		p, err := s.repo.PersonForMention(proj.OrgID, m)
		if err != nil {
			mentions = append(mentions, bcPersonOut{PersonID: m.PersonID})
			continue
		}
		out := bcPersonOut{PersonID: p.PersonID, Name: p.Name, Client: p.Client}
		if p.UserID != "" && s.repo.IsMember(proj.OrgID, p.UserID) {
			out.UserID = p.UserID
		}
		mentions = append(mentions, out)
	}
	data["mentions"] = mentions
	data["attachments"] = domain.BasecampAttachments(content)

	thread := map[string]any{"recordingId": targetID, "title": targetTitle,
		"appUrl": domain.SafeBasecampURL(targetURL), "recent": recent}
	if t := f.thread; t != nil {
		thread["type"] = t.Type
		thread["awaitingReply"] = t.AwaitingReply
		thread["awaitingSince"] = t.AwaitingSince
		if t.LastExternalAt != nil {
			thread["lastExternal"] = map[string]any{"name": t.LastExternalName, "client": t.LastExternalClient,
				"at": t.LastExternalAt}
		}
		if t.LastTeamReplyAt != nil {
			thread["lastTeamReply"] = map[string]any{"userId": t.LastTeamReplyByID, "at": t.LastTeamReplyAt}
		}
	}
	data["thread"] = thread

	if todo != nil {
		assignees := []bcPersonOut{}
		for _, a := range todo.Assignees {
			assignees = append(assignees, s.personOut(proj.OrgID, a))
		}
		data["todo"] = map[string]any{"assignees": assignees, "dueOn": todo.DueOn, "completed": todo.Completed}
		if p := todo.Parent; p != nil {
			data["list"] = map[string]any{"id": p.ID, "title": p.Title, "type": p.Type, "appUrl": domain.SafeBasecampURL(p.AppURL)}
		}
	}

	cac := map[string]any{"taskId": "", "taskRef": "", "requestCodes": []string{}}
	if f.item != nil {
		cac["taskId"] = f.item.ID
		cac["taskRef"] = s.repo.TaskRefOf(f.item)
		cac["requestCodes"] = s.repo.OpenRequestCodes(f.item.ID)
	}
	data["cac"] = cac
	return data
}

// Awaiting: los hilos por contestar de la org.
func (s *BasecampService) Awaiting(orgID string) ([]domain.BasecampAwaiting, error) {
	return s.repo.Awaiting(orgID)
}
