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

// Related: GET /api/v1/graph/{kind}/{id}. Todo lo que toca a una cosa —una
// tarea (también por su ref), una decisión, un incidente, una página, una
// pestaña, una sección o un servidor—, en los dos sentidos, con título,
// estado y ref. Sólo de una org de la que se es miembro.
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
	if err != nil {
		SendErrorResponse(w, http.StatusInternalServerError, "Failed to read what is related", err.Error())
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.Related]{Success: true, Data: out})
}
