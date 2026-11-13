package handler

import (
	"errors"
	"net/http"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

type GraphHandler struct{ svc *service.GraphService }

func NewGraphHandler(svc *service.GraphService) *GraphHandler { return &GraphHandler{svc: svc} }

// Similar: GET /api/v1/graph/{kind}/{id}/similar. Las decisiones o los
// incidentes vigentes de la misma org con un título parecido: lo que el MCP
// le enseña al agente justo después de apuntar uno, para que sustituya en vez
// de duplicar (command-and-control-351).
func (h *GraphHandler) Similar(w http.ResponseWriter, r *http.Request) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return
	}
	out, err := h.svc.Similar(domain.EntityKind(chi.URLParam(r, "kind")), chi.URLParam(r, "id"), MemberOrgs(user), user.Superadmin)
	if errors.Is(err, service.ErrGraphNotFound) {
		SendErrorResponse(w, http.StatusNotFound, "No decision or incident with that id in your organizations", "not-found")
		return
	}
	if err != nil {
		SendErrorResponse(w, http.StatusInternalServerError, "Failed to look for similar entries", err.Error())
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.SimilarEntry]{Success: true, Data: out})
}

// Related: GET /api/v1/graph/{kind}/{id}. Todo lo que toca a una cosa —una
// tarea (también por su ref), una decisión, un incidente, una página, una
// pestaña, una sección o un servidor—, en los dos sentidos, con título,
// estado y ref. Sólo de una org de la que se es miembro. Con `rule` como tipo,
// la sección que es esa regla (`/graph/rule/R-012`).
func (h *GraphHandler) Related(w http.ResponseWriter, r *http.Request) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return
	}
	out, err := h.svc.Related(domain.EntityKind(chi.URLParam(r, "kind")), chi.URLParam(r, "id"), MemberOrgs(user), user.Superadmin)
	if errors.Is(err, service.ErrGraphNotFound) {
		SendErrorResponse(w, http.StatusNotFound, "Nothing of that kind with that id in your organizations", "not-found")
		return
	}
	if errors.Is(err, service.ErrGraphSeveral) {
		SendErrorResponse(w, http.StatusConflict,
			"More than one rules registry in your organizations has that rule; ask by the section's id instead", "several-rules")
		return
	}
	if err != nil {
		SendErrorResponse(w, http.StatusInternalServerError, "Failed to read what is related", err.Error())
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.Related]{Success: true, Data: out})
}
