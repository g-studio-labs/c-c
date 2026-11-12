package handler

import (
	"encoding/json"
	"net/http"
	"strconv"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// AgentUsageStore: lo que el handler necesita del repositorio.
type AgentUsageStore interface {
	RecordShown(userID string, req domain.AgentShownRequest) error
	Usage(userID string, since time.Time) (domain.AgentUsage, error)
}

type AgentUsageHandler struct{ store AgentUsageStore }

func NewAgentUsageHandler(store AgentUsageStore) *AgentUsageHandler {
	return &AgentUsageHandler{store: store}
}

// Shown: POST /api/v1/agents/shown. Un hook dice qué le puso delante al
// agente de quien tiene el token; se cruza después con lo que ese agente
// abrió (ver domain.AgentCall). Cualquier token puede: sólo habla de sí.
func (h *AgentUsageHandler) Shown(w http.ResponseWriter, r *http.Request) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return
	}
	var req domain.AgentShownRequest
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 8<<10)).Decode(&req); err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid body", err.Error())
		return
	}
	if domain.NormalizeAgentTool(req.Hook) == "" {
		SendErrorResponse(w, http.StatusBadRequest, "hook must be a hook's name, like cac-prompt", "invalid-hook")
		return
	}
	if err := h.store.RecordShown(user.UserID, req); err != nil {
		SendErrorResponse(w, http.StatusInternalServerError, "Failed to record what was shown", err.Error())
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[map[string]bool]{Success: true, Data: map[string]bool{"recorded": true}})
}

// Usage: GET /api/v1/agents/usage?days=. Qué herramientas usan los agentes y
// cuánto de lo que enseñan los hooks se abre después. Cada quien ve lo suyo;
// un superadmin, lo de todos (con `all=1`).
func (h *AgentUsageHandler) Usage(w http.ResponseWriter, r *http.Request) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return
	}
	days, _ := strconv.Atoi(r.URL.Query().Get("days"))
	if days < 1 || days > 90 {
		days = 30
	}
	who := user.UserID
	if user.Superadmin && r.URL.Query().Get("all") == "1" {
		who = ""
	}
	out, err := h.store.Usage(who, time.Now().AddDate(0, 0, -days))
	if err != nil {
		SendErrorResponse(w, http.StatusInternalServerError, "Failed to read the agents' usage", err.Error())
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[domain.AgentUsage]{Success: true, Data: out})
}
