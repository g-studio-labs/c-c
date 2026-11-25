package http

import (
	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/adapters/handler"
	"github.com/guz-studio/cac/backend/internal/adapters/middleware"
)

// mountRequestRoutes: los pedidos de los agentes a una persona (ver
// domain/request.go). Aparte para que las pruebas recorran **este** enrutado.
func mountRequestRoutes(r chi.Router, h *handler.RequestHandler) {
	r.Route("/api/v1/requests", func(r chi.Router) {
		r.Use(middleware.AuthMiddleware)
		r.Get("/", h.List)
		r.Post("/", h.Create)
		r.Get("/{id}", h.Get)
		r.Patch("/{id}", h.Update)
		r.Post("/{id}/answer", h.Answer)
		r.Post("/{id}/withdraw", h.Withdraw)
		r.Post("/{id}/published", h.Published)
		r.Post("/{id}/open", h.Open)
		r.Post("/{id}/opened", h.Opened)
		r.Get("/{id}/wait", h.Wait)
	})
}
