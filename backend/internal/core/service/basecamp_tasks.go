package service

import (
	"errors"
	"fmt"
	"net/http"
	"strings"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	lg "github.com/guz-studio/cac/backend/internal/core/logger"
)

// La fase 3 de Basecamp (command-and-control-557): un to-do (o una tarjeta)
// nuevo o asignado es una tarea de cac enlazada a él, con sus responsables; y
// una tarea cuya descripción trae la URL de un recording se enlaza sola.
//
// Lo que no hace, a propósito: mover el estado de la tarea (completar o
// reabrir en Basecamp es una línea en el hilo; el estado lo decide el equipo,
// y las dos máquinas de estados siguen igual) ni escribir en Basecamp.

const (
	// basecampNotes: lo que se copia de las notas de un to-do a la tarea.
	basecampNotes = 4000
	// basecampTitleMax: el título de una tarea (items.title es varchar(300)).
	basecampTitleMax = 300
)

// processAssignable: un to-do o una tarjeta que se creó o cambió de
// asignados. El webhook no trae ni las notas ni los asignados, así que se le
// pregunta a Basecamp; y como se le pregunta por cómo está **ahora**, da igual
// en qué orden lleguen el alta y las asignaciones.
func (s *BasecampService) processAssignable(full basecampFull, who, actorID string) error {
	proj, ev := full.proj, full.ev
	recording := ev.Recording.ID
	// Un enlace a mano a un recording de otro proyecto no es de este secreto.
	if link, err := s.repo.LinkFor(proj.OrgID, recording); err == nil && link.ProjectID != proj.ProjectID {
		return nil
	} else if err != nil && !errors.Is(err, gorm.ErrRecordNotFound) {
		return err
	}
	path := fmt.Sprintf("/buckets/%d/todos/%d.json", proj.ProjectID, recording)
	if domain.BasecampIsCard(ev.Kind) {
		path = fmt.Sprintf("/buckets/%d/card_tables/cards/%d.json", proj.ProjectID, recording)
	}
	var rec domain.BasecampAssignable
	if _, err := s.api(proj.OrgID, http.MethodGet, path, nil, &rec); err != nil {
		switch {
		case errors.Is(err, gorm.ErrRecordNotFound):
			// Ya no existe (a la papelera antes de que llegara el evento).
			return nil
		case basecampRefused(err):
			// Un «no» de Basecamp (403: quien conectó ya no ve ese proyecto)
			// no cambia al reintentar, y desactivarían el webhook: se atiende,
			// pero quien enlazó el proyecto tiene que enterarse.
			lg.Warn(fmt.Sprintf("basecamp: reading recording %d of project %d: %v", recording, proj.ProjectID, err))
			s.notifyProjectRefused(proj)
			return nil
		case errors.Is(err, domain.ErrBasecampNotConnected), errors.Is(err, domain.ErrBasecampReconnect),
			errors.Is(err, domain.ErrBasecampOff):
			// Reintentar no lo arregla, y Basecamp desactiva el webhook tras
			// diez fallos: se perdería también lo de la fase 1. Los admins ya
			// tienen el aviso de volver a conectar.
			lg.Warn(fmt.Sprintf("basecamp: reading recording %d of project %d: %v", recording, proj.ProjectID, err))
			return nil
		}
		// Basecamp no contestó: el evento se suelta y su reintento vuelve.
		return err
	}
	if rec.ID != recording || rec.Bucket.ID != proj.ProjectID {
		return nil
	}
	item, created, err := s.taskFor(proj, ev, rec, actorID)
	if err != nil {
		return err
	}
	// El evento completo, con el to-do ya leído, antes que cualquier aviso.
	full.item, full.todo = item, &rec
	full.textHTML = rec.Description
	if domain.BasecampIsCard(ev.Kind) {
		full.textHTML = rec.Content
	}
	s.publishEvent(full)
	if item == nil {
		// Sin tarea (el espacio no tiene listas, o la que tenía se borró):
		// como en la fase 1, sólo un aviso a quien enlazó el proyecto.
		key := "notify.basecamp.todo_created"
		if ev.Kind == domain.BasecampTodoAssignment || ev.Kind == domain.BasecampCardAssignment {
			key = "notify.basecamp.todo_assigned"
		}
		s.notifyUnlinked(proj, actorID, nil, basecampNote{who: who, titleKey: key,
			excerpt: strings.TrimSpace(rec.Title), url: domain.SafeBasecampURL(rec.AppURL), key: full.key})
		return nil
	}
	if created && s.hub != nil {
		s.hub.Publish(events.Event{Type: "task:new", OrgID: item.OrgID,
			Data: map[string]string{"listId": item.ListID, "taskId": item.ID}})
	}
	return s.syncAssignees(proj, item, rec.Assignees, who, actorID, full.key)
}

// taskFor: la tarea enlazada al recording, creándola si no la hay. nil si el
// espacio no tiene listas, o si la tarea enlazada se borró: quien la borró lo
// hizo a propósito, así que no se recrea; y un error aquí haría reintentar a
// Basecamp hasta desactivar el webhook entero.
func (s *BasecampService) taskFor(proj *domain.BasecampProject, ev domain.BasecampWebhookEvent,
	rec domain.BasecampAssignable, actorID string) (*domain.Item, bool, error) {
	if link, err := s.repo.LinkFor(proj.OrgID, rec.ID); err == nil {
		it, err := s.repo.Item(link.ItemID)
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, false, nil
		}
		return it, false, err
	}
	listID := s.repo.FirstListOf(proj.SpaceID)
	if listID == "" {
		return nil, false, nil
	}
	url := domain.SafeBasecampURL(rec.AppURL)
	if url == "" {
		url = domain.SafeBasecampURL(ev.Recording.AppURL)
	}
	notes := rec.Description
	if domain.BasecampIsCard(ev.Kind) {
		notes = rec.Content
	}
	t := &domain.Item{OrgID: proj.OrgID, SpaceID: proj.SpaceID, ListID: listID,
		Title: basecampTaskTitle(rec, ev), Description: basecampTaskDescription(notes, url),
		Status: domain.ReportPending, Origin: "internal", Visibility: domain.VisibilityInternal,
		Priority: domain.ItemPriorityNone, CreatedByID: actorID}
	return s.repo.CreateLinkedTask(t, &domain.BasecampLink{OrgID: proj.OrgID, RecordingID: rec.ID,
		ProjectID: proj.ProjectID, AppURL: url})
}

func basecampTaskTitle(rec domain.BasecampAssignable, ev domain.BasecampWebhookEvent) string {
	title := strings.Join(strings.Fields(rec.Title), " ")
	if title == "" {
		title = strings.Join(strings.Fields(ev.Recording.Title), " ")
	}
	if title == "" {
		title = "Basecamp to-do"
	}
	if r := []rune(title); len(r) > basecampTitleMax {
		title = strings.TrimSpace(string(r[:basecampTitleMax-1])) + "…"
	}
	return title
}

// basecampTaskDescription: las notas en texto y, al final, de dónde viene.
func basecampTaskDescription(notesHTML, url string) string {
	notes := domain.BasecampPlainText(notesHTML, basecampNotes)
	if url == "" {
		return notes
	}
	src := "**Basecamp:** " + url
	if notes == "" {
		return src
	}
	return notes + "\n\n" + src
}

// syncAssignees deja de responsables a los asignados en Basecamp que casan
// con un miembro, y avisa a quien entra. Quita sólo a quien puso Basecamp:
// al que puso el equipo no lo toca, aunque en Basecamp ya no esté.
//
// Sin nadie de cac a cargo (nadie asignado, o alguien sin casar), se le dice
// a quien enlazó el proyecto: es lo que hacía la fase 1 con lo que no tenía a
// quién llegar.
func (s *BasecampService) syncAssignees(proj *domain.BasecampProject, item *domain.Item,
	people []domain.BasecampPersonWire, who, actorID, key string) error {
	var wanted []string
	unmatched := 0
	seen := map[string]bool{}
	for _, w := range people {
		p, err := s.repo.UpsertPerson(proj.OrgID, w)
		if err != nil {
			return err
		}
		// Casada, y aún de la org: una persona casada con quien se fue de la
		// org no le da una tarea de aquí.
		if p.UserID == "" || !s.repo.IsMember(proj.OrgID, p.UserID) {
			unmatched++
			continue
		}
		if !seen[p.UserID] {
			seen[p.UserID] = true
			wanted = append(wanted, p.UserID)
		}
	}
	changed := false
	link := "/tasks?task=" + item.ID
	for _, uid := range wanted {
		added, err := s.repo.AddBasecampAssignee(proj.OrgID, item.ID, uid)
		if err != nil {
			return err
		}
		if !added {
			continue
		}
		changed = true
		if uid != actorID && s.inbox != nil {
			s.inbox.Notify(domain.Aviso{UserID: uid, OrgID: item.OrgID, Kind: "task:assigned",
				TitleKey: "notify.item.assigned", Body: item.Title, Link: link,
				Group: domain.ItemGroup(item.ID), Label: item.Title, SourceKey: key})
		}
	}
	byBasecamp, err := s.repo.BasecampAssignees(item.ID)
	if err != nil {
		return err
	}
	for _, uid := range byBasecamp {
		if seen[uid] {
			continue
		}
		removed, err := s.repo.RemoveBasecampAssignee(item.ID, uid)
		if err != nil {
			return err
		}
		changed = changed || removed
	}
	if changed && s.hub != nil {
		s.hub.Publish(events.Event{Type: "task:update", OrgID: item.OrgID,
			Data: map[string]string{"listId": item.ListID, "taskId": item.ID}})
	}
	if (len(wanted) > 0 && unmatched == 0) || s.inbox == nil || proj.LinkedByID == "" || proj.LinkedByID == actorID {
		return nil
	}
	titleKey := "notify.basecamp.todo_unmatched"
	if len(people) == 0 {
		titleKey = "notify.basecamp.todo_unassigned"
	}
	s.inbox.Notify(domain.Aviso{UserID: proj.LinkedByID, OrgID: item.OrgID, Kind: domain.BasecampActivityKind,
		TitleKey: titleKey, TitleArgs: map[string]string{"who": who}, Body: item.Title, Link: link,
		Group: domain.ItemGroup(item.ID), Label: item.Title, SourceKey: key})
	return nil
}

// ─── Enlazar por la descripción ──────────────────────────────────────────────

// LinkFromDescription enlaza la tarea a cada recording de la cuenta conectada
// cuya URL trae su descripción, si ese recording no está enlazado ya (a ésta o
// a otra: el enlace que había gana, y no se avisa de nada). Lo llaman crear y
// editar una tarea; un fallo aquí no tumba lo que la persona guardó.
func (s *BasecampService) LinkFromDescription(item *domain.Item) {
	if item == nil {
		return
	}
	urls := domain.BasecampURLsIn(item.Description)
	if len(urls) == 0 {
		return
	}
	conn, err := s.repo.Connection(item.OrgID)
	if err != nil {
		return
	}
	for _, raw := range urls {
		account, project, recording, err := domain.ParseBasecampRecordingURL(raw)
		if err != nil || account != conn.AccountID {
			continue
		}
		if _, err := s.repo.LinkFor(item.OrgID, recording); err == nil {
			continue
		}
		err = s.repo.CreateLink(&domain.BasecampLink{OrgID: item.OrgID, ItemID: item.ID, RecordingID: recording,
			ProjectID: project, AppURL: domain.SafeBasecampURL(raw)})
		if err != nil && !errors.Is(err, domain.ErrBasecampLinkTaken) {
			lg.Warn("basecamp: linking a task from its description: " + err.Error())
		}
	}
}

// ─── Los webhooks de los proyectos ya enlazados ──────────────────────────────

// ReconcileWebhooks pone al día los tipos de los webhooks dados de alta con
// otros (los de la fase 1, o una versión anterior): cambia el webhook que hay
// (PUT con su misma dirección), nunca da de alta otro. La dirección se le
// pide a Basecamp, porque aquí sólo se guarda el hash de su secreto; y si el
// secreto que devuelve no es el de la fila, no se toca.
func (s *BasecampService) ReconcileWebhooks() {
	if !s.Configured() {
		return
	}
	want := domain.BasecampTypesKey(domain.BasecampWebhookTypes)
	projects, err := s.repo.AllProjects()
	if err != nil {
		lg.Error("basecamp: listing projects to reconcile webhooks: " + err.Error())
		return
	}
	for _, p := range projects {
		have := p.WebhookTypes
		if have == "" {
			have = domain.BasecampTypesKey(domain.BasecampLegacyWebhookTypes)
		}
		if have == want || p.WebhookID == 0 {
			continue
		}
		if err := s.reconcileWebhook(&p, want); err != nil {
			lg.Warn(fmt.Sprintf("basecamp: updating webhook %d of project %d: %v", p.WebhookID, p.ProjectID, err))
		}
	}
}

func (s *BasecampService) reconcileWebhook(p *domain.BasecampProject, want string) error {
	path := fmt.Sprintf("/buckets/%d/webhooks/%d.json", p.ProjectID, p.WebhookID)
	var hook struct {
		PayloadURL string `json:"payload_url"`
	}
	if _, err := s.api(p.OrgID, http.MethodGet, path, nil, &hook); err != nil {
		return err
	}
	secret := hook.PayloadURL[strings.LastIndex(hook.PayloadURL, "/")+1:]
	if secret == "" || domain.BasecampSecretHash(secret) != p.SecretHash {
		return errors.New("the webhook's address is not this project's")
	}
	if _, err := s.api(p.OrgID, http.MethodPut, path, map[string]any{
		"payload_url": hook.PayloadURL,
		"types":       domain.BasecampWebhookTypes,
	}, nil); err != nil {
		return err
	}
	return s.repo.SetWebhookTypes(p.ID, want)
}

func (s *BasecampService) notifyProjectRefused(proj *domain.BasecampProject) {
	if s.inbox == nil || proj.LinkedByID == "" {
		return
	}
	s.inbox.Notify(domain.Aviso{UserID: proj.LinkedByID, OrgID: proj.OrgID, Kind: domain.BasecampActivityKind,
		TitleKey: "notify.basecamp.project_refused", TitleArgs: map[string]string{"project": proj.Name},
		Link: "/organization", Group: domain.BasecampProjectGroup(proj.ProjectID), Label: proj.Name})
}
