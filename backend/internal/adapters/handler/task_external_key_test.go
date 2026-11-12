package handler

import (
	"context"
	"encoding/json"
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

// `GET /tasks/by-ref/PROTEUS-165` contesta la tarea que nombra esa clave de
// fuera, y `…/proteus-165` la tarea de cac con ese número: numeran aparte y
// no se cruzan (command-and-control-353).
//
// Roturas que atrapa: pasar la clave a minúsculas y contestar la tarea de ese
// número; dar por ambigua una clave porque otra tarea la cita de pasada;
// contestar una cualquiera cuando varias son ese ticket; contestar la de
// una organización de la que no se es miembro; un 500 o un 200 vacío cuando
// nadie la nombra.
func TestATrackerKeyAndARefAreDifferentNames(t *testing.T) {
	db, cleanup := docOrgDB(t)
	defer cleanup()
	if err := db.AutoMigrate(&domain.User{}, &domain.TaskList{}, &domain.TaskFolder{}, &domain.Item{}, &domain.ItemAssignee{},
		&domain.ItemBlock{}, &domain.ItemComment{}, &domain.ItemAttachment{}, &domain.TaskTag{}, &domain.TaskTagLink{},
		&domain.ReportProject{}, &domain.SpaceKeyAlias{}, &domain.Decision{}, &domain.DecisionLink{}, &domain.DocSection{},
		&domain.TaskGitLink{}, &domain.GitHubRepo{}, &domain.GraphLink{}, &domain.TaskExternalKey{}); err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	db.Exec(`UPDATE task_spaces SET key = 'proteus' WHERE id = 'esp-1'`)
	db.Exec(`UPDATE task_spaces SET key = 'otro' WHERE id = 'esp-2'`)
	db.Exec(`INSERT INTO task_lists (id, space_id, name, rank, created_at, updated_at)
		VALUES ('l-1','esp-1','Lista','0.5',?,?), ('l-2','esp-2','Ajena','0.5',?,?)`, now, now, now, now)
	tasks := repository.NewTaskRepository(db)
	create := func(org, list, space, title, description string) *domain.Task {
		it := &domain.Task{OrgID: org, ListID: list, Title: title, Description: description, Status: domain.ReportPending}
		if err := tasks.CreateTask(it, space); err != nil {
			t.Fatal(err)
		}
		return it
	}
	var numbered *domain.Task
	for numbered == nil || numbered.Seq < 3 { // proteus-3: la tarea de cac con ese número
		numbered = create("org-1", "l-1", "esp-1", "La tarea número tres", "")
	}
	named := create("org-1", "l-1", "esp-1", "El cierre cuadra", "**Jira:** PROTEUS-3")
	create("org-1", "l-1", "esp-1", "Una", "**Jira:** PROTEUS-9")
	create("org-1", "l-1", "esp-1", "Otra", "**Jira:** PROTEUS-9")
	// Nombrar un ticket no es ser su tarea: la que lo cita de pasada no
	// compite con la que lo lleva en su línea **Jira:**.
	create("org-1", "l-1", "esp-1", "La que lo cita", "Como en PROTEUS-3, pero con otra cosa. **Referencias:** PROTEUS-3")
	create("org-2", "l-2", "esp-2", "De otra organización", "PROTEUS-77")

	h := &taskHandler{svc: service.NewTaskService(tasks, nil, nil, nil)}
	r := chi.NewRouter()
	r.Get("/by-ref/{ref}", h.GetTaskByRef)
	ask := func(ref string) (int, string) {
		req := httptest.NewRequest(http.MethodGet, "/by-ref/"+ref, nil)
		claims := &domain.ClaimsJWT{Orgs: []domain.OrgMembershipClaim{{OrgID: "org-1"}}}
		rec := httptest.NewRecorder()
		r.ServeHTTP(rec, req.WithContext(context.WithValue(req.Context(), repository.UserContextKey, claims)))
		return rec.Code, rec.Body.String()
	}
	id := func(body string) string {
		var res struct {
			Data struct {
				Task struct{ ID string } `json:"task"`
			} `json:"data"`
		}
		_ = json.Unmarshal([]byte(body), &res)
		return res.Data.Task.ID
	}

	if code, body := ask("PROTEUS-3"); code != http.StatusOK || id(body) != named.ID {
		t.Errorf("PROTEUS-3 → %d, tarea %q; quiero la que lo nombra (%s), no la número tres (%s)", code, id(body), named.ID, numbered.ID)
	}
	if code, body := ask("proteus-3"); code != http.StatusOK || id(body) != numbered.ID {
		t.Errorf("proteus-3 → %d, tarea %q; quiero la número tres (%s)", code, id(body), numbered.ID)
	}
	if code, body := ask("PROTEUS-9"); code != http.StatusConflict || !strings.Contains(body, "Una") || !strings.Contains(body, "Otra") {
		t.Errorf("PROTEUS-9 lo nombran dos tareas → %d %s; quiero un 409 que diga cuáles", code, body)
	}
	if code, body := ask("PROTEUS-77"); code != http.StatusNotFound {
		t.Errorf("PROTEUS-77 sólo lo nombra otra organización → %d %s; quiero 404", code, body)
	}
	if code, _ := ask("PROTEUS-500"); code != http.StatusNotFound {
		t.Errorf("PROTEUS-500 no lo nombra nadie → %d; quiero 404", code)
	}
}
