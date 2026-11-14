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
