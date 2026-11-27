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
	// basecampMaxAttachments y basecampMaxMentions: los que viajan en el
	// evento. Sin tope, un comentario con ~120 imágenes pasa de
	// UserEventMaxData y el diario cambia todo `data` por {truncated, bytes},
	// ids incluidos. Lo que sobra se marca con `attachmentsTruncated` /
	// `mentionsTruncated`.
	basecampMaxAttachments = 40
	basecampMaxMentions    = 40
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
	// matched: casado con alguien que hoy es miembro de la org.
	matched bool
	// team: casado con un miembro de la org y no marcado cliente. Todo lo
	// demás es de fuera, también una persona sin casar o casada con alguien
	// que ya dejó la org.
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
	a.matched = person != nil && person.UserID != "" && s.repo.IsMember(orgID, person.UserID)
	a.team = a.matched && !person.Client && !wire.Client
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
		AuthorUserID: author.userID(), AuthorPersonID: author.wire.ID, AuthorMatched: author.matched,
		Excerpt: domain.BasecampPlainText(ev.Recording.Content, basecampExcerpt)}
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
	// unreadable: Basecamp no deja leer el to-do o la tarjeta (borrado, un
	// «no», sin conexión). El evento sale igual, sin completar y sin volver a
	// preguntar.
	unreadable bool
}

// bcPersonOut: una persona de Basecamp en el evento. `matched: false` con
// `client: false` es alguien sin casar (o casado con quien ya no es de la
// org): la nativa y la skill dicen «cásalo en Organización → Basecamp».
type bcPersonOut struct {
	PersonID int64  `json:"personId"`
	Name     string `json:"name"`
	Client   bool   `json:"client"`
	UserID   string `json:"userId"`
	Matched  bool   `json:"matched"`
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
	enriched := !f.unreadable
	todo := f.todo
	var recent []map[string]any
	if f.unreadable {
		recent = []map[string]any{}
	} else {
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
		var ok bool
		recent, ok = s.recentComments(ctx, f.proj, targetID)
		enriched = enriched && ok
	}

	data, mentioned := s.eventData(f, todo, recent, enriched)
	assigned := map[string]bool{}
	if t, ok := data["todo"].(map[string]any); ok {
		for _, a := range t["assignees"].([]bcPersonOut) {
			if a.UserID != "" {
				assigned[a.UserID] = true
			}
		}
	}
	// Lo que escribió la propia persona no es «suyo» para `--mine`, aunque
	// esté asignada al to-do o se mencione.
	authorUID := ""
	if f.author.person != nil {
		authorUID = f.author.person.UserID
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
			Key: f.key, Mine: (mentioned[uid] || assigned[uid]) && uid != authorUID, External: !f.author.team, Data: per}); err != nil {
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
		out.UserID, out.Matched = p.UserID, true
	}
	return out
}

// eventData: el `data` del evento, igual para todos los miembros salvo `you`,
// y los usuarios que nombra (todos, también los que no caben en `mentions`).
func (s *BasecampService) eventData(f basecampFull, todo *domain.BasecampAssignable, recent []map[string]any, enriched bool) (map[string]any, map[string]bool) {
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
		Client: f.author.wire.Client || (f.author.person != nil && f.author.person.Client), UserID: f.author.userID(),
		Matched: f.author.matched}
	data["author"] = author
	data["team"] = f.author.team

	content := f.textHTML
	if content == "" {
		content = ev.Recording.Content
	}
	putText(data, "text", domain.BasecampMarkdown(content))
	mentions := []bcPersonOut{}
	mentioned := map[string]bool{}
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
			out.UserID, out.Matched = p.UserID, true
			mentioned[p.UserID] = true
		}
		mentions = append(mentions, out)
	}
	if len(mentions) > basecampMaxMentions {
		mentions = mentions[:basecampMaxMentions]
		data["mentionsTruncated"] = true
	}
	data["mentions"] = mentions
	attachments := domain.BasecampAttachments(content)
	if len(attachments) > basecampMaxAttachments {
		attachments = attachments[:basecampMaxAttachments]
		data["attachmentsTruncated"] = true
	}
	data["attachments"] = attachments

	thread := map[string]any{"recordingId": targetID, "title": targetTitle,
		"appUrl": domain.SafeBasecampURL(targetURL), "recent": recent}
	if t := f.thread; t != nil {
		thread["type"] = t.Type
		thread["awaitingReply"] = t.AwaitingReply
		thread["awaitingSince"] = t.AwaitingSince
		if t.LastExternalAt != nil {
			thread["lastExternal"] = t.LastExternalOut()
		}
		if c := t.LastClientOut(); c != nil {
			thread["lastClient"] = c
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
	return data, mentioned
}

// Awaiting: los hilos por contestar de la org.
func (s *BasecampService) Awaiting(orgID string) ([]domain.BasecampAwaiting, error) {
	return s.repo.Awaiting(orgID)
}
