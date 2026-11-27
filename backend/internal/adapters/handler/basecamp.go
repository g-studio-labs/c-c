package handler

import (
	"errors"
	"io"
	"net/http"
	"strconv"

	"github.com/go-chi/chi/v5"
	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// BasecampHandler: la integración con Basecamp (ver domain/basecamp.go).
type BasecampHandler struct{ svc *service.BasecampService }

func NewBasecampHandler(svc *service.BasecampService) *BasecampHandler {
	return &BasecampHandler{svc: svc}
}

// Webhook: `POST /webhooks/basecamp/{secret}`, fuera del JWT. Basecamp no
// firma: el secreto de la dirección es la prueba. Uno que no existe es un 404
// sin cuerpo, el mismo que daría una ruta que no existe.
func (h *BasecampHandler) Webhook(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxWebhookBody))
	if err != nil {
		w.WriteHeader(http.StatusRequestEntityTooLarge)
		return
	}
	switch err := h.svc.HandleWebhook(chi.URLParam(r, "secret"), body); {
	case err == nil:
		w.WriteHeader(http.StatusNoContent)
	case errors.Is(err, service.ErrBasecampUnknownHook):
		w.WriteHeader(http.StatusNotFound)
	default:
		// Basecamp reintenta lo que no fue 2xx.
		w.WriteHeader(http.StatusInternalServerError)
	}
}

// Setup: `GET /webhooks/basecamp/setup`, la vuelta del navegador desde
// Launchpad. Una página corta: la lee una persona.
func (h *BasecampHandler) Setup(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	if q.Get("error") != "" {
		setupPage(w, http.StatusBadRequest, "Basecamp was not connected", "The authorization was cancelled. Start again from cac: Organization → Integrations.")
		return
	}
	err := h.svc.Setup(q.Get("code"), q.Get("state"))
	switch {
	case err == nil:
		setupPage(w, http.StatusOK, "Basecamp is connected to cac", "You can close this tab and go back to cac.")
	case errors.Is(err, domain.ErrBasecampOff):
		setupPage(w, http.StatusServiceUnavailable, "Basecamp is not configured", "This cac server has no Basecamp app set up.")
	case errors.Is(err, domain.ErrBasecampNoAccount):
		setupPage(w, http.StatusBadRequest, "No Basecamp account", "That login has no Basecamp account cac can use.")
	case errors.Is(err, domain.ErrBasecampBadState):
		setupPage(w, http.StatusBadRequest, "This link has expired", "Start again from cac: Organization → Integrations.")
	case errors.Is(err, domain.ErrBasecampReconnect):
		setupPage(w, http.StatusBadRequest, "Basecamp did not accept the authorization", "Start again from cac: Organization → Integrations.")
	default:
		setupPage(w, http.StatusBadGateway, "Basecamp did not answer", "Try again in a moment from cac: Organization → Integrations.")
	}
}

func basecampError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, domain.ErrBasecampOff):
		SendErrorResponse(w, http.StatusServiceUnavailable, "Basecamp is not configured", "basecamp-off")
	case errors.Is(err, domain.ErrBasecampNotConnected):
		SendErrorResponse(w, http.StatusConflict, "Basecamp is not connected", "basecamp-not-connected")
	case errors.Is(err, domain.ErrBasecampBadURL):
		SendErrorResponse(w, http.StatusBadRequest, "That is not a link to this Basecamp account", "basecamp-bad-url")
	case errors.Is(err, domain.ErrBasecampLinkTaken):
		SendErrorResponse(w, http.StatusConflict, "That Basecamp item is linked to another task", "basecamp-link-taken")
	case errors.Is(err, domain.ErrBasecampUserOutside):
		SendErrorResponse(w, http.StatusBadRequest, "That user is not in this organization", "user-outside-org")
	case errors.Is(err, service.ErrSpaceOutsideOrg):
		SendErrorResponse(w, http.StatusBadRequest, "That space is not in this organization", "space-outside-org")
	case errors.Is(err, gorm.ErrRecordNotFound):
		SendErrorResponse(w, http.StatusNotFound, "Not found", "not-found")
	case errors.Is(err, domain.ErrBasecampReconnect):
		SendErrorResponse(w, http.StatusConflict, "Basecamp asks to connect again", "basecamp-reconnect")
	case errors.Is(err, domain.ErrBasecampUnreachable):
		SendErrorResponse(w, http.StatusBadGateway, "Basecamp did not answer", "basecamp-unreachable")
	default:
		SendErrorResponse(w, http.StatusInternalServerError, "Basecamp failed", "basecamp-failed")
	}
}

// Status: `GET /api/v1/organizations/{id}/basecamp`, para cualquiera de la org.
func (h *BasecampHandler) Status(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleViewer)
	if !ok {
		return
	}
	out, err := h.svc.Status(orgID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.BasecampStatus]{Success: true, Data: out})
}

// Awaiting: `GET …/basecamp/awaiting`, para cualquiera de la org: los hilos
// de Basecamp que esperan respuesta del equipo, del más viejo al más nuevo.
func (h *BasecampHandler) Awaiting(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleViewer)
	if !ok {
		return
	}
	out, err := h.svc.Awaiting(orgID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.BasecampAwaiting]{Success: true, Data: out})
}

// Connect: `POST …/basecamp/connect`, admin. Devuelve a dónde mandar el
// navegador.
func (h *BasecampHandler) Connect(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleAdmin)
	if !ok {
		return
	}
	user, _ := currentUser(r)
	u, err := h.svc.ConnectURL(orgID, user.UserID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.BasecampConnectResponse]{Success: true,
		Data: &domain.BasecampConnectResponse{URL: u}})
}

// Disconnect: `DELETE …/basecamp`, admin.
func (h *BasecampHandler) Disconnect(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleAdmin)
	if !ok {
		return
	}
	if err := h.svc.Disconnect(orgID); err != nil {
		basecampError(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// AvailableProjects: `GET …/basecamp/projects`, admin.
func (h *BasecampHandler) AvailableProjects(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleAdmin)
	if !ok {
		return
	}
	out, err := h.svc.AvailableProjects(orgID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.BasecampAvailableProject]{Success: true, Data: out})
}

func int64Param(r *http.Request, name string) int64 {
	n, _ := strconv.ParseInt(chi.URLParam(r, name), 10, 64)
	return n
}

// LinkProject: `PUT …/basecamp/projects/{projectId}` con `{spaceId}`, admin.
func (h *BasecampHandler) LinkProject(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleAdmin)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.LinkBasecampProjectRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	user, _ := currentUser(r)
	p, err := h.svc.LinkProject(orgID, user.UserID, int64Param(r, "projectId"), req.SpaceID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.BasecampProject]{Success: true, Data: p})
}

// UnlinkProject: `DELETE …/basecamp/projects/{projectId}`, admin.
func (h *BasecampHandler) UnlinkProject(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleAdmin)
	if !ok {
		return
	}
	if err := h.svc.UnlinkProject(orgID, int64Param(r, "projectId")); err != nil {
		basecampError(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// People: `GET …/basecamp/people`, para cualquiera de la org.
func (h *BasecampHandler) People(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleViewer)
	if !ok {
		return
	}
	out, err := h.svc.People(orgID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.BasecampPerson]{Success: true, Data: out})
}

// MatchPerson: `PUT …/basecamp/people/{personId}` con `{userId}`, admin.
func (h *BasecampHandler) MatchPerson(w http.ResponseWriter, r *http.Request) {
	orgID, ok := scopeOrg(w, r, domain.OrgRoleAdmin)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.MatchBasecampPersonRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	p, err := h.svc.MatchPerson(orgID, int64Param(r, "personId"), req.UserID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.BasecampPerson]{Success: true, Data: p})
}

// taskInOrg: la tarea de la URL, para un miembro de su org con el rol pedido
// (404 a quien no es de la org, como a una tarea que no existe).
func (h *BasecampHandler) taskInOrg(w http.ResponseWriter, r *http.Request, write bool) (*domain.Item, bool) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return nil, false
	}
	item, err := h.svc.Task(chi.URLParam(r, "id"))
	if err != nil {
		SendErrorResponse(w, http.StatusNotFound, "Task not found", "not-found")
		return nil, false
	}
	role, member := user.RoleInOrg(item.OrgID)
	if user.Superadmin {
		role, member = domain.OrgRoleAdmin, true
	}
	if !member {
		SendErrorResponse(w, http.StatusNotFound, "Task not found", "not-found")
		return nil, false
	}
	if write && !role.CanWrite() {
		SendErrorResponse(w, http.StatusForbidden, "Forbidden", "insufficient-role")
		return nil, false
	}
	return item, true
}

// TaskLinks: `GET /api/v1/tasks/{id}/basecamp`.
func (h *BasecampHandler) TaskLinks(w http.ResponseWriter, r *http.Request) {
	item, ok := h.taskInOrg(w, r, false)
	if !ok {
		return
	}
	out, err := h.svc.TaskLinks(item.ID)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.BasecampLink]{Success: true, Data: out})
}

// LinkTask: `POST /api/v1/tasks/{id}/basecamp` con `{url}`.
func (h *BasecampHandler) LinkTask(w http.ResponseWriter, r *http.Request) {
	item, ok := h.taskInOrg(w, r, true)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.LinkBasecampRecordingRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	l, err := h.svc.LinkTask(item, req.URL)
	if err != nil {
		basecampError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.BasecampLink]{Success: true, Data: l})
}

// UnlinkTask: `DELETE /api/v1/tasks/{id}/basecamp/{recordingId}`.
func (h *BasecampHandler) UnlinkTask(w http.ResponseWriter, r *http.Request) {
	item, ok := h.taskInOrg(w, r, true)
	if !ok {
		return
	}
	if err := h.svc.UnlinkTask(item.ID, int64Param(r, "recordingId")); err != nil {
		basecampError(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}
