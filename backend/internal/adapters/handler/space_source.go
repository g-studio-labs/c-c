package handler

import (
	"errors"
	"net/http"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// SpaceSourceHandler: las fuentes de un espacio (command-and-control-354).
type SpaceSourceHandler struct {
	svc    *service.SpaceSourceService
	spaces *repository.TaskRepository
}

func NewSpaceSourceHandler(svc *service.SpaceSourceService, spaces *repository.TaskRepository) *SpaceSourceHandler {
	return &SpaceSourceHandler{svc: svc, spaces: spaces}
}

// space: el espacio de la ruta, sólo para quien es de su organización (404 a
// quien no: ver authorizeOrg).
func (h *SpaceSourceHandler) space(w http.ResponseWriter, r *http.Request, needWrite bool) (*domain.TaskSpace, bool) {
	sp, err := h.spaces.FindSpace(chi.URLParam(r, "id"))
	if err != nil {
		SendErrorResponse(w, http.StatusNotFound, "Not found", "not-found")
		return nil, false
	}
	if _, ok := authorizeOrg(w, r, sp.OrgID, needWrite); !ok {
		return nil, false
	}
	return sp, true
}

// sourceErrors: lo que una fuente mal escrita contesta, con qué arreglar.
var sourceErrors = map[error]struct {
	status int
	msg    string
}{
	domain.ErrSourceRole:       {http.StatusBadRequest, "A source's role is one of: tracker, rules, rules-copy, decisions, open-questions"},
	domain.ErrSourceSystem:     {http.StatusBadRequest, "A source's system is one of: jira, notion, cac, other"},
	domain.ErrSourceRef:        {http.StatusBadRequest, "A source needs a ref: the Jira project key, the page URL, or the id of the cac page"},
	domain.ErrSourcePattern:    {http.StatusBadRequest, "A rules source needs a pattern: a regular expression of at most 40 characters that matches a whole rule id, such as R-\\d+"},
	domain.ErrSourceRulesInCac: {http.StatusBadRequest, "The rules registry lives in cac: system cac, and ref the id of its page"},
	domain.ErrSourceTooMany:    {http.StatusBadRequest, "A space holds at most 20 sources"},
	domain.ErrSourceRepeated:   {http.StatusBadRequest, "The same source (role, system and ref) is listed twice"},
	domain.ErrSourcePage:       {http.StatusBadRequest, "No documentation page with that id in this organization"},
	domain.ErrSourceNotFound:   {http.StatusNotFound, "No source with that id in this space"},
	domain.ErrSourceSyncFuture: {http.StatusBadRequest, "A source cannot be read up to a moment that has not happened yet"},
}

func sendSourceError(w http.ResponseWriter, err error, fallback string) {
	for known, e := range sourceErrors {
		if errors.Is(err, known) {
			SendErrorResponse(w, e.status, e.msg, known.Error())
			return
		}
	}
	SendErrorResponse(w, http.StatusInternalServerError, fallback, err.Error())
}

// List: GET /api/v1/task-spaces/{id}/sources.
func (h *SpaceSourceHandler) List(w http.ResponseWriter, r *http.Request) {
	sp, ok := h.space(w, r, false)
	if !ok {
		return
	}
	out, err := h.svc.List(sp.ID)
	if err != nil {
		SendErrorResponse(w, http.StatusInternalServerError, "Failed to read the space's sources", err.Error())
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.SpaceSource]{Success: true, Data: out})
}

// Replace: PUT /api/v1/task-spaces/{id}/sources. La lista entera.
func (h *SpaceSourceHandler) Replace(w http.ResponseWriter, r *http.Request) {
	sp, ok := h.space(w, r, true)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.SpaceSourcesRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	out, err := h.svc.Replace(sp, req)
	if err != nil {
		sendSourceError(w, err, "Failed to save the space's sources")
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.SpaceSource]{Success: true, Data: out})
}

// Synced: POST /api/v1/task-spaces/{id}/sources/{sourceId}/synced. Mueve el
// cursor de una fuente: lo llama la puesta al día cuando terminó de leerla.
func (h *SpaceSourceHandler) Synced(w http.ResponseWriter, r *http.Request) {
	sp, ok := h.space(w, r, true)
	if !ok {
		return
	}
	req, err := ValidateRequest[domain.SourceSyncedRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	out, err := h.svc.MarkSynced(sp.ID, chi.URLParam(r, "sourceId"), req.At)
	if err != nil {
		sendSourceError(w, err, "Failed to move the source's cursor")
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.SpaceSource]{Success: true, Data: out})
}
