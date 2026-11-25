package handler

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// Por la API, de punta a punta: un handoff y un comentario escritos con un
// token personal salen firmados como de un agente de la persona, con el
// `agent` que mandó; escritos desde la app, como la persona aunque manden
// `agent` (command-and-control-522).
//
// Roturas que atrapa: que el handler no pase la firma al servicio (el PATCH o
// el comentario de un agente salen como de la persona), o que la decida por
// lo que manda el cliente y no por el token (la app firma como un agente).
func TestATokenWritesAsAnAgentAndTheAppAsThePerson(t *testing.T) {
	db, cleanup := docOrgDB(t)
	defer cleanup()
	if err := db.AutoMigrate(&domain.User{}, &domain.TaskList{}, &domain.TaskFolder{}, &domain.Item{}, &domain.ItemAssignee{},
		&domain.ItemBlock{}, &domain.ItemWatcher{}, &domain.ItemComment{}, &domain.ItemAttachment{}, &domain.TaskTag{}, &domain.TaskTagLink{},
		&domain.ReportProject{}, &domain.SpaceKeyAlias{}, &domain.Decision{}, &domain.DecisionLink{}, &domain.DocSection{},
		&domain.TaskGitLink{}, &domain.GitHubRepo{}, &domain.GraphLink{}, &domain.TaskExternalKey{}); err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	db.Exec(`INSERT INTO task_lists (id, space_id, name, rank, created_at, updated_at) VALUES ('l-1','esp-1','Lista','0.5',?,?)`, now, now)
	jose := &domain.User{Username: "jose", Name: "Jose Guzman"}
	jose.ID = "u-jose"
	if err := db.Create(jose).Error; err != nil {
		t.Fatal(err)
	}
	tasks := repository.NewTaskRepository(db)
	task := &domain.Task{OrgID: "org-1", ListID: "l-1", Title: "La tarea", Status: domain.ReportPending}
	if err := tasks.CreateTask(task, "esp-1"); err != nil {
		t.Fatal(err)
	}
	svc := service.NewTaskService(tasks, repository.NewReportRepository(db), repository.NewOrganizationRepository(db), nil)
	h := &taskHandler{svc: svc}
	r := chi.NewRouter()
	r.Patch("/{id}", h.UpdateTask)
	r.Post("/{id}/comments", h.AddComment)
	send := func(method, path, body string, viaToken bool) {
		t.Helper()
		req := httptest.NewRequest(method, path, strings.NewReader(body))
		req.Header.Set("Content-Type", "application/json")
		claims := &domain.ClaimsJWT{UserID: "u-jose", Superadmin: true, ViaToken: viaToken}
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, req.WithContext(context.WithValue(req.Context(), repository.UserContextKey, claims)))
		if rec.Code != http.StatusOK && rec.Code != http.StatusCreated {
			t.Fatalf("%s %s → %d %s", method, path, rec.Code, rec.Body.String())
		}
	}
	const agent = `"agent":{"role":"builder","model":"sonnet"}`
	want := domain.AgentSignature{Of: "Jose Guzman", Role: "builder", Model: "sonnet"}

	send(http.MethodPatch, "/"+task.ID, `{"handoff":"Falta la release.",`+agent+`}`, true)
	d, err := svc.Detail(task.ID)
	if err != nil {
		t.Fatal(err)
	}
	if d.HandoffAgent == nil || *d.HandoffAgent != want {
		t.Fatalf("el handoff escrito con token: %+v", d.HandoffAgent)
	}
	send(http.MethodPatch, "/"+task.ID, `{"handoff":"Ya salió.",`+agent+`}`, false)
	if d, _ = svc.Detail(task.ID); d.HandoffAgent != nil || d.HandoffBy != "Jose Guzman" {
		t.Fatalf("el handoff escrito desde la app salió firmado por un agente: %+v", d.HandoffAgent)
	}

	send(http.MethodPost, "/"+task.ID+"/comments", `{"body":"del agente",`+agent+`}`, true)
	send(http.MethodPost, "/"+task.ID+"/comments", `{"body":"de la persona",`+agent+`}`, false)
	d, _ = svc.Detail(task.ID)
	signed := map[string]*domain.AgentSignature{}
	for _, c := range d.Comments {
		signed[c.Body] = c.Agent
	}
	if a := signed["del agente"]; a == nil || *a != want {
		t.Fatalf("el comentario escrito con token: %+v", a)
	}
	if a, ok := signed["de la persona"]; !ok || a != nil {
		t.Fatalf("el comentario escrito desde la app salió firmado por un agente: %+v (%v)", a, ok)
	}
}
