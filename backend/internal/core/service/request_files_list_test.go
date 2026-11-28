package service

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// La lista de pedidos trae los ficheros con su nombre, su papel y su tamaño,
// pero sin el contenido (hasta 1 MB por pedido, y la app la vuelve a pedir con
// cada evento); el detalle sí lo trae entero (command-and-control-567).
//
// Roturas que atrapa: que la lista lleve el contenido; que pierda el nombre,
// el papel o el tamaño; que el detalle venga sin contenido o sin tamaño.
func TestTheRequestListCarriesFilesWithoutTheirContent(t *testing.T) {
	f := newRequestFixture(t)
	apply := "update pagos set moneda = 'USD' where id = 7;\n"
	restore := "update pagos set moneda = 'MXN' where id = 7;\n"
	body, _ := json.Marshal(map[string]any{
		"command": "Pegar apply.sql", "where": "Supabase", "expected": "UPDATE 1", "undo": "Pegar restore.sql",
		"files": []map[string]string{{"name": "apply.sql", "content": apply, "role": "apply"}, {"name": "restore.sql", "content": restore, "role": "undo"}},
	})
	v := f.create(t, "it-0", "u-ana", "Bea Gil", domain.ViaMCP, domain.RequestRun, string(body))

	list, err := f.svc.List(repository.RequestFilter{OrgIDs: []string{"org-1"}, RequestedOf: "u-bea", Statuses: []domain.RequestStatus{domain.RequestOpen}}, "u-bea")
	if err != nil || len(list) != 1 {
		t.Fatalf("la lista: %v %+v", err, list)
	}
	want := []domain.RequestFile{{Name: "apply.sql", Role: "apply", Size: len(apply)}, {Name: "restore.sql", Role: "undo", Size: len(restore)}}
	got := list[0].Body.Files
	if len(got) != len(want) || got[0] != want[0] || got[1] != want[1] {
		t.Fatalf("en la lista, cada fichero con nombre, papel y tamaño, sin contenido:\n got %+v\nwant %+v", got, want)
	}
	wire, _ := json.Marshal(list[0])
	if strings.Contains(string(wire), "moneda") || strings.Contains(string(wire), `"content"`) {
		t.Fatalf("el contenido no viaja en la lista: %s", wire)
	}

	detail := f.svc.View(f.find(t, v.Code), "u-bea", true)
	full := []domain.RequestFile{{Name: "apply.sql", Content: apply, Role: "apply", Size: len(apply)}, {Name: "restore.sql", Content: restore, Role: "undo", Size: len(restore)}}
	if len(detail.Body.Files) != 2 || detail.Body.Files[0] != full[0] || detail.Body.Files[1] != full[1] {
		t.Fatalf("el detalle trae el contenido entero:\n got %+v\nwant %+v", detail.Body.Files, full)
	}
}
