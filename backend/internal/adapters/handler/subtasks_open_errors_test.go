package handler

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// Terminar o cerrar un padre con subtareas abiertas contesta 409, con su
// código y con una frase que dice el motivo y la cuenta, por las dos puertas
// (command-and-control-387).
//
// La frase importa tanto como el código: un agente por el MCP o un tenant por
// su API imprimen lo que contesta el servidor, y «409» a secas no les dice qué
// hacer.
//
// Roturas que atrapa: que la pantalla de reportes lo deje caer al 500 (no
// tenía el caso); que una puerta conteste otro código que la otra; y una frase
// fija que no lleve cuántas quedan.
func TestOpenSubtasksAnswer409WithTheReasonThroughBothDoors(t *testing.T) {
	refusal := &service.SubtasksOpenError{Open: 2, To: domain.ReportClosed}
	doors := map[string]func(http.ResponseWriter, error) bool{
		"board":   mapTaskError,
		"reports": mapReportError,
	}
	for door, mapError := range doors {
		rec := httptest.NewRecorder()
		if !mapError(rec, refusal) {
			t.Fatalf("%s: el rechazo cayó al default y saldría como 500", door)
		}
		var body domain.APIResponse[any]
		if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
			t.Fatal(err)
		}
		if rec.Code != http.StatusConflict || body.Error != "subtasks-open" {
			t.Fatalf("%s: %d %q; quería 409 subtasks-open", door, rec.Code, body.Error)
		}
		if body.Message != "cannot close this task: 2 subtasks are still open — finish or close them first" {
			t.Fatalf("%s: la frase no dice el motivo con su cuenta: %q", door, body.Message)
		}
	}
}

// A un cliente que entra con la llave de su proyecto se le niega igual, pero
// sin contarle que hay subtareas internas colgando de su ticket ni cuántas:
// son internas siempre, y es trabajo que no ve. Ni en la frase ni en el código.
//
// Roturas que atrapa: mandarle la frase o el código de dentro, con la palabra
// «subtask» o la cuenta; dejarle sin un código propio que reconocer; quitarle
// la cuenta también a una persona del equipo; y tomar por este rechazo
// cualquier otro fallo del tenant.
func TestATenantIsRefusedWithoutTheInternalCount(t *testing.T) {
	refusal := &service.SubtasksOpenError{Open: 2, To: domain.ReportClosed}
	tenant := &domain.ClaimsJWT{ProjectID: "proj-1"}
	if !tenant.IsProjectScoped() {
		t.Fatal("la fixture no es una llave de proyecto: hay que rehacer esta prueba")
	}

	rec := httptest.NewRecorder()
	if !refuseOpenSubtasksToTenant(rec, tenant, refusal) {
		t.Fatal("al tenant le llegaría la frase de dentro")
	}
	var body domain.APIResponse[any]
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if rec.Code != http.StatusConflict || body.Error != "work-still-open" {
		t.Fatalf("%d %q; quería 409 work-still-open", rec.Code, body.Error)
	}
	if strings.Contains(rec.Body.String(), "subtask") || strings.Contains(body.Message, "2") {
		t.Fatalf("la frase le cuenta al cliente lo interno: %q", body.Message)
	}
	if !strings.Contains(body.Message, "cannot resolve or close this report yet") {
		t.Fatalf("la frase no dice qué pasa: %q", body.Message)
	}

	// Una persona del equipo recibe la de siempre, por el camino de siempre.
	if refuseOpenSubtasksToTenant(httptest.NewRecorder(), &domain.ClaimsJWT{UserID: "u1"}, refusal) {
		t.Fatal("a alguien del equipo no se le esconde la cuenta")
	}
	// Y otro fallo del tenant no es éste.
	if refuseOpenSubtasksToTenant(httptest.NewRecorder(), tenant, service.ErrInvalidTransition) {
		t.Fatal("cualquier fallo de un tenant saldría como subtareas abiertas")
	}
}
