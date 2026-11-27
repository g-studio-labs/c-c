package http

import (
	"context"

	"github.com/go-chi/chi/v5"
	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/adapters/handler"
	"github.com/guz-studio/cac/backend/internal/adapters/middleware"
	"github.com/guz-studio/cac/backend/internal/core/events"
	lg "github.com/guz-studio/cac/backend/internal/core/logger"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// BasecampConfigFromEnv: la app de 37signals de este servidor. Sin
// BASECAMP_CLIENT_ID y BASECAMP_CLIENT_SECRET, conectar contesta 503
// «basecamp-off» y lo demás arranca igual.
func BasecampConfigFromEnv() service.BasecampConfig {
	return service.BasecampConfig{
		ClientID:     repository.GetEnv("BASECAMP_CLIENT_ID", ""),
		ClientSecret: repository.GetEnv("BASECAMP_CLIENT_SECRET", ""),
		RedirectURI:  repository.GetEnv("BASECAMP_REDIRECT_URI", ""),
	}
}

// NewBasecampServiceFor: el servicio, antes que las rutas de tareas, que lo
// usan para enlazar una tarea con lo que cita su descripción.
func NewBasecampServiceFor(db *gorm.DB, hub *events.Hub, cfg service.BasecampConfig) *service.BasecampService {
	inbox := service.NewNotificationService(repository.NewNotificationRepository(db))
	requests := service.NewRequestService(repository.NewRequestRepository(db), repository.NewTaskRepository(db),
		repository.NewDocRepository(db), hub, inbox)
	svc := service.NewBasecampService(repository.NewBasecampRepository(db), cfg, hub, inbox, requests)
	if !svc.Configured() {
		lg.Warn("basecamp: BASECAMP_CLIENT_ID / BASECAMP_CLIENT_SECRET not set — connecting Basecamp answers basecamp-off")
	}
	return svc
}

func InitBasecampRoutesWith(r *chi.Mux, svc *service.BasecampService) {
	h := handler.NewBasecampHandler(svc)

	// Fuera del JWT: al setup vuelve el navegador desde Launchpad con el
	// `state` firmado; al webhook llama Basecamp, con el secreto en la ruta.
	r.Get("/webhooks/basecamp/setup", h.Setup)
	r.Post("/webhooks/basecamp/{secret}", h.Webhook)

	r.Route("/api/v1/organizations/{id}/basecamp", func(r chi.Router) {
		r.Use(middleware.AuthMiddleware)
		r.Get("/", h.Status)
		r.Delete("/", h.Disconnect)
		r.Post("/connect", h.Connect)
		r.Get("/projects", h.AvailableProjects)
		r.Put("/projects/{projectId}", h.LinkProject)
		r.Delete("/projects/{projectId}", h.UnlinkProject)
		r.Get("/awaiting", h.Awaiting)
		r.Get("/people", h.People)
		r.Put("/people/{personId}", h.MatchPerson)
	})
	r.Route("/api/v1/tasks/{id}/basecamp", func(r chi.Router) {
		r.Use(middleware.AuthMiddleware)
		r.Get("/", h.TaskLinks)
		r.Post("/", h.LinkTask)
		r.Delete("/{recordingId}", h.UnlinkTask)
	})
}

// startBasecampRefresher: renueva los tokens antes de que caduquen.
func startBasecampRefresher(svc *service.BasecampService) {
	svc.Start(context.Background())
}
