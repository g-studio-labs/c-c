package handler

import (
	"errors"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// RequestHandler: los pedidos de los agentes a una persona («Por atender»,
// command-and-control-500/504). Todo pide ser de la organización del pedido;
// quien no lo es recibe 404, como en las tareas. Escribir pide poder escribir
// en ella, y contestar, además, ser la persona a quien se le pidió.
type RequestHandler struct {
	svc *service.RequestService
}

func NewRequestHandler(svc *service.RequestService) *RequestHandler {
	return &RequestHandler{svc: svc}
}

func agentRequestError(w http.ResponseWriter, err error) {
	var colleague *service.ErrNoSuchColleague
	switch {
	case errors.Is(err, repository.ErrRequestNotFound), errors.Is(err, service.ErrRequestTask):
		SendErrorResponse(w, http.StatusNotFound, "Not found", "not-found")
	case errors.Is(err, service.ErrRequestKind):
		SendErrorResponse(w, http.StatusBadRequest, "kind is approve_message, test, decide, run or obtain", "request-kind")
	case errors.Is(err, domain.ErrRequestBody), errors.Is(err, domain.ErrRequestAnswer):
		SendErrorResponse(w, http.StatusBadRequest, err.Error(), "request-invalid")
	case errors.As(err, &colleague):
		SendErrorResponse(w, http.StatusBadRequest, err.Error(), "no-such-colleague")
	case errors.Is(err, service.ErrRequestNotYours):
		SendErrorResponse(w, http.StatusForbidden,
			"This request was made to someone else: only they answer it (or open it in their app).", "request-not-yours")
	case errors.Is(err, repository.ErrRequestNotOpen):
		SendErrorResponse(w, http.StatusConflict,
			"The request is no longer open: it was answered, withdrawn or re-issued meanwhile. Read it again.", "request-not-open")
	case errors.Is(err, service.ErrRequestNotPublishable):
		SendErrorResponse(w, http.StatusConflict, "Only an approved message is published.", "request-not-publishable")
	default:
		SendErrorResponse(w, http.StatusInternalServerError, "Request failed", err.Error())
	}
}

// request: el pedido de la ruta (`req-…` o uuid), si quien llama es de su org.
func (h *RequestHandler) request(w http.ResponseWriter, r *http.Request, needWrite bool) (*domain.Request, *domain.ClaimsJWT, bool) {
	q, err := h.svc.Find(chi.URLParam(r, "id"))
	if err != nil {
		agentRequestError(w, err)
		return nil, nil, false
	}
	user, ok := authorizeOrg(w, r, q.OrgID, needWrite)
	if !ok {
		return nil, nil, false
	}
	return q, user, true
}

// fromSession: la escritura llega de una sesión de agente (el MCP, o un token
// personal), no de un botón de la app.
func fromSession(r *http.Request, user *domain.ClaimsJWT) bool {
	return domain.ViaFrom(r.Context()) == domain.ViaMCP || user.ViaToken
}

func requestVia(r *http.Request, user *domain.ClaimsJWT) string {
	if fromSession(r, user) {
		return domain.ViaMCP
	}
	return domain.ViaApp
}

func sendRequest(w http.ResponseWriter, status int, v *domain.RequestView) {
	SendResult(w, status, domain.APIResponse[*domain.RequestView]{Success: true, Data: v})
}

// Create: POST /api/v1/requests.
func (h *RequestHandler) Create(w http.ResponseWriter, r *http.Request) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return
	}
	req, err := ValidateRequest[domain.CreateRequestRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	task, err := h.svc.ResolveTask(MemberOrgs(user), req.TaskID)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	if _, ok := authorizeOrg(w, r, task.OrgID, true); !ok {
		return
	}
	v, err := h.svc.Create(task, user.UserID, requestVia(r, user), req)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	sendRequest(w, http.StatusCreated, v)
}

// List: GET /api/v1/requests?scope=mine|team&status=open|answered|withdrawn|closed|all&taskId=&orgId=
//
// Por defecto lo pedido a quien mira y abierto, en todas sus organizaciones.
// `closed` junta contestados y retirados, que es «Resueltos» en la bandeja.
func (h *RequestHandler) List(w http.ResponseWriter, r *http.Request) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return
	}
	q := r.URL.Query()
	f := repository.RequestFilter{OrgIDs: MemberOrgs(user)}
	if org := strings.TrimSpace(q.Get("orgId")); org != "" {
		if _, ok := authorizeOrg(w, r, org, false); !ok {
			return
		}
		f.OrgIDs = []string{org}
	}
	switch q.Get("scope") {
	case "", "mine":
		f.RequestedOf = user.UserID
	case "team":
	default:
		SendErrorResponse(w, http.StatusBadRequest, "scope is mine or team", "request-scope")
		return
	}
	switch q.Get("status") {
	case "", "open":
		f.Statuses = []domain.RequestStatus{domain.RequestOpen}
	case "answered":
		f.Statuses = []domain.RequestStatus{domain.RequestAnswered}
	case "withdrawn":
		f.Statuses = []domain.RequestStatus{domain.RequestWithdrawn}
	case "closed":
		f.Statuses = []domain.RequestStatus{domain.RequestAnswered, domain.RequestWithdrawn}
	case "all":
	default:
		SendErrorResponse(w, http.StatusBadRequest, "status is open, answered, withdrawn, closed or all", "request-status")
		return
	}
	if t := strings.TrimSpace(q.Get("taskId")); t != "" {
		task, err := h.svc.ResolveTask(f.OrgIDs, t)
		if err != nil {
			agentRequestError(w, err)
			return
		}
		if _, ok := authorizeOrg(w, r, task.OrgID, false); !ok {
			return
		}
		f.TaskID = task.ID
		f.OrgIDs = []string{task.OrgID}
	}
	if n, err := strconv.Atoi(q.Get("limit")); err == nil {
		f.Limit = n
	}
	out, err := h.svc.List(f, user.UserID)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.RequestView]{Success: true, Data: out})
}

// Get: GET /api/v1/requests/{id}, con historial y el handoff de la tarea.
func (h *RequestHandler) Get(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, false)
	if !ok {
		return
	}
	v := h.svc.View(q, user.UserID, true)
	sendRequest(w, http.StatusOK, &v)
}

// Update: PATCH /api/v1/requests/{id}. Rehace un pedido abierto.
func (h *RequestHandler) Update(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, true)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.UpdateRequestRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	v, err := h.svc.Update(q, user.UserID, requestVia(r, user), req)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	sendRequest(w, http.StatusOK, v)
}

// Answer: POST /api/v1/requests/{id}/answer.
func (h *RequestHandler) Answer(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, true)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.RequestAnswer](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	v, err := h.svc.Answer(q, user.UserID, fromSession(r, user), req)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	sendRequest(w, http.StatusOK, v)
}

// Withdraw: POST /api/v1/requests/{id}/withdraw.
func (h *RequestHandler) Withdraw(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, true)
	if !ok {
		return
	}
	v, err := h.svc.Withdraw(q, user.UserID)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	sendRequest(w, http.StatusOK, v)
}

// Published: POST /api/v1/requests/{id}/published {url}.
func (h *RequestHandler) Published(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, true)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.PublishedRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	v, err := h.svc.Publish(q, user.UserID, req.URL)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	sendRequest(w, http.StatusOK, v)
}

// Open: POST /api/v1/requests/{id}/open. Le pide a la app abierta de la
// persona que lo enseñe; contesta si alguna dijo que lo abrió.
func (h *RequestHandler) Open(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, false)
	if !ok {
		return
	}
	out, err := h.svc.Open(r.Context(), q, user.UserID)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[domain.RequestOpened]{Success: true, Data: out})
}

// Opened: POST /api/v1/requests/{id}/opened {nonce}. La app dice que lo enseñó.
func (h *RequestHandler) Opened(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, false)
	if !ok {
		return
	}
	req, err := ValidateRequest[struct {
		Nonce string `json:"nonce" validate:"required,max=64"`
	}](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	h.svc.Opened(user.UserID, q.OrgID, req.Nonce)
	w.WriteHeader(http.StatusNoContent)
}

// Wait: GET /api/v1/requests/{id}/wait?timeout=<segundos>. Espera larga hasta
// que se conteste o se retire, como mucho unos segundos por petición: quien
// espera más (el MCP) encadena varias.
func (h *RequestHandler) Wait(w http.ResponseWriter, r *http.Request) {
	q, user, ok := h.request(w, r, false)
	if !ok {
		return
	}
	secs, _ := strconv.Atoi(r.URL.Query().Get("timeout"))
	out, err := h.svc.Wait(r.Context(), q, user.UserID, time.Duration(secs)*time.Second)
	if err != nil {
		agentRequestError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[domain.RequestWait]{Success: true, Data: out})
}
