package http_test

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"

	adapterhttp "github.com/guz-studio/cac/backend/internal/adapters/http"
	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
)

// Un cliente que cierra su ticket por su API mientras el equipo tiene una
// subtarea abierta debajo: se le niega con 409 y un código suyo, el ticket no
// cambia, y nada de lo que recibe le cuenta lo interno (command-and-control-387).
//
// De punta a punta, con el router de verdad y la llave del proyecto, porque lo
// que hay que sujetar es el cableado: la regla vive en el servicio y la frase
// para el cliente en el handler, y una prueba de cada pieza por separado no ve
// si la ruta dejó de pasar por alguna.
//
// Roturas que atrapa: que la puerta del tenant no imponga la regla; que le
// llegue la frase o el código de dentro, con «subtask» o la cuenta; que el rechazo cambie
// el estado igualmente; y que, ya sin subtareas abiertas, siga sin poder
// cerrar.
func TestATenantClosingATicketWithOpenInternalWorkIsRefusedWithoutDetails(t *testing.T) {
	db, cleanup := e2eDB(t)
	defer cleanup()

	org := &domain.Organization{Name: "E2E Org", Slug: "e2e-org"}
	org.ID = "org-e2e"
	if err := db.Create(org).Error; err != nil {
		t.Fatal(err)
	}
	const key = "pk_e2e_subtasks_key"
	proj := mkProject(t, db, "proj-s", "portento", org.ID, key)
	rep := mkReport(t, db, "rep-s", proj.ID, "ticket con trabajo debajo")
	// La subtarea, como la deja `CreateTask`: interna y sin proyecto.
	parent := rep.ID
	step := &domain.Item{Title: "Un paso interno", Status: domain.ReportInProgress, Visibility: domain.VisibilityInternal, ParentID: &parent}
	step.ID = "step-s"
	if err := db.Create(step).Error; err != nil {
		t.Fatal(err)
	}

	r := chi.NewRouter()
	adapterhttp.InitReportRoutes(db, r, events.NewHub())
	srv := httptest.NewServer(r)
	defer srv.Close()

	closeIt := func() (int, domain.APIResponse[json.RawMessage], string) {
		t.Helper()
		req, _ := http.NewRequest(http.MethodPatch, srv.URL+"/api/v1/reports/"+rep.ID, bytes.NewBufferString(`{"status":"closed"}`))
		req.Header.Set("X-Ingest-Key", key)
		req.Header.Set("Content-Type", "application/json")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		raw, _ := io.ReadAll(resp.Body)
		var body domain.APIResponse[json.RawMessage]
		_ = json.Unmarshal(raw, &body)
		return resp.StatusCode, body, string(raw)
	}
	statusNow := func() domain.ReportStatus {
		t.Helper()
		var it domain.Item
		if err := db.First(&it, "id = ?", rep.ID).Error; err != nil {
			t.Fatal(err)
		}
		return it.Status
	}

	code, body, raw := closeIt()
	if code != http.StatusConflict || body.Error != "work-still-open" {
		t.Fatalf("cerrar con trabajo abierto debajo → %d %q, quería 409 work-still-open: %s", code, body.Error, raw)
	}
	if strings.Contains(strings.ToLower(raw), "subtask") || strings.ContainsAny(body.Message, "0123456789") {
		t.Fatalf("la respuesta le cuenta al cliente lo interno: %s", raw)
	}
	if !strings.Contains(body.Message, "cannot resolve or close this report yet") {
		t.Fatalf("la frase no dice qué pasa: %q", body.Message)
	}
	if got := statusNow(); got != domain.ReportPending {
		t.Fatalf("rechazado y aun así quedó en %s", got)
	}

	// Con el paso interno terminado, el cliente ya puede cerrar.
	if err := db.Model(&domain.Item{}).Where("id = ?", step.ID).Update("status", domain.ReportResolved).Error; err != nil {
		t.Fatal(err)
	}
	if code, _, raw := closeIt(); code != http.StatusOK {
		t.Fatalf("sin trabajo abierto debajo → %d, quería 200: %s", code, raw)
	}
	if got := statusNow(); got != domain.ReportClosed {
		t.Fatalf("no se cerró: %s", got)
	}
}
