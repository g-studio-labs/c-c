package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

/*
Un padre no se termina ni se cierra con subtareas abiertas
(command-and-control-387, decidido por jose el 17-nov-2026).

La regla es `domain.OpenSubtasksBlock`; aquí se sujeta que las dos puertas que
cambian un estado la imponen (el tablero, `TaskService.MoveTask`, y la pantalla
de reportes, `ReportService.Update`), en los dos flujos, y que la cuenta que la
alimenta cuenta lo que tiene que contar. Con Postgres de verdad: sin `DB_HOST`
estas pruebas se saltan y un verde no dice nada.
*/

func statusOf(t *testing.T, db *gorm.DB, id string) domain.ReportStatus {
	t.Helper()
	var it domain.Item
	if err := db.First(&it, "id = ?", id).Error; err != nil {
		t.Fatal(err)
	}
	return it.Status
}

func setStatus(t *testing.T, db *gorm.DB, id string, to domain.ReportStatus) {
	t.Helper()
	if err := db.Model(&domain.Item{}).Where("id = ?", id).Update("status", to).Error; err != nil {
		t.Fatal(err)
	}
}

// wantOpenSubtasks: el error es el rechazo de la regla, con esa cuenta.
func wantOpenSubtasks(t *testing.T, err error, open int64, what string) {
	t.Helper()
	var refusal *SubtasksOpenError
	if !errors.Is(err, ErrSubtasksOpen) || !errors.As(err, &refusal) {
		t.Fatalf("%s: se esperaba el rechazo por subtareas abiertas y vino %v", what, err)
	}
	if refusal.Open != open {
		t.Fatalf("%s: dice %d abiertas y son %d", what, refusal.Open, open)
	}
}

// En el tablero, una tarea interna con una subtarea abierta no pasa ni a hecha
// ni a cerrada, y no queda rastro de un cambio que no ocurrió; con la subtarea
// terminada (o cerrada) sí pasa.
//
// Roturas que atrapa: no preguntar en esta puerta; vigilar sólo «hecho» y
// dejar cerrar (la regla de antes); rechazar y aun así dejar la línea de
// «status:» en el hilo; seguir bloqueando cuando ya no queda ninguna abierta;
// contar una subtarea cerrada como abierta; y bloquear a la propia subtarea,
// que no tiene hijas.
func TestTheBoardRefusesToFinishOrCloseAParentWithOpenSubtasks(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)
	ctx := context.Background()
	move := func(id string, to domain.ReportStatus) error {
		return svc.MoveTask(ctx, id, ana, domain.MoveTaskRequest{StatusID: domain.SyntheticStatusID("lista-1", to)})
	}

	for _, to := range []domain.ReportStatus{domain.ReportResolved, domain.ReportClosed} {
		wantOpenSubtasks(t, move("int-1", to), 1, "pendiente → "+string(to))
		if got := statusOf(t, db, "int-1"); got != domain.ReportPending {
			t.Fatalf("rechazada y aun así quedó en %s", got)
		}
	}
	if rows := threadRows(t, db, "int-1"); len(rows) != 0 {
		t.Fatalf("un cambio rechazado no deja línea en el hilo: %+v", rows)
	}
	// Lo de todos los días no se toca: pasar a en curso con subtareas abiertas.
	if err := move("int-1", domain.ReportInProgress); err != nil {
		t.Fatalf("pasar a en curso no es terminar nada: %v", err)
	}
	wantOpenSubtasks(t, move("int-1", domain.ReportResolved), 1, "en curso → hecha")

	// La subtarea no tiene hijas: se termina sin más. Y con ella hecha, el
	// padre ya puede.
	if err := move("sub-1", domain.ReportResolved); err != nil {
		t.Fatalf("una subtarea sin hijas se termina: %v", err)
	}
	if err := move("int-1", domain.ReportResolved); err != nil {
		t.Fatalf("sin ninguna abierta el padre se termina: %v", err)
	}

	// Cerrada también vale como «ya no está abierta».
	setStatus(t, db, "int-1", domain.ReportInProgress)
	setStatus(t, db, "sub-1", domain.ReportClosed)
	if err := move("int-1", domain.ReportClosed); err != nil {
		t.Fatalf("una subtarea cerrada no es trabajo pendiente: %v", err)
	}
}

// Reabrir un padre se puede siempre, aunque tenga subtareas abiertas, y
// reordenarlo dentro de la columna en la que ya está también.
//
// Es la salida de un padre que quedó terminado con algo abierto debajo (los
// que ya lo estaban antes de la regla, o al que se le añadió una subtarea
// después).
//
// Roturas que atrapa: aplicar la regla a cualquier movimiento de un padre con
// subtareas abiertas, que lo dejaría atrapado en «hecha»; y contar como cambio
// soltar la tarjeta en su misma columna.
func TestReopeningOrReorderingAParentWithOpenSubtasksIsAllowed(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)
	ctx := context.Background()
	setStatus(t, db, "int-1", domain.ReportResolved)

	moveTo(t, svc, ctx, "int-1", ana, domain.ReportResolved) // reordenar en su columna
	moveTo(t, svc, ctx, "int-1", ana, domain.ReportInProgress)
	if got := statusOf(t, db, "int-1"); got != domain.ReportInProgress {
		t.Fatalf("no se reabrió: %s", got)
	}
	setStatus(t, db, "int-1", domain.ReportClosed)
	moveTo(t, svc, ctx, "int-1", ana, domain.ReportPending)
}

// La regla vale igual en el flujo del cliente, y por las dos puertas: un
// ticket de un cliente con una subtarea abierta no se resuelve ni se cierra
// desde el tablero ni desde la pantalla de reportes.
//
// Roturas que atrapa: que la pantalla de reportes no pregunte (no preguntaba:
// la regla de antes sólo vivía en el tablero); que el flujo del cliente quede
// fuera; que el rechazo cambie el estado igualmente; y que, ya sin subtareas
// abiertas, esa puerta siga negándose.
func TestTheClientFlowAndTheReportsDoorRefuseToo(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	tasks := taskSvcFor(db)
	reports := reportSvcDePrueba(db, &notificadorEspia{})
	ctx := context.Background()

	// item-1 es del cliente, público y está en curso: desde ahí su flujo deja
	// resolver y cerrar. Se le cuelga una subtarea, que es interna siempre.
	parent := "item-1"
	child := &domain.Item{OrgID: "org-1", ListID: "lista-1", Title: "Un paso", Status: domain.ReportInProgress, Visibility: domain.VisibilityInternal, ParentID: &parent}
	child.ID = "sub-client"
	if err := db.Create(child).Error; err != nil {
		t.Fatal(err)
	}

	for _, to := range []domain.ReportStatus{domain.ReportResolved, domain.ReportClosed} {
		to := to
		err := tasks.MoveTask(ctx, "item-1", ana, domain.MoveTaskRequest{StatusID: domain.SyntheticStatusID("lista-1", to)})
		wantOpenSubtasks(t, err, 1, "tablero, ticket de cliente → "+string(to))
		_, err = reports.Update(ctx, "team", ana, "item-1", domain.UpdateReportRequest{Status: &to})
		wantOpenSubtasks(t, err, 1, "reportes, ticket de cliente → "+string(to))
		if got := statusOf(t, db, "item-1"); got != domain.ReportInProgress {
			t.Fatalf("rechazada y aun así quedó en %s", got)
		}
	}
	if rows := threadRows(t, db, "item-1"); len(rows) != 0 {
		t.Fatalf("un cambio rechazado no deja línea que lea el cliente: %+v", rows)
	}

	setStatus(t, db, "sub-client", domain.ReportResolved)
	resolved := domain.ReportResolved
	if _, err := reports.Update(ctx, "team", ana, "item-1", domain.UpdateReportRequest{Status: &resolved}); err != nil {
		t.Fatalf("sin subtareas abiertas la pantalla de reportes resuelve: %v", err)
	}
	if got := statusOf(t, db, "item-1"); got != domain.ReportResolved {
		t.Fatalf("no se resolvió: %s", got)
	}
}

// La cuenta: abiertas son las que no están ni hechas ni cerradas, sólo las
// hijas directas de esa tarea, y sin las archivadas ni las borradas.
//
// Roturas que atrapa: contar todas las subtareas; contar sólo las pendientes y
// olvidar las que están en curso; tomar por abierta una cerrada; contar las de
// otro padre; y contar una archivada o una borrada, que dejarían al padre
// atrapado por algo que ya no sale en ninguna pantalla.
func TestCountOpenSubtasksCountsOnlyWhatIsStillOpen(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	repo := repository.NewTaskRepository(db)

	// `sub-1` ya cuelga de `int-1`, pendiente.
	parent, other := "int-1", "int-proj"
	now := time.Now()
	add := func(id string, of *string, status domain.ReportStatus, change func(*domain.Item)) {
		t.Helper()
		it := &domain.Item{OrgID: "org-1", ListID: "lista-1", Title: id, Status: status, Visibility: domain.VisibilityInternal, ParentID: of}
		it.ID = id
		if change != nil {
			change(it)
		}
		if err := db.Create(it).Error; err != nil {
			t.Fatal(err)
		}
	}
	add("s-progress", &parent, domain.ReportInProgress, nil)
	add("s-done", &parent, domain.ReportResolved, nil)
	add("s-closed", &parent, domain.ReportClosed, nil)
	add("s-archived", &parent, domain.ReportPending, func(it *domain.Item) { it.ArchivedAt = &now })
	add("s-deleted", &parent, domain.ReportPending, nil)
	if err := db.Delete(&domain.Item{}, "id = ?", "s-deleted").Error; err != nil {
		t.Fatal(err)
	}
	add("s-elsewhere", &other, domain.ReportPending, nil)

	open, err := repo.CountOpenSubtasks("int-1")
	if err != nil || open != 2 {
		t.Fatalf("abiertas de int-1: %d (%v); son la pendiente y la que está en curso", open, err)
	}
	// La otra puerta hace la misma cuenta.
	if open, err := repository.NewReportRepository(db).CountOpenSubtasks("int-1"); err != nil || open != 2 {
		t.Fatalf("la pantalla de reportes cuenta otra cosa: %d (%v)", open, err)
	}
	if open, err := repo.CountOpenSubtasks("sub-1"); err != nil || open != 0 {
		t.Fatalf("una subtarea sin hijas no tiene ninguna abierta: %d (%v)", open, err)
	}
}
