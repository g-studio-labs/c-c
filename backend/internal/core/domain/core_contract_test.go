package domain

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// El contrato con el core en Rust (`core/`), que lee estas respuestas con
// tipos escritos a mano: no hay OpenAPI que los genere.
//
// Esta prueba serializa un ejemplo **con todos los campos puestos** de cada
// forma que el core lee y lo compara con el fichero que hay en
// `core/tests/fixtures/`. La prueba `contract` del core lee esos mismos
// ficheros con sus tipos. Así un campo que aquí se renombra, se quita o se
// añade rompe una de las dos, en vez de romper la app de alguien:
//
//   - renombrar o quitar: este fichero ya no coincide, y falla aquí;
//   - tras regenerarlo, el core no encuentra el campo que esperaba, o
//     encuentra uno que no conoce, y falla allí.
//
// Para regenerar a propósito: `UPDATE_CORE_FIXTURES=1 go test ./internal/core/domain -run TestTheCoreReadsWhatTheServerWrites`.
func TestTheCoreReadsWhatTheServerWrites(t *testing.T) {
	day := time.Date(2026, 9, 30, 0, 0, 0, 0, time.UTC)
	fixtures := map[string]any{
		"open_task.json": OpenTask{
			ID:            "0d6bf322-2267-4259-9b81-337575aca792",
			Seq:           12,
			Title:         "Subir el backend",
			Priority:      ItemPriorityMedium,
			DueAt:         &day,
			Status:        ReportInProgress,
			StatusName:    "In progress",
			StatusKind:    StatusKindOpen,
			ListID:        "ca0bfd49-0909-43eb-8135-bc8ecd0f282c",
			ListName:      "tasks",
			SpaceID:       "e8bd6d46-a9ab-494e-ac29-de84d80a55f4",
			SpaceName:     "Command and control",
			UpdatedAt:     time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC),
			SubtasksDone:  1,
			SubtasksTotal: 3,
			ParentSeq:     7,
			ParentTitle:   "La tarea madre",
			Assignee:      "ana",
			Folio:         "ACME-7",
			Flow:          FlowClient,
		},
		"auth_response.json": AuthResponse{
			AccessToken:  "access",
			RefreshToken: "refresh",
			ExpiresIn:    900,
			Session: Session{
				ID:                 "u1",
				Username:           "ana",
				Name:               "Ana Pérez",
				Email:              "ana@example.com",
				Superadmin:         true,
				MustChangePassword: true,
				Scopes:             []string{"tasks:write"},
				Locale:             "es",
			},
		},
		"auth_refresh_response.json": AuthRefreshResponse{
			AccessToken:  "access",
			RefreshToken: "refresh",
		},
		"organization.json": OrganizationResponse{
			ID:                       "35d32154-922f-47dc-a6d9-051d4e78ae8a",
			Name:                     "G Studio",
			Slug:                     "g-studio",
			Role:                     OrgRoleAdmin,
			MemberCount:              4,
			CreatedAt:                time.Date(2026, 6, 9, 12, 0, 0, 0, time.UTC),
			Domain:                   "guz-studio.dev",
			DefaultInviteRole:        OrgRoleMember,
			ClientsSeeOnlyTheirSpace: true,
			GuestsCanUseDevTools:     true,
			DoneNeedsSubtasksDone:    true,
			Timezone:                 "America/Mexico_City",
		},
		"space_tree.json": SpaceTree{
			ID:        "e8bd6d46-a9ab-494e-ac29-de84d80a55f4",
			Key:       "command-and-control",
			OrgID:     "35d32154-922f-47dc-a6d9-051d4e78ae8a",
			Name:      "Command and control",
			Color:     "#20D9E8",
			ProjectID: "p1",
			Kind:      "general",
			Folders: []FolderTree{{
				ID:      "4d087751-3496-4a0d-a335-56212a84a54d",
				Name:    "App",
				Folders: []FolderTree{{ID: "f2", Name: "Dentro", Folders: []FolderTree{}, Lists: []ListSummary{}}},
				Lists: []ListSummary{{
					ID: "ca0bfd49-0909-43eb-8135-bc8ecd0f282c", Name: "tasks",
					OpenCount: 6, ProjectID: "p2", TaskCount: 40,
				}},
			}},
			Lists:  []ListSummary{{ID: "l0", Name: "sueltas", OpenCount: 1, ProjectID: "p3", TaskCount: 2}},
			People: []SpacePerson{{UserID: "u1", Username: "ana", Name: "Ana Pérez"}},
		},
		"doc_mark.json": DocMark{
			Written: true, PinnedLine: "Desplegar: backend antes que app", Stale: true,
			MaintainerID: "u1", MaintainerName: "Ana Pérez", ReviewedAt: &day, Pages: 3,
		},
		"task_detail.json":    fullTaskDetail(day),
		"state_machines.json": stateMachines(),
	}

	dir := filepath.Join("..", "..", "..", "..", "core", "tests", "fixtures")
	for name, value := range fixtures {
		got, err := json.MarshalIndent(value, "", "  ")
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		got = append(got, '\n')
		file := filepath.Join(dir, name)

		if os.Getenv("UPDATE_CORE_FIXTURES") != "" {
			if err := os.WriteFile(file, got, 0o644); err != nil {
				t.Fatalf("%s: %v", name, err)
			}
			continue
		}

		want, err := os.ReadFile(file)
		if err != nil {
			t.Fatalf("%s: falta el fichero que lee el core: %v", name, err)
		}
		if !bytes.Equal(got, want) {
			t.Errorf("%s ya no es lo que el servidor escribe.\n"+
				"El core en Rust lee esta forma con tipos a mano: cambia también `core/src`, "+
				"regenera con UPDATE_CORE_FIXTURES=1 y pasa `cargo test` en `core/`.\n"+
				"ahora:\n%s\nfichero:\n%s", name, got, want)
		}
	}
}

// fullTaskDetail es el detalle de una tarea con **todos** los campos puestos,
// también los de dentro: un campo con su valor cero y `omitempty` no saldría
// en el fichero, y el core no se enteraría de que existe.
func fullTaskDetail(day time.Time) TaskDetail {
	at := time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC)
	str := func(s string) *string { return &s }
	base := func(id string) BaseModel { return BaseModel{ID: id, CreatedAt: at, UpdatedAt: at} }
	tag := TaskTag{BaseModel: base("tag1"), OrgID: "o1", Name: "urgente", Color: "#f00"}
	ana := UserSummary{ID: "u1", Username: "ana", Name: "Ana Pérez"}
	file := TaskAttachment{
		BaseModel: base("a1"), ItemID: "t1", CommentID: str("c1"),
		URL: "/api/v1/tasks/t1/attachments/a1/raw", FileName: "captura.png",
		ContentType: "image/png", Bytes: 2048, UploadedBy: "u1",
	}
	card := TaskCard{
		ID: "t2", Seq: 13, Title: "Una línea del checklist", Priority: "normal", StatusID: "s1",
		DueAt: &day, HasDescription: true, CommentCount: 2, AttachmentCount: 1,
		SubtaskCount: 3, SubtaskDone: 1, Tags: []TaskTag{tag}, Assignees: []UserSummary{ana},
		UpdatedAt: at, Category: "bug", Area: "login", CreatedAt: at, Flow: FlowInternal,
		Git:  &GitSummary{Branches: 1, PRs: 2, Commits: 3, PRBadge: PRStateOpen},
		Keys: []string{"PROTEUS-165"},
	}
	link := TaskGitLink{
		BaseModel: base("g1"), ItemID: "t1", RepoID: 7, RepoFullName: "g-studio-labs/c-c",
		Kind: "commit", Key: "0fbe389", Title: "feat(core): …", HTMLURL: "https://github.com/x/y/commit/0fbe389",
		AuthorLogin: "ana", State: "open", HeadBranch: "rama", BaseBranch: "main", HeadSha: "0fbe389",
		Via: "text", MergedAt: &at, OccurredAt: at,
		ReleasedIn: &ReleaseRef{Tag: "v1.33.0", At: at, URL: "https://github.com/x/y/releases/v1.33.0"},
	}
	return TaskDetail{
		Task: Task{
			BaseModel: base("t1"), OrgID: "o1", ListID: "l1", SpaceID: "sp1", ProjectID: "p1", Seq: 12,
			Title: "Subir el backend", Description: "Con **markdown**.", Status: ReportInProgress,
			Category: CategoryBug, Priority: "normal", Area: "login", Origin: "user",
			URL: "https://app.example.com/login", UserAgent: "Mozilla/5.0", Viewport: "1280x720",
			TelemetryPurgeAt: &at, ReporterName: "Ana", ReporterEmail: "ana@example.com", ReporterID: "ext-1",
			AssigneeUserID: str("u1"), Visibility: VisibilityPublic, ParentID: str("t0"),
			StartAt: &at, DueAt: &day, ResolvedAt: &at, Handoff: "Dónde quedó.", HandoffAt: &at,
			HandoffByID: "u1", CreatedByID: "u1", ArchivedAt: &at,
		},
		ListName:  "tasks",
		SpaceName: "Command and control",
		Status: TaskStatus{
			BaseModel: base("s1"), ListID: "l1", Name: "In progress", Color: "#20D9E8",
			Kind: StatusKindActive, Status: ReportInProgress,
		},
		Tags:      []TaskTag{tag},
		Assignees: []UserSummary{ana},
		Comments: []TaskCommentResponse{{
			ID: "c1",
			Author: &CommentAuthor{
				Kind: "team", Name: "Ana Pérez", UserID: "u1",
				ProjectID: "p1", ProjectName: "Acme", ExternalID: "ext-1",
			},
			AuthorUserID: "u1", AuthorName: "Ana Pérez", Visibility: VisibilityInternal,
			Kind: "user", Source: "gh", Via: "mcp", Body: "Una nota.",
			Attachments: []TaskAttachment{file}, CreatedAt: at, UpdatedAt: at,
		}},
		Attachments: []TaskAttachment{file},
		Subtasks:    []TaskCard{card},
		Parent:      &card,
		Folio:       "acme-7",
		ProjectSlug: "acme",
		Ref:         "acme-7",
		Keys:        []string{"PROTEUS-165"},
		Tickets:     []string{"PROTEUS-165"},
		HandoffBy:   "Ana Pérez",
		BlockedBy:   []TaskBlocker{{ID: "t9", Ref: "command-and-control-9", Title: "La de antes", Status: ReportPending, Open: true}},
		Decisions: []DecisionRef{{
			ID: "d1", Title: "GPUI Kit", OwnerKind: "space", OwnerID: "sp1",
			DecidedAt: at, SupersededByID: "d2",
		}},
		Telemetry: json.RawMessage(`{"breadcrumbs":[]}`),
		Git: &TaskGitLinks{
			Summary:  GitSummary{Branches: 1, PRs: 1, Commits: 1, PRBadge: PRStateOpen},
			Branches: []TaskGitLink{link}, PRs: []TaskGitLink{link}, Commits: []TaskGitLink{link},
		},
	}
}

// stateMachines son las dos máquinas de estados tal como las decide el
// servidor, par por par, más a quién le toca cada una y los nombres que se
// aceptan. El core las repite a mano (`core/src/task/flow.rs`) para no ofrecer
// un cambio que va a contestar 409; aquí se le da con qué comprobarse.
func stateMachines() map[string]any {
	statuses := []ReportStatus{ReportPending, ReportInProgress, ReportResolved, ReportClosed, "open", "done"}
	type pair struct {
		Flow    ItemFlow     `json:"flow"`
		From    ReportStatus `json:"from"`
		To      ReportStatus `json:"to"`
		Allowed bool         `json:"allowed"`
	}
	pairs := []pair{}
	for _, f := range []ItemFlow{FlowClient, FlowInternal} {
		for _, from := range statuses {
			for _, to := range statuses {
				pairs = append(pairs, pair{f, from, to, CanTransition(f, from, to)})
			}
		}
	}
	type owner struct {
		ProjectID  string         `json:"projectId"`
		Visibility ItemVisibility `json:"visibility"`
		Flow       ItemFlow       `json:"flow"`
	}
	owners := []owner{}
	for _, projectID := range []string{"", "p1"} {
		for _, v := range []ItemVisibility{"", VisibilityPublic, VisibilityInternal} {
			owners = append(owners, owner{projectID, v, FlowFor(projectID, v)})
		}
	}
	type priority struct {
		Stored ItemPriority `json:"stored"`
		Wire   ItemPriority `json:"wire"`
	}
	priorities := []priority{}
	for _, p := range ItemPriorities() {
		priorities = append(priorities, priority{p, p.TaskWire()})
	}
	return map[string]any{"transitions": pairs, "flows": owners, "priorities": priorities}
}
