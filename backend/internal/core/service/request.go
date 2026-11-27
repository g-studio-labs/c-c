package service

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"time"

	"github.com/google/uuid"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Los pedidos de los agentes a una persona (command-and-control-500 y 504).
// Ver domain/request.go para qué es un pedido y por qué no es un estado.

var (
	// ErrRequestNotYours: contestar (o abrir en la app) un pedido hecho a otra
	// persona. En «Equipo» se ven, pero nadie contesta por otro.
	ErrRequestNotYours = errors.New("request-not-yours")
	// ErrRequestNotPublishable: «publicado» sólo cabe en un mensaje aprobado.
	ErrRequestNotPublishable = errors.New("request-not-publishable")
	ErrRequestKind           = errors.New("request-kind-unknown")
	ErrRequestTask           = errors.New("request-task-not-found")
)

type RequestService struct {
	repo  *repository.RequestRepository
	tasks *repository.TaskRepository
	docs  *repository.DocRepository
	hub   *events.Hub
	inbox Notifier
	now   func() time.Time
	// openAck: cuánto se espera a que una app abierta diga «lo abrí». Una app
	// conectada contesta en milisegundos; sin ninguna, esto es lo que tarda el
	// MCP en decir que no llegó.
	openAck time.Duration
	// waitCap: lo más que dura una espera larga. Por debajo del WriteTimeout
	// del servidor (15 s) y del plazo del cliente del MCP (30 s): el MCP
	// encadena esperas hasta su propio plazo.
	waitCap time.Duration
	// afterCheck: para las pruebas, avisa cada vez que la espera se pone a
	// escuchar (tras mirar la base, y tras cada evento que no era el suyo).
	// Nil fuera de ellas.
	afterCheck func()
}

func NewRequestService(repo *repository.RequestRepository, tasks *repository.TaskRepository,
	docs *repository.DocRepository, hub *events.Hub, inbox Notifier) *RequestService {
	return &RequestService{
		repo: repo, tasks: tasks, docs: docs, hub: hub, inbox: inbox,
		now:     func() time.Time { return time.Now().UTC() },
		openAck: 4 * time.Second,
		waitCap: 12 * time.Second,
	}
}

// ResolveTask: la tarea por su id o por su ref (`proteus-12`), entre las
// organizaciones dadas. La autorización la hace quien llama, con la org de la
// tarea.
func (s *RequestService) ResolveTask(orgIDs []string, idOrRef string) (*domain.Task, error) {
	key := strings.TrimSpace(idOrRef)
	if t, err := s.tasks.FindTask(key); err == nil {
		return t, nil
	}
	refs := domain.ExtractTaskRefs(strings.ToLower(key), false)
	if len(refs) == 0 || refs[0].Slug == "" {
		return nil, ErrRequestTask
	}
	t, err := s.tasks.FindByRef(orgIDs, refs[0])
	if err != nil {
		return nil, ErrRequestTask
	}
	return t, nil
}

func looksLikeUUID(s string) bool {
	_, err := uuid.Parse(s)
	return err == nil
}

func (s *RequestService) Find(idOrCode string) (*domain.Request, error) {
	return s.repo.Find(idOrCode)
}

// resolvePerson: a quién se le pide. Vacío es quien llama —lo corriente: el
// agente le pide a la persona de su token—. Si no, un id, un usuario o un
// nombre de un miembro de la organización, y sólo uno.
func (s *RequestService) resolvePerson(orgID, callerID, who string) (string, error) {
	who = strings.TrimSpace(who)
	if who == "" {
		return callerID, nil
	}
	if looksLikeUUID(who) && s.repo.IsMember(orgID, who) {
		return who, nil
	}
	matches := s.docs.MembersMatching(orgID, strings.TrimPrefix(who, "@"))
	if len(matches) != 1 {
		return "", &ErrNoSuchColleague{Buscado: who, Opciones: aQuienesSePuedeNombrar(matches, s.docs.OrgMemberNames(orgID))}
	}
	return matches[0].ID, nil
}

func decodeBody(kind domain.RequestKind, raw json.RawMessage) ([]byte, error) {
	var b domain.RequestBody
	if len(raw) > 0 && string(raw) != "null" {
		if err := json.Unmarshal(raw, &b); err != nil {
			return nil, &domain.RequestBodyError{Msg: "body is not the shape of its kind: " + err.Error()}
		}
	}
	clean, err := b.Normalize(kind)
	if err != nil {
		return nil, err
	}
	return json.Marshal(clean)
}

// Create deja el pedido y avisa a la persona.
func (s *RequestService) Create(task *domain.Task, callerID, via string, req domain.CreateRequestRequest) (*domain.RequestView, error) {
	if !domain.ValidRequestKind(req.Kind) {
		return nil, ErrRequestKind
	}
	summary := strings.TrimSpace(req.Summary)
	if summary == "" {
		return nil, &domain.RequestBodyError{Msg: "summary is required"}
	}
	body, err := decodeBody(req.Kind, req.Body)
	if err != nil {
		return nil, err
	}
	of, err := s.resolvePerson(task.OrgID, callerID, req.RequestedOf)
	if err != nil {
		return nil, err
	}
	q := &domain.Request{
		OrgID: task.OrgID, TaskID: task.ID, Kind: req.Kind, Summary: summary, Body: body,
		Version: 1, RequestedOfID: of, RequestedByID: callerID, Via: domain.NormalizeVia(via),
		Status: domain.RequestOpen,
	}
	if err := s.repo.Create(q); err != nil {
		return nil, err
	}
	s.announce(domain.RequestCreatedEvent, q)
	s.notify(q, task, callerID, via)
	v := s.View(q, callerID, false)
	return &v, nil
}

// notify: la campana (y el teléfono) de la persona. Si la persona se lo pide a
// sí misma desde la app no hace falta avisarle; si lo pide su agente sí, que
// es justo el caso para el que existe esto.
func (s *RequestService) notify(q *domain.Request, task *domain.Task, callerID, via string) {
	if s.inbox == nil || (q.RequestedOfID == callerID && domain.NormalizeVia(via) != domain.ViaMCP) {
		return
	}
	s.inbox.Notify(domain.Aviso{
		UserID: q.RequestedOfID, OrgID: q.OrgID, Kind: domain.RequestNotifyKind,
		TitleKey:  "notify.request.new",
		TitleArgs: map[string]string{"task": task.Title},
		Body:      q.Summary,
		Link:      domain.RequestLink(q.Code),
		Via:       via,
		Group:     domain.ItemGroup(task.ID),
		Label:     task.Title,
	})
}

// announce: el evento a la organización, para que la bandeja (y su contador)
// se pinte sola. Sólo ids y estado: el contenido se lee con permisos.
func (s *RequestService) announce(kind string, q *domain.Request) {
	if s.hub == nil {
		return
	}
	s.hub.Publish(events.Event{Type: kind, OrgID: q.OrgID, Data: map[string]string{
		"id": q.ID, "code": q.Code, "taskId": q.TaskID, "requestedOfId": q.RequestedOfID, "status": string(q.Status),
	}})
}

// Update rehace un pedido abierto: lo de antes pasa al historial.
func (s *RequestService) Update(q *domain.Request, callerID, via string, req domain.UpdateRequestRequest) (*domain.RequestView, error) {
	// Que siga abierto lo comprueba Reissue al escribir, no aquí: entre leer y
	// escribir lo puede contestar el botón.
	kind := q.Kind
	if req.Kind != nil {
		kind = *req.Kind
		if !domain.ValidRequestKind(kind) {
			return nil, ErrRequestKind
		}
	}
	fields := map[string]any{"requested_by_id": callerID, "via": domain.NormalizeVia(via), "kind": kind}
	if req.Summary != nil {
		sum := strings.TrimSpace(*req.Summary)
		if sum == "" {
			return nil, &domain.RequestBodyError{Msg: "summary cannot be empty"}
		}
		fields["summary"] = sum
	}
	switch {
	case len(req.Body) > 0 && string(req.Body) != "null":
		body, err := decodeBody(kind, req.Body)
		if err != nil {
			return nil, err
		}
		fields["body"] = body
	case kind != q.Kind:
		return nil, &domain.RequestBodyError{Msg: "changing the kind needs the new body"}
	default:
		// Mismo cuerpo, pero se valida otra vez: un cuerpo guardado sigue
		// siendo de su tipo.
	}
	if req.RequestedOf != nil {
		of, err := s.resolvePerson(q.OrgID, callerID, *req.RequestedOf)
		if err != nil {
			return nil, err
		}
		fields["requested_of_id"] = of
	}
	if len(fields) == 3 && kind == q.Kind {
		return nil, &domain.RequestBodyError{Msg: "nothing to change: send summary, body, kind or requestedOf"}
	}
	if err := s.repo.Reissue(*q, fields); err != nil {
		return nil, err
	}
	fresh, err := s.repo.Find(q.ID)
	if err != nil {
		return nil, err
	}
	s.announce(domain.RequestUpdatedEvent, fresh)
	if task, err := s.tasks.FindTask(fresh.TaskID); err == nil {
		s.notify(fresh, task, callerID, via)
	}
	v := s.View(fresh, callerID, true)
	return &v, nil
}

// Answer: sólo la persona a quien se le pidió. `viaSession` es que lo apunta
// una sesión porque ella lo dijo en la terminal (con su propio token).
func (s *RequestService) Answer(q *domain.Request, callerID string, viaSession bool, a domain.RequestAnswer) (*domain.RequestView, error) {
	if q.RequestedOfID != callerID {
		return nil, ErrRequestNotYours
	}
	if q.Status != domain.RequestOpen {
		return nil, repository.ErrRequestNotOpen
	}
	answeredVia := domain.AnsweredViaApp
	if viaSession {
		answeredVia = domain.AnsweredViaSession
	}
	return s.answerVia(q, callerID, answeredVia, a)
}

func (s *RequestService) answerVia(q *domain.Request, callerID, answeredVia string, a domain.RequestAnswer) (*domain.RequestView, error) {
	var body domain.RequestBody
	_ = json.Unmarshal(q.Body, &body)
	clean, err := a.Normalize(q.Kind, body)
	if err != nil {
		return nil, err
	}
	raw, _ := json.Marshal(clean)
	if err := s.repo.Settle(q.ID, map[string]any{
		"status": domain.RequestAnswered, "answer": raw, "answered_by_id": callerID,
		"answered_at": s.now(), "answered_via": answeredVia,
	}); err != nil {
		return nil, err
	}
	fresh, err := s.repo.Find(q.ID)
	if err != nil {
		return nil, err
	}
	s.announce(domain.RequestAnsweredEvent, fresh)
	s.notifyAnswered(fresh, answeredVia, clean)
	v := s.View(fresh, callerID, false)
	return &v, nil
}

// notifyAnswered: a quien lo pidió, que su sesión se entere de la respuesta
// sin preguntar (command-and-control-558). No si la apuntó su propia sesión:
// ya la sabe, es quien la escribió. El `finalText` de un mensaje aprobado y la
// nota van enteros en `detail`, que el cuerpo de la campana no lleva.
func (s *RequestService) notifyAnswered(q *domain.Request, answeredVia string, a domain.RequestAnswer) {
	if s.inbox == nil || q.RequestedByID == "" || answeredVia == domain.AnsweredViaSession {
		return
	}
	title := ""
	if t, err := s.tasks.FindTask(q.TaskID); err == nil {
		title = t.Title
	}
	detail := map[string]any{"code": q.Code, "kind": q.Kind, "finalText": a.FinalText, "note": a.Note, "reason": a.Reason}
	if a.Approved != nil {
		detail["approved"] = *a.Approved
	}
	s.inbox.Notify(domain.Aviso{
		UserID: q.RequestedByID, OrgID: q.OrgID, Kind: domain.RequestAnsweredNotifyKind,
		TitleKey:  "notify.request.answered",
		TitleArgs: map[string]string{"task": title},
		Body:      q.Summary,
		Link:      domain.RequestLink(q.Code),
		Group:     domain.ItemGroup(q.TaskID),
		Label:     title,
		Detail:    detail,
	})
}

// AnswerFromBasecamp: lo que `userID` escribió en el hilo de Basecamp enlazado
// a la tarea contesta su pedido abierto en ella (command-and-control-516,
// punto 5). Sólo los que se contestan con texto (conseguir y correr): un
// «decide» o un «probar» piden un botón, y adivinarlo de una frase sería
// contestar por la persona. Sólo los suyos: nadie contesta por otro. Y sólo
// si es **uno**: con dos abiertos, un comentario no dice a cuál contesta (ni
// si contesta alguno: puede ser un «gracias»), así que no contesta ninguno y
// devuelve cuántos hay para que se le avise.
func (s *RequestService) AnswerFromBasecamp(orgID, taskID, userID, text string) (answered bool, open int, err error) {
	if s == nil || orgID == "" || taskID == "" || userID == "" || strings.TrimSpace(text) == "" {
		return false, 0, nil
	}
	list, err := s.repo.List(repository.RequestFilter{OrgIDs: []string{orgID}, TaskID: taskID, RequestedOf: userID,
		Statuses: []domain.RequestStatus{domain.RequestOpen}})
	if err != nil {
		return false, 0, err
	}
	var textual []*domain.Request
	for i := range list {
		if list[i].Kind == domain.RequestObtain || list[i].Kind == domain.RequestRun {
			textual = append(textual, &list[i])
		}
	}
	if len(textual) != 1 {
		return false, len(textual), nil
	}
	q := textual[0]
	a := domain.RequestAnswer{Note: text}
	if q.Kind == domain.RequestRun {
		// Lo que se pega en un «correr» es lo que salió del comando.
		a = domain.RequestAnswer{Output: text}
	}
	if _, err := s.answerVia(q, userID, domain.AnsweredViaBasecamp, a); err != nil {
		return false, 1, err
	}
	return true, 1, nil
}

// Withdraw: el agente ya no lo necesita. Cualquiera que escriba en la org: la
// sesión que lo retira no tiene por qué ser la que lo pidió.
func (s *RequestService) Withdraw(q *domain.Request, callerID string) (*domain.RequestView, error) {
	if err := s.repo.Settle(q.ID, map[string]any{"status": domain.RequestWithdrawn, "withdrawn_at": s.now()}); err != nil {
		return nil, err
	}
	fresh, err := s.repo.Find(q.ID)
	if err != nil {
		return nil, err
	}
	s.announce(domain.RequestWithdrawnEvent, fresh)
	v := s.View(fresh, callerID, false)
	return &v, nil
}

// Publish: dónde acabó el mensaje aprobado.
func (s *RequestService) Publish(q *domain.Request, callerID, url string) (*domain.RequestView, error) {
	var a domain.RequestAnswer
	_ = json.Unmarshal(q.Answer, &a)
	if q.Kind != domain.RequestApproveMessage || q.Status != domain.RequestAnswered || a.Approved == nil || !*a.Approved {
		return nil, ErrRequestNotPublishable
	}
	if err := s.repo.SetPublished(q.ID, strings.TrimSpace(url)); err != nil {
		return nil, err
	}
	fresh, err := s.repo.Find(q.ID)
	if err != nil {
		return nil, err
	}
	v := s.View(fresh, callerID, false)
	return &v, nil
}

// List: la bandeja.
func (s *RequestService) List(f repository.RequestFilter, viewerID string) ([]domain.RequestView, error) {
	rows, err := s.repo.List(f)
	if err != nil {
		return nil, err
	}
	out := make([]domain.RequestView, 0, len(rows))
	refs := map[string]domain.RequestTaskRef{}
	names := map[string]string{}
	for i := range rows {
		out = append(out, s.view(&rows[i], viewerID, false, refs, names))
	}
	return out, nil
}

// View: el pedido para leer; con `detail`, también su historial y el handoff
// de la tarea, que es lo que una sesión necesita para seguir con él.
func (s *RequestService) View(q *domain.Request, viewerID string, detail bool) domain.RequestView {
	return s.view(q, viewerID, detail, map[string]domain.RequestTaskRef{}, map[string]string{})
}

func (s *RequestService) view(q *domain.Request, viewerID string, detail bool,
	refs map[string]domain.RequestTaskRef, names map[string]string) domain.RequestView {
	name := func(id string) string {
		if id == "" {
			return ""
		}
		n, ok := names[id]
		if !ok {
			n = s.repo.DisplayName(id)
			names[id] = n
		}
		return n
	}
	var body domain.RequestBody
	_ = json.Unmarshal(q.Body, &body)
	v := domain.RequestView{
		ID: q.ID, Code: q.Code, OrgID: q.OrgID, Kind: q.Kind, Summary: q.Summary, Body: body,
		Version: q.Version, Status: q.Status, Via: q.Via, CreatedAt: q.CreatedAt, UpdatedAt: q.UpdatedAt,
		RequestedOf: domain.RequestPerson{ID: q.RequestedOfID, Name: name(q.RequestedOfID)},
		RequestedBy: domain.RequestPerson{ID: q.RequestedByID, Name: name(q.RequestedByID)},
		AnsweredAt:  q.AnsweredAt, AnsweredVia: q.AnsweredVia, Published: q.PublishedURL, WithdrawnAt: q.WithdrawnAt,
		CanAnswer: q.Status == domain.RequestOpen && q.RequestedOfID == viewerID,
		Link:      domain.RequestLink(q.Code),
	}
	if len(q.Answer) > 0 {
		var a domain.RequestAnswer
		if json.Unmarshal(q.Answer, &a) == nil {
			v.Answer = &a
		}
	}
	if q.AnsweredByID != "" {
		v.AnsweredBy = &domain.RequestPerson{ID: q.AnsweredByID, Name: name(q.AnsweredByID)}
	}
	ref, ok := refs[q.TaskID]
	var task *domain.Task
	if !ok || detail {
		if t, err := s.tasks.FindTask(q.TaskID); err == nil {
			task = t
			ref = domain.RequestTaskRef{
				ID: t.ID, Ref: s.taskRef(t), Title: t.Title,
				Priority: string(t.Priority.TaskWire()), Status: string(t.Status),
				Link: "/tasks?task=" + t.ID,
			}
		} else {
			ref = domain.RequestTaskRef{ID: q.TaskID, Link: "/tasks?task=" + q.TaskID}
		}
		refs[q.TaskID] = ref
	}
	v.Task = ref
	if detail {
		if hist, err := s.repo.History(q.ID); err == nil {
			for _, h := range hist {
				var b domain.RequestBody
				_ = json.Unmarshal(h.Body, &b)
				v.History = append(v.History, domain.RequestVersionView{
					Version: h.Version, Kind: h.Kind, Summary: h.Summary, Body: b,
					By: domain.RequestPerson{ID: h.ByID, Name: name(h.ByID)}, CreatedAt: h.CreatedAt,
				})
			}
		}
		if task != nil && strings.TrimSpace(task.Handoff) != "" {
			v.Handoff = &domain.RequestHandoff{Text: task.Handoff, At: task.HandoffAt, By: name(task.HandoffByID)}
		}
	}
	return v
}

// taskRef: cómo se nombra la tarea (el folio de un cliente o la clave de su
// espacio y su número). La misma regla que TaskService.Detail.
func (s *RequestService) taskRef(t *domain.Task) string {
	if t.ProjectID != "" {
		if slug := s.tasks.ProjectSlug(t.ProjectID); slug != "" {
			return domain.Folio(slug, t.Seq)
		}
		return ""
	}
	if sp, err := s.tasks.FindSpace(t.SpaceID); err == nil && sp != nil {
		return domain.SpaceTaskRef(sp.Key, t.Seq)
	}
	return ""
}

// OpenRequestsOfTasks: suelta para que la usen el detalle de la tarea y
// get_context sin construir el servicio entero.
func OpenRequestsOfTasks(repo *repository.RequestRepository, taskIDs []string) map[string]*domain.TaskOpenRequests {
	out := map[string]*domain.TaskOpenRequests{}
	open, err := repo.OpenOfTasks(taskIDs)
	if err != nil {
		return out
	}
	names := map[string]string{}
	for taskID, qs := range open {
		mark := &domain.TaskOpenRequests{WaitingOn: []string{}, Codes: []string{}}
		seen := map[string]bool{}
		for _, q := range qs {
			mark.Count++
			mark.Codes = append(mark.Codes, q.Code)
			if seen[q.RequestedOfID] {
				continue
			}
			seen[q.RequestedOfID] = true
			n, ok := names[q.RequestedOfID]
			if !ok {
				n = repo.DisplayName(q.RequestedOfID)
				names[q.RequestedOfID] = n
			}
			mark.WaitingOn = append(mark.WaitingOn, n)
		}
		out[taskID] = mark
	}
	return out
}

// MyOpen: cuántos pedidos abiertos tiene alguien y los primeros, para la
// sesión que arranca.
func (s *RequestService) MyOpen(orgIDs []string, userID string, top int) domain.MyOpenRequests {
	out := domain.MyOpenRequests{Top: []domain.MyOpenRequestLine{}}
	rows, err := s.List(repository.RequestFilter{OrgIDs: orgIDs, RequestedOf: userID, Statuses: []domain.RequestStatus{domain.RequestOpen}}, userID)
	if err != nil {
		return out
	}
	out.Count = len(rows)
	for i, v := range rows {
		if i >= top {
			break
		}
		out.Top = append(out.Top, domain.MyOpenRequestLine{Code: v.Code, Kind: v.Kind, Summary: v.Summary, TaskRef: v.Task.Ref, Link: v.Link})
	}
	return out
}

// ─── Abrirlo en la app, y esperar la respuesta ───────────────────────────────

// eventField lee un campo del Data de un evento. Llega como el mapa que se
// publicó (mismo pod) o como lo que dejó el JSON del bus (otro pod).
func eventField(data any, key string) string {
	switch d := data.(type) {
	case map[string]string:
		return d[key]
	case map[string]any:
		s, _ := d[key].(string)
		return s
	}
	raw, err := json.Marshal(data)
	if err != nil {
		return ""
	}
	var m map[string]any
	if json.Unmarshal(raw, &m) != nil {
		return ""
	}
	s, _ := m[key].(string)
	return s
}

// Open le pide a la app abierta de la persona que enseñe el pedido, y espera a
// que alguna diga que lo abrió. El backend es el puente, así que vale igual
// para el escritorio que para la web, y en cualquier pod: el «lo abrí» viaja
// por el mismo bus que el evento.
//
// `delivered: false` es honesto: ninguna app contestó a tiempo. No se puede
// saber de otra forma —una conexión viva en otro pod no se ve desde éste— y
// tampoco haría falta: lo que importa es si alguien lo enseñó.
func (s *RequestService) Open(ctx context.Context, q *domain.Request, callerID string) (domain.RequestOpened, error) {
	out := domain.RequestOpened{Link: domain.RequestLink(q.Code)}
	if q.RequestedOfID != callerID {
		return out, ErrRequestNotYours
	}
	if s.hub == nil {
		return out, nil
	}
	nonce := uuid.NewString()
	// Suscrito antes de publicar: un «lo abrí» que llegara antes de escuchar se
	// perdería y diría que no hay app.
	ch, stop := s.hub.Subscribe(callerID, nil)
	defer stop()
	s.hub.Publish(events.Event{Type: domain.RequestOpenEvent, OrgID: q.OrgID, UserID: callerID,
		Data: map[string]string{"id": q.ID, "code": q.Code, "nonce": nonce}})
	timer := time.NewTimer(s.openAck)
	defer timer.Stop()
	for {
		select {
		case <-ctx.Done():
			return out, nil
		case <-timer.C:
			return out, nil
		case ev, ok := <-ch:
			if !ok {
				return out, nil
			}
			if ev.Type == domain.RequestOpenedEvent && eventField(ev.Data, "nonce") == nonce {
				out.Delivered = true
				return out, nil
			}
		}
	}
}

// Opened: la app dice que enseñó el pedido. Va sólo a la misma persona, que es
// quien escucha en Open.
func (s *RequestService) Opened(userID, orgID, nonce string) {
	if s.hub == nil || nonce == "" {
		return
	}
	s.hub.Publish(events.Event{Type: domain.RequestOpenedEvent, OrgID: orgID, UserID: userID,
		Data: map[string]string{"nonce": nonce}})
}

// Wait espera a que el pedido deje de estar abierto o a que pase `timeout`
// (como mucho waitCap), y devuelve cómo quedó. Despierta con el evento, no
// preguntando a la base cada tanto.
func (s *RequestService) Wait(ctx context.Context, q *domain.Request, viewerID string, timeout time.Duration) (domain.RequestWait, error) {
	if timeout <= 0 || timeout > s.waitCap {
		timeout = s.waitCap
	}
	var ch <-chan events.Event
	if s.hub != nil {
		var stop func()
		// Suscrito antes de mirar la base: una respuesta entre las dos cosas
		// no se pierde.
		ch, stop = s.hub.Subscribe(viewerID, []string{q.OrgID})
		defer stop()
	}
	settled := func() (domain.RequestWait, error) {
		fresh, err := s.repo.Find(q.ID)
		if err != nil {
			return domain.RequestWait{}, err
		}
		v := s.View(fresh, viewerID, false)
		return domain.RequestWait{Settled: fresh.Status != domain.RequestOpen, Request: &v}, nil
	}
	if w, err := settled(); err != nil || w.Settled {
		return w, err
	}
	timer := time.NewTimer(timeout)
	defer timer.Stop()
	for {
		if s.afterCheck != nil {
			s.afterCheck()
		}
		select {
		case <-ctx.Done():
			return settled()
		case <-timer.C:
			return settled()
		case ev, ok := <-ch:
			if !ok {
				return settled()
			}
			if (ev.Type == domain.RequestAnsweredEvent || ev.Type == domain.RequestWithdrawnEvent) && eventField(ev.Data, "id") == q.ID {
				return settled()
			}
		}
	}
}
