package service

import (
	"context"
	"errors"
	"testing"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

/*
Un cambio de estado deja su línea en el hilo, también en una tarea interna, y
dice quién lo hizo (command-and-control-383, decidido por jose el 17-nov-2026:
«creo que es lo correcto para que quede a modo de logs, y quién lo hizo sería un
plus útil en x casos»).

Hasta entonces sólo la dejaba una tarea que ve un cliente. Lo que estas pruebas
sujetan son las dos mitades de la regla: lo interno gana su rastro con autor, y
lo que el cliente lee no cambia en nada: ni el texto, ni la audiencia, ni un
autor que le diría qué compañero fue.
*/

// statusTraceDB: la fixture de los avisos (item-1 e item-2 son de un cliente y
// públicos) más lo que hace falta aquí: una tarea interna, su subtarea, y el
// nombre de Ana, que no es su usuario.
func statusTraceDB(t *testing.T) (*gorm.DB, func()) {
	t.Helper()
	db, cleanup := avisosDB(t)
	// Lo que lee el detalle de una tarea además de lo que ya migra la fixture.
	if err := db.AutoMigrate(&domain.TaskTag{}, &domain.TaskTagLink{}, &domain.ItemBlock{}, &domain.TaskGitLink{}); err != nil {
		t.Fatal(err)
	}
	if err := db.Model(&domain.User{}).Where("id = ?", ana).Update("name", "Ana Pérez").Error; err != nil {
		t.Fatal(err)
	}
	internal := &domain.Item{OrgID: "org-1", ListID: "lista-1", Title: "Interna", Status: domain.ReportPending, Visibility: domain.VisibilityInternal}
	internal.ID = "int-1"
	parent := "int-1"
	subtask := &domain.Item{OrgID: "org-1", ListID: "lista-1", Title: "Una línea del checklist", Status: domain.ReportPending, Visibility: domain.VisibilityInternal, ParentID: &parent}
	subtask.ID = "sub-1"
	// Una del canal del cliente que se le ha escondido: sigue siendo de su
	// proyecto (el folio está gastado) pero ya no la ve. Es la interna que
	// alcanza la pantalla de reportes, que sólo abre lo que tiene proyecto.
	retracted := &domain.Item{OrgID: "org-1", ListID: "lista-1", ProjectID: "proj-1", Seq: 90, Title: "Retirada del canal", Status: domain.ReportPending, Visibility: domain.VisibilityInternal}
	retracted.ID = "int-proj"
	for _, it := range []*domain.Item{internal, subtask, retracted} {
		if err := db.Create(it).Error; err != nil {
			t.Fatal(err)
		}
	}
	return db, cleanup
}

func taskSvcFor(db *gorm.DB) *TaskService {
	return NewTaskService(repository.NewTaskRepository(db), repository.NewReportRepository(db),
		repository.NewOrganizationRepository(db), nil)
}

// threadRows: las filas del hilo tal como están guardadas, en orden.
func threadRows(t *testing.T, db *gorm.DB, itemID string) []domain.ItemComment {
	t.Helper()
	var rows []domain.ItemComment
	if err := db.Where("item_id = ?", itemID).Order("created_at ASC").Find(&rows).Error; err != nil {
		t.Fatal(err)
	}
	return rows
}

func moveTo(t *testing.T, svc *TaskService, ctx context.Context, id, actor string, to domain.ReportStatus) {
	t.Helper()
	if err := svc.MoveTask(ctx, id, actor, domain.MoveTaskRequest{StatusID: domain.SyntheticStatusID("lista-1", to)}); err != nil {
		t.Fatalf("mover %s a %s: %v", id, to, err)
	}
}

// Mover una tarea interna deja una línea, interna, de sistema, con quien la
// movió; y volver a soltarla en la misma columna no deja otra.
//
// Roturas que atrapa: no escribir nada en una interna (lo de antes); escribirla
// pública; escribirla sin autor o como comentario de persona; cambiar el texto;
// y escribir una línea cada vez que la tarjeta se reordena sin cambiar de
// estado.
func TestMovingAnInternalTaskLeavesAnInternalLineWithWhoDidIt(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)
	ctx := context.Background()

	moveTo(t, svc, ctx, "int-1", ana, domain.ReportInProgress)
	rows := threadRows(t, db, "int-1")
	if len(rows) != 1 {
		t.Fatalf("un cambio de estado es una línea, y hay %d: %+v", len(rows), rows)
	}
	line := rows[0]
	if line.Body != "status: pending → in_progress" {
		t.Errorf("el texto es el que las apps ya pintan, y salió %q", line.Body)
	}
	if line.Kind != domain.CommentKindSystem {
		t.Errorf("es una línea del servidor, no un comentario de nadie: kind=%q", line.Kind)
	}
	if line.Visibility != domain.VisibilityInternal {
		t.Errorf("lo de una tarea interna es interno: visibility=%q", line.Visibility)
	}
	if line.AuthorUserID == nil || *line.AuthorUserID != ana {
		t.Errorf("tenía que quedar quién la movió (%s): %v", ana, line.AuthorUserID)
	}

	// La misma columna otra vez (reordenar): ninguna línea más.
	moveTo(t, svc, ctx, "int-1", ana, domain.ReportInProgress)
	if rows := threadRows(t, db, "int-1"); len(rows) != 1 {
		t.Fatalf("reordenar sin cambiar de estado no es noticia, y dejó %d líneas", len(rows))
	}
}

// Quién movió la tarea llega al tablero con el nombre con que se le llama, y
// sin su id: la línea es del servidor, y la app de siempre ofrece editar y
// borrar todo comentario cuyo `authorUserId` es el de quien mira.
//
// Roturas que atrapa: mandar el usuario («ana») en vez del nombre; mandar el id
// del autor en una línea de sistema; etiquetarla con un `author` de persona; y
// dejar de mandar el nombre.
func TestTheBoardNamesWhoChangedTheStatusButTheLineIsNobodys(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)
	moveTo(t, svc, context.Background(), "int-1", ana, domain.ReportInProgress)

	detail, err := svc.Detail("int-1")
	if err != nil {
		t.Fatal(err)
	}
	if len(detail.Comments) != 1 {
		t.Fatalf("el detalle trae la línea: %+v", detail.Comments)
	}
	line := detail.Comments[0]
	if line.AuthorName != "Ana Pérez" {
		t.Errorf("se enseña cómo se llama, no cómo entra: %q", line.AuthorName)
	}
	if line.AuthorUserID != "" || line.Author != nil {
		t.Errorf("una línea de sistema no es de nadie: id=%q author=%+v", line.AuthorUserID, line.Author)
	}
	if line.Kind != domain.CommentKindSystem || line.Visibility != domain.VisibilityInternal {
		t.Errorf("llega como línea de sistema interna: %q %q", line.Kind, line.Visibility)
	}
}

// El check de «hecho» de una subtarea es un cambio de estado como otro
// cualquiera (Open → Done, que sólo existe en el flujo interno), y deja el
// mismo rastro en el hilo de la subtarea.
//
// Rotura que atrapa: que el rastro dependa de algo que una subtarea no tiene
// (un proyecto, un folio) o que el salto directo a hecho se quede sin línea.
func TestTickingASubtaskLeavesTheLineToo(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)

	moveTo(t, svc, context.Background(), "sub-1", bea, domain.ReportResolved)
	rows := threadRows(t, db, "sub-1")
	if len(rows) != 1 || rows[0].Body != "status: pending → resolved" ||
		rows[0].Visibility != domain.VisibilityInternal || rows[0].AuthorUserID == nil || *rows[0].AuthorUserID != bea {
		t.Fatalf("marcar una subtarea deja su línea interna con quien la marcó: %+v", rows)
	}
	if rows := threadRows(t, db, "int-1"); len(rows) != 0 {
		t.Fatalf("la línea es de la subtarea, no de su madre: %+v", rows)
	}
}

// Sin persona detrás (una automatización, un webhook) la línea se escribe
// igual, sin autor; y por dónde entró —un agente con el token de alguien—
// queda en la fila.
//
// Roturas que atrapa: no dejar línea cuando no se sabe quién fue; guardar un
// autor vacío («») en vez de ninguno, que el tablero resolvería contra nadie; y
// perder la marca de que lo movió un agente.
func TestAnUnknownActorLeavesTheLineWithoutAnAuthor(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)

	moveTo(t, svc, context.Background(), "int-1", "", domain.ReportInProgress)
	rows := threadRows(t, db, "int-1")
	if len(rows) != 1 || rows[0].Kind != domain.CommentKindSystem || rows[0].Visibility != domain.VisibilityInternal {
		t.Fatalf("sin persona también queda la línea: %+v", rows)
	}
	if rows[0].AuthorUserID != nil {
		t.Errorf("sin persona no hay autor, y se guardó %q", *rows[0].AuthorUserID)
	}

	// De vuelta a pendiente y no a hecha: `int-1` tiene una subtarea abierta,
	// y con ella no se termina (command-and-control-387).
	moveTo(t, svc, domain.WithVia(context.Background(), domain.ViaMCP), "int-1", ana, domain.ReportPending)
	rows = threadRows(t, db, "int-1")
	if len(rows) != 2 || rows[1].Via != domain.ViaMCP {
		t.Fatalf("lo que mueve un agente con el token de Ana lo dice la fila: %+v", rows)
	}
}

// En una tarea que ve un cliente no cambia nada: la línea pública de siempre,
// sin autor, y ni una fila más. Lo que el cliente recibe de ella no nombra a
// nadie del equipo.
//
// Roturas que atrapa: ponerle autor a la línea pública (el `authorUserId` y el
// `authorName` de un comentario viajan en lo que devuelve la API del tenant);
// añadir una segunda línea interna con el quién; y volverla interna, que se la
// quitaría al cliente.
func TestMovingAClientVisibleTaskLeavesExactlyThePublicLineAsBefore(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)

	// item-2 está pendiente; en el flujo del cliente sólo puede ir a en curso.
	moveTo(t, svc, context.Background(), "item-2", ana, domain.ReportInProgress)
	rows := threadRows(t, db, "item-2")
	if len(rows) != 1 {
		t.Fatalf("una línea, la de siempre, y hay %d: %+v", len(rows), rows)
	}
	line := rows[0]
	if line.Body != "status: pending → in_progress" || line.Kind != domain.CommentKindSystem || line.Visibility != domain.VisibilityPublic {
		t.Errorf("la línea pública cambió: %q %q %q", line.Body, line.Kind, line.Visibility)
	}
	if line.AuthorUserID != nil || line.Via != "" {
		t.Errorf("el cliente no tiene por qué saber quién la movió ni por dónde: autor=%v via=%q", line.AuthorUserID, line.Via)
	}

	// Lo que lee el cliente (su API y el reporter): la línea, y nadie del equipo.
	seen, err := repository.NewReportRepository(db).ListComments("item-2", false)
	if err != nil {
		t.Fatal(err)
	}
	if len(seen) != 1 || seen[0].Body != line.Body || seen[0].AuthorUserID != nil || seen[0].AuthorName != "" || seen[0].Author != nil {
		t.Fatalf("lo que ve el cliente: %+v", seen)
	}
}

// La forma que se manda fuera de cac nunca dice quién está detrás de una línea
// de sistema, aunque la fila lo lleve; y una línea interna no sale.
//
// Es la red de `tagAuthor`: hoy ninguna línea pública lleva autor, pero la
// columna existe, y el día que alguien se lo ponga esta forma no puede
// enseñárselo al tenant.
//
// Roturas que atrapa: dejar pasar `authorUserId`/`authorName` de una línea de
// sistema en la forma del tenant; y que la línea interna de una tarea salga a
// quien lee desde fuera.
func TestTheClientFacingShapeNeverNamesWhoIsBehindASystemLine(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	reports := repository.NewReportRepository(db)
	author := ana
	for id, visibility := range map[string]domain.ItemVisibility{"sys-pub": domain.VisibilityPublic, "sys-int": domain.VisibilityInternal} {
		c := &domain.ItemComment{ItemID: "item-1", Kind: domain.CommentKindSystem, Visibility: visibility, Body: "status: a → b (" + id + ")", AuthorUserID: &author}
		c.ID = id
		if err := db.Create(c).Error; err != nil {
			t.Fatal(err)
		}
	}

	outside, err := reports.ListComments("item-1", false)
	if err != nil {
		t.Fatal(err)
	}
	if len(outside) != 1 || outside[0].ID != "sys-pub" {
		t.Fatalf("fuera sólo sale lo público: %+v", outside)
	}
	if outside[0].AuthorUserID != nil || outside[0].AuthorName != "" || outside[0].Author != nil {
		t.Errorf("al cliente se le dijo quién: id=%v name=%q author=%+v", outside[0].AuthorUserID, outside[0].AuthorName, outside[0].Author)
	}
}

// La otra puerta, la pantalla de reportes y la API (`PATCH /reports/{id}`),
// deja lo mismo que el tablero: en una interna, la línea interna con la
// persona; si quien llama es un tenant con su llave, sin autor; y en una del
// cliente, la pública sin autor.
//
// Roturas que atrapa: que esta puerta siga escribiendo la línea pública sin
// autor en una tarea interna (lo que hacía); que tome por persona a un tenant;
// y que le ponga autor a la línea que lee el cliente.
func TestTheReportsDoorLeavesTheSameTraceAsTheBoard(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := reportSvcDePrueba(db, &notificadorEspia{})
	ctx := context.Background()
	// En una interna, `Update` hace el cambio y luego no encuentra la ficha al
	// releerla para contestar: el detalle de reportes resuelve el proyecto con
	// la regla de fuera (`ProjectForReport`, que no ve lo retirado). Es de
	// antes y no es de esta tarea; aquí lo que se mira es lo que quedó escrito.
	update := func(actor, userID, id string, to domain.ReportStatus) {
		t.Helper()
		_, err := svc.Update(ctx, actor, userID, id, domain.UpdateReportRequest{Status: &to})
		if err != nil && !(id == "int-proj" && errors.Is(err, repository.ErrReportNotFound)) {
			t.Fatalf("mover %s a %s: %v", id, to, err)
		}
	}

	update("team", ana, "int-proj", domain.ReportInProgress)
	rows := threadRows(t, db, "int-proj")
	if len(rows) != 1 || rows[0].Body != "status: pending → in_progress" || rows[0].Kind != domain.CommentKindSystem ||
		rows[0].Visibility != domain.VisibilityInternal || rows[0].AuthorUserID == nil || *rows[0].AuthorUserID != ana {
		t.Fatalf("una interna movida desde reportes: %+v", rows)
	}
	// El mismo estado otra vez: nada.
	update("team", ana, "int-proj", domain.ReportInProgress)
	if rows := threadRows(t, db, "int-proj"); len(rows) != 1 {
		t.Fatalf("pedir el estado que ya tiene no deja línea: %+v", rows)
	}
	// Un tenant no es una persona de cac, traiga lo que traiga en el id. (Su
	// puerta no le deja llegar a una interna; esto es la segunda negativa.)
	update("project:portento", "no-es-nadie", "int-proj", domain.ReportResolved)
	if rows := threadRows(t, db, "int-proj"); len(rows) != 2 || rows[1].AuthorUserID != nil {
		t.Fatalf("lo que mueve un tenant va sin autor: %+v", rows)
	}

	update("team", ana, "item-2", domain.ReportInProgress)
	rows = threadRows(t, db, "item-2")
	if len(rows) != 1 || rows[0].Visibility != domain.VisibilityPublic || rows[0].AuthorUserID != nil ||
		rows[0].Body != "status: pending → in_progress" {
		t.Fatalf("la del cliente, como siempre: %+v", rows)
	}
}

// Mover una tarea no es haber hablado en su hilo: quien sólo cambió el estado
// no pasa a recibir los comentarios que se escriban después.
//
// Rotura que atrapa: contar al autor de una línea de sistema entre los
// implicados del hilo, que es lo que haría la consulta de antes ahora que esas
// líneas llevan autor.
func TestWhoOnlyMovedATaskIsNotInItsThread(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := taskSvcFor(db)
	moveTo(t, svc, context.Background(), "int-1", caro, domain.ReportInProgress)

	involved, err := repository.NewReportRepository(db).Involved("int-1")
	if err != nil {
		t.Fatal(err)
	}
	if len(involved) != 0 {
		t.Fatalf("Caro sólo la movió, y quedó como implicada: %v", involved)
	}
	// Y quien sí escribe, sí.
	if _, err := svc.AddComment(context.Background(), "int-1", bea, "lo miro", domain.VisibilityInternal); err != nil {
		t.Fatal(err)
	}
	if involved, _ := repository.NewReportRepository(db).Involved("int-1"); len(involved) != 1 || involved[0] != bea {
		t.Fatalf("quien comenta es del hilo: %v", involved)
	}
}

// Por la puerta de reportes una línea de sistema ya era inmutable para todos
// (`ErrCommentImmutable`); que ahora pueda llevar autor no la hace de nadie.
//
// Rotura que atrapa: quitar esa negativa de editar o de borrar, que antes no
// se notaba porque sin autor tampoco casaba con nadie, y ahora sí casaría.
func TestTheReportsDoorKeepsAnAuthoredSystemLineImmutable(t *testing.T) {
	db, cleanup := statusTraceDB(t)
	defer cleanup()
	svc := reportSvcDePrueba(db, &notificadorEspia{})
	author := ana
	line := &domain.ItemComment{ItemID: "item-1", Kind: domain.CommentKindSystem, Visibility: domain.VisibilityInternal, Body: "status: a → b", AuthorUserID: &author}
	line.ID = "sys-1"
	if err := db.Create(line).Error; err != nil {
		t.Fatal(err)
	}
	body := "otra cosa"
	if _, err := svc.EditComment(context.Background(), ana, "item-1", "sys-1", CommentEdit{Body: &body}); !errors.Is(err, ErrCommentImmutable) {
		t.Errorf("editar la línea de su cambio: %v", err)
	}
	if err := svc.DeleteComment(ana, "item-1", "sys-1"); !errors.Is(err, ErrCommentImmutable) {
		t.Errorf("borrar la línea de su cambio: %v", err)
	}
	if rows := threadRows(t, db, "item-1"); len(rows) != 1 || rows[0].Body != "status: a → b" {
		t.Fatalf("la línea sigue como estaba: %+v", rows)
	}
}
