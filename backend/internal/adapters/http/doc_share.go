package http

import (
	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/adapters/handler"
	"github.com/guz-studio/cac/backend/internal/adapters/middleware"
)

// mountDocShareMemberRoutes: las de miembro, dentro del grupo de
// `/api/v1/docs` (con su JWT), junto a las páginas.
func mountDocShareMemberRoutes(r chi.Router, h handler.DocShareHandler) {
	r.Post("/{kind}/{ownerId}/pages/{pageId}/shares", h.Create)
	r.Get("/{kind}/{ownerId}/pages/{pageId}/shares", h.List)
}

// mountDocShareRoutes: revocar y la puerta pública. Aparte para que la prueba
// de las puertas recorra **este** enrutado y no una copia.
func mountDocShareRoutes(r chi.Router, h handler.DocShareHandler) {
	r.With(middleware.AuthMiddleware).Delete("/api/v1/doc-shares/{id}", h.Revoke)
	// Todos los enlaces vivos de la organización (command-and-control-577).
	r.With(middleware.AuthMiddleware).Get("/api/v1/organizations/{orgId}/doc-shares", h.ListForOrg)

	// La puerta de quien no tiene cuenta. **Sin JWT, a propósito, y sólo estas
	// dos**: las abre el token de `X-Doc-Share`, y sólo para leer.
	r.Get("/api/v1/shared-docs", h.PublicDoc)
	r.Get("/api/v1/shared-docs/attachments/{attachmentId}", h.PublicAttachment)
}
