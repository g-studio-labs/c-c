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
	pageParent, pageID := "p1", "p2"
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
		// La documentación de un nodo llega como un sobre (`doc`, `tabs`,
		// `decisions`) que arma el handler; lo que el core lee de dentro son
		// estas dos formas.
		"doc_tab.json": DocTab{
			BaseModel: BaseModel{
				ID:        "t1",
				CreatedAt: time.Date(2026, 6, 9, 12, 0, 0, 0, time.UTC),
				UpdatedAt: time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC),
			},
			DocID:         "d1",
			Key:           DocRunbook,
			Body:          "## Desplegar\n\nEl backend antes que la app.",
			BodyHash:      "abc123",
			UpdatedBy:     "u1",
			UpdatedByName: "Ana Pérez",
			// Con una decisión que la sostiene: es lo que el core pinta
			// junto al título de la sección.
			Sections: []DocSectionView{{
				ID: "s1", Heading: "Desplegar", Level: 2, Index: 0,
				Decisions: []DecisionRef{{
					ID: "dec1", Title: "El backend despliega antes que la app", OwnerKind: DocOwnerSpace, OwnerID: "s1",
					DecidedAt: day, SupersededByID: "dec2",
				}},
			}},
		},
		// El documento en sí (`doc` en el sobre): quién responde de él y
		// cuándo se confirmó por última vez. Es la cabecera que el core
		// pinta sobre las pestañas.
		"doc.json": Doc{
			BaseModel: BaseModel{
				ID:        "d1",
				CreatedAt: time.Date(2026, 6, 9, 12, 0, 0, 0, time.UTC),
				UpdatedAt: time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC),
			},
			OrgID:                 "o1",
			OwnerKind:             DocOwnerSpace,
			OwnerID:               "s1",
			Body:                  "lo que había antes de las pestañas",
			UpdatedBy:             "u1",
			UpdatedByName:         "Ana Pérez",
			MaintainerID:          "u1",
			MaintainerName:        "Ana Pérez",
			ReviewedAt:            &day,
			ReviewedBy:            "u2",
			ReviewedByName:        "Luis Gil",
			PinnedLine:            "Desplegar: backend antes que app",
			ReviewRequestedAt:     &day,
			ReviewRequestedBy:     "u2",
			ReviewRequestedByName: "Luis Gil",
			ReviewRequestNote:     "Cambié el runbook",
			Stale:                 true,
		},
		// Una página del documento, abierta: ella, sus migas, lo que cuelga
		// de ella y lo que la enlaza. Dentro va también la fila del árbol
		// (`DocPageTreeItem`), que es lo que contesta la lista de páginas.
		"doc_page_view.json": DocPageView{
			Page: &DocPage{
				BaseModel: BaseModel{
					ID:        "p2",
					CreatedAt: time.Date(2026, 6, 9, 12, 0, 0, 0, time.UTC),
					UpdatedAt: time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC),
				},
				DocID:         "d1",
				OrgID:         "o1",
				ParentID:      &pageParent,
				Title:         "El core compartido",
				Rank:          "0.75",
				Body:          "## Qué es\n\nLa lógica que comparten las apps.",
				BodyHash:      "abc123",
				UpdatedBy:     "u1",
				UpdatedByName: "Ana Pérez",
				Sections:      []DocSectionView{{ID: "s1", Heading: "Qué es", Level: 2, Index: 0}},
			},
			Breadcrumb: []DocPageCrumb{{ID: "p1", Title: "Arquitectura"}},
			Children: []DocPageTreeItem{{
				ID: "p3", ParentID: &pageID, Rank: "0.5", Title: "Los tipos", HasBody: true,
				UpdatedAt: time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC), DeletedAt: &day, Subpages: 2,
			}},
			OrgID: "o1",
			ReferencedFrom: []DocBacklink{
				{Kind: "tab", Title: "Command and control", Where: "runbook", Link: "/tasks?doc=space:s1&tab=runbook"},
			},
		},
		"doc_decision.json": Decision{
			BaseModel: BaseModel{
				ID:        "dec1",
				CreatedAt: time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC),
				UpdatedAt: time.Date(2026, 11, 14, 20, 17, 40, 0, time.UTC),
			},
			DocID:          "d1",
			Title:          "La interfaz se construye con GPUI Kit",
			Body:           "Porque trae los componentes.",
			Tag:            "arquitectura",
			AuthorID:       "u1",
			AuthorName:     "Ana Pérez",
			DecidedAt:      day,
			Origin:         DecisionFromTask,
			OriginTaskID:   "task1",
			OriginTitle:    "cac de escritorio en GPUI",
			SourceURL:      "https://example.com/ticket/1",
			SourceAuthor:   "Luis",
			Via:            "agent",
			SupersedesID:   "dec0",
			SupersededByID: "dec2",
			Links:          []DecisionLinkView{{ID: "k1", Kind: "task", TaskID: "task2", TaskRef: "acme-2", AddedAt: day}},
		},
		// Lo que cuentan la campana y el riel (`core/src/counters.rs`). El core
		// lee de cada una sólo lo que cuenta —`unread`, `spaceId` y `count`,
		// `orgId`, `status`—; van enteras para que el fichero sea la forma de
		// verdad el día que una pantalla lea lo demás.
		"notification_feed.json": NotificationFeed{
			Items: []Notification{{
				BaseModel:  BaseModel{ID: "n1", CreatedAt: day, UpdatedAt: day},
				OrgID:      "35d32154-922f-47dc-a6d9-051d4e78ae8a",
				Kind:       "chat:mention",
				Title:      "Ana te mencionó en #producto",
				Body:       "¿lo miras?",
				Link:       "/chat/e8bd6d46-a9ab-494e-ac29-de84d80a55f4",
				ReadAt:     &day,
				Via:        "mcp",
				GroupKey:   "chat:e8bd6d46-a9ab-494e-ac29-de84d80a55f4",
				GroupLabel: "#producto",
			}},
			Unread: 3,
			Groups: []GroupTally{{Key: "chat:e8bd6d46-a9ab-494e-ac29-de84d80a55f4", Label: "#producto", Total: 5, Unread: 3}},
		},
		"chat_unread.json": ChatUnread{SpaceID: "e8bd6d46-a9ab-494e-ac29-de84d80a55f4", Count: 4},
		"dm_summary.json": DMSummary{
			ConversationID: "c1",
			OrgID:          "35d32154-922f-47dc-a6d9-051d4e78ae8a",
			UserID:         "u2",
			Username:       "luis",
			Unread:         2,
			LastMessageAt:  &day,
			LastSeenAt:     &day,
			LastBody:       "¿quedamos mañana?",
			LastAuthorID:   "u2",
			OtherReadAt:    &day,
		},
		// La comunicación: una línea de un canal y una de un directo, con su
		// cita y sus reacciones, la última línea de un canal para su fila en la
		// lista, a quién se puede escribir y el aviso de que el otro leyó.
		//
		// Los eventos `chat:message` y `dm:message` **no** están aquí: el
		// servidor los escribe como mapas sueltos (`service/chat.go`,
		// `service/dm.go`), no con un struct, así que no hay forma que fijar. El
		// core los lee con tipos escritos mirando ese código.
		"chat_message_response.json": ChatMessageResponse{
			ID:           "m2",
			SpaceID:      "e8bd6d46-a9ab-494e-ac29-de84d80a55f4",
			AuthorUserID: "u2",
			AuthorName:   "Luis Mora",
			Kind:         ChatKindUser,
			Via:          "mcp",
			Body:         "Sí, **mañana** sale",
			CreatedAt:    day,
			UpdatedAt:    time.Date(2026, 9, 30, 0, 5, 0, 0, time.UTC),
			Reactions:    []ReactionSummary{{Emoji: ReactionEmojis[0], Count: 2, Mine: true, Names: []string{"Ana Pérez", "Luis Mora"}}},
			ReplyTo:      &MessageQuote{ID: "m1", AuthorName: "Ana Pérez", Body: "¿sale mañana?", Gone: true},
		},
		"dm_message_response.json": DMMessageResponse{
			ID:             "d2",
			ConversationID: "c1",
			AuthorUserID:   "u2",
			AuthorName:     "Luis Mora",
			Via:            "mcp",
			Body:           "¿quedamos mañana?",
			CreatedAt:      day,
			UpdatedAt:      time.Date(2026, 9, 30, 0, 5, 0, 0, time.UTC),
			Reactions:      []ReactionSummary{{Emoji: ReactionEmojis[1], Count: 1, Mine: true, Names: []string{"Ana Pérez"}}},
			ReplyTo:        &MessageQuote{ID: "d1", AuthorName: "Ana Pérez", Body: "¿cuándo?", Gone: true},
		},
		"chat_latest.json": ChatLatest{
			SpaceID:      "e8bd6d46-a9ab-494e-ac29-de84d80a55f4",
			AuthorUserID: "u2",
			AuthorName:   "Luis Mora",
			Kind:         ChatKindSystem,
			Body:         "Grabación lista",
			CreatedAt:    day,
		},
		"user_summary.json":  UserSummary{ID: "u2", Username: "luis", Name: "Luis Mora", LastSeenAt: &day},
		"dm_read_event.json": DMReadEvent{ConversationID: "c1", UserID: "u2", At: day},
		"server_response.json": ServerResponse{
			ID:                "s1",
			OrgID:             "35d32154-922f-47dc-a6d9-051d4e78ae8a",
			Name:              "pixel-vps",
			Host:              "203.0.113.7",
			SSHPort:           22,
			SSHUser:           "deploy",
			Type:              ServerTypeDockerSwarm,
			AgentPort:         8080,
			Status:            "offline",
			HasAgentToken:     true,
			AgentTokenPreview: "cac_…9f2a",
			AgentSeenAt:       &day,
			AgentVersion:      2,
		},
		// Lo que contesta subir un fichero a un canal o a un directo: la
		// caja de escribir lo cita por su `url` (relativa) y su nombre.
		"chat_attachment.json": ChatAttachment{
			BaseModel:   BaseModel{ID: "f1", CreatedAt: day, UpdatedAt: day},
			SpaceID:     "e8bd6d46-a9ab-494e-ac29-de84d80a55f4",
			URL:         "/api/v1/task-spaces/e8bd6d46-a9ab-494e-ac29-de84d80a55f4/chat/attachments/f1/raw",
			FileName:    "captura [final].png",
			ContentType: "image/png",
			Bytes:       2048,
			UploadedBy:  "u1",
		},
		"dm_attachment.json": DMAttachment{
			BaseModel:      BaseModel{ID: "f2", CreatedAt: day, UpdatedAt: day},
			ConversationID: "c1",
			URL:            "/api/v1/dm/c1/attachments/f2/raw",
			FileName:       "acta.pdf",
			ContentType:    "application/pdf",
			Bytes:          4096,
			UploadedBy:     "u1",
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
	// La regla de las subtareas abiertas, con su tabla: el core la copia
	// (`open_subtasks_block`) para no ofrecer lo que aquí se va a rechazar.
	type block struct {
		From    ReportStatus `json:"from"`
		To      ReportStatus `json:"to"`
		Open    int64        `json:"open"`
		Blocked bool         `json:"blocked"`
	}
	blocks := []block{}
	for _, from := range statuses {
		for _, to := range statuses {
			for _, open := range []int64{0, 1, 2} {
				blocks = append(blocks, block{from, to, open, OpenSubtasksBlock(from, to, open)})
			}
		}
	}
	return map[string]any{"transitions": pairs, "flows": owners, "priorities": priorities, "subtaskBlocks": blocks}
}
