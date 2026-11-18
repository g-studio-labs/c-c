package domain

import "testing"

// Dos máquinas, y cuál gobierna cada ficha.
//
// La estricta protege lo que un cliente ve. La interna es un tablero corriente.
// Confundirlas tiene consecuencias en las dos direcciones: aplicar la estricta a
// una tarea interna dejó el check de las subtareas sin hacer nada —Open → Done no
// existe— y aplicar la interna a un ticket de cliente le dejaría saltar de
// recién recibido a resuelto sin que nadie lo tocara.
func TestQueMaquinaGobierna(t *testing.T) {
	proyecto := "proj-1"

	t.Run("una tarea levantada aquí es interna", func(t *testing.T) {
		var i Item
		if got := i.Flow(); got != FlowInternal {
			t.Fatalf("%q", got)
		}
	})

	t.Run("una atada a un canal de cliente, no", func(t *testing.T) {
		i := Item{ProjectID: proyecto}
		if got := i.Flow(); got != FlowClient {
			t.Fatalf("%q", got)
		}
	})

	// El caso que separa «tiene canal» de «lo ve el cliente»: una nota interna en
	// una lista de cliente sigue siendo asunto nuestro, y se mueve como tal.
	t.Run("marcada como interna, aunque esté en una lista de cliente", func(t *testing.T) {
		i := Item{ProjectID: proyecto, Visibility: VisibilityInternal}
		if got := i.Flow(); got != FlowInternal {
			t.Fatalf("%q", got)
		}
	})
}

func TestTransicionesPorFlujo(t *testing.T) {
	// El fallo que dio origen a esto: marcar una subtarea abierta como hecha.
	t.Run("una tarea interna va de Open a Done", func(t *testing.T) {
		if !CanTransition(FlowInternal, ReportPending, ReportResolved) {
			t.Fatal("el check de las subtareas hace exactamente esto")
		}
	})

	t.Run("y un ticket de cliente no", func(t *testing.T) {
		if CanTransition(FlowClient, ReportPending, ReportResolved) {
			t.Fatal("un ticket recién recibido no puede salir resuelto sin tocarlo")
		}
	})

	/**
	 * Cerrado es terminal para un cliente y no para nosotros.
	 *
	 * Reabrir algo que ya se le comunicó como cerrado es una promesa rota. Cerrar
	 * una tarea interna por error y no poder deshacerlo sería un fallo, no una
	 * garantía.
	 */
	t.Run("cerrado se reabre en interno, y no en cliente", func(t *testing.T) {
		if !CanTransition(FlowInternal, ReportClosed, ReportPending) {
			t.Fatal("cerrar por error tiene que tener vuelta atrás")
		}
		if CanTransition(FlowClient, ReportClosed, ReportPending) {
			t.Fatal("cerrado es lo que se le dijo al cliente")
		}
	})

	// Quedarse donde se está no es moverse. Sin esto, soltar una tarjeta en su
	// propia columna se rechazaría.
	t.Run("no moverse siempre vale", func(t *testing.T) {
		for _, f := range []ItemFlow{FlowClient, FlowInternal} {
			if !CanTransition(f, ReportClosed, ReportClosed) {
				t.Fatalf("%q: quedarse quieto se rechazó", f)
			}
		}
	})

	// Los dos vocabularios conviven mientras haya apps instaladas viejas: «open»
	// es `pending` y «done» es `resolved`.
	t.Run("los alias viejos se pliegan igual", func(t *testing.T) {
		if !CanTransition(FlowInternal, "open", "done") {
			t.Fatal("una app anterior al renombrado manda estos nombres")
		}
	})

	t.Run("el flujo desconocido cae en el estricto", func(t *testing.T) {
		// Proteger de más nunca pierde datos de nadie; proteger de menos sí.
		if CanTransition(ItemFlow("vete a saber"), ReportPending, ReportResolved) {
			t.Fatal("ante la duda, la máquina que protege al cliente")
		}
	})
}

// Una subtarea nunca la gobierna la máquina del cliente.
//
// El servicio le borra el proyecto al crearla —para no gastar un folio del
// cliente en una línea de checklist— y de eso depende algo que no se ve desde
// aquí: el check de «hecho» del panel salta directo de Open a Done, que sólo es
// legal en el flujo interno.
//
// Se fija aquí porque el día que alguien deje de borrar ese proyecto, ese botón
// volverá a no hacer nada y a no decir por qué. Es el fallo del que sale todo
// esto, y no dejó rastro en ninguna prueba la primera vez.
func TestUnaSubtareaEsInterna(t *testing.T) {
	padre := Item{ProjectID: "proj-1"}
	if padre.Flow() != FlowClient {
		t.Fatal("el padre sí lo ve el cliente; si no, la prueba no prueba nada")
	}

	// Como la crea `CreateTask` cuando llega con `parentID`: sin proyecto.
	sub := Item{ProjectID: "", ParentID: &padre.ID}
	if got := sub.Flow(); got != FlowInternal {
		t.Fatalf("una subtarea con la máquina del cliente deja el check muerto: %q", got)
	}
	if !CanTransition(sub.Flow(), ReportPending, ReportResolved) {
		t.Fatal("marcar hecha una subtarea abierta es exactamente esto")
	}
}

// Un padre no se termina ni se cierra con subtareas abiertas
// (command-and-control-387, decidido por jose el 17-nov-2026).
//
// Hasta entonces era una opción de la organización, sólo para «hecho», y
// cerrar era la salida. Ahora vale para los dos y para todos: lo que quedaba
// abierto debajo de un padre cerrado dejaba de verse en la vista principal.
//
// Roturas que atrapa: dejar pasar «cerrado» (la regla de antes); dejar pasar
// «hecho»; impedir reabrir; impedir reordenar dentro de la columna en la que
// ya está; bloquear a quien no tiene ninguna abierta; y que una app vieja se
// la salte mandando «done».
func TestOpenSubtasksBlockDoneAndClosed(t *testing.T) {
	for _, to := range []ReportStatus{ReportResolved, ReportClosed} {
		for _, from := range []ReportStatus{ReportPending, ReportInProgress} {
			if !OpenSubtasksBlock(from, to, 2) {
				t.Fatalf("%s → %s con dos subtareas abiertas tiene que rechazarse", from, to)
			}
			if OpenSubtasksBlock(from, to, 0) {
				t.Fatalf("%s → %s sin ninguna abierta no tiene nada que esperar", from, to)
			}
		}
	}
	// Entre los dos estados terminados también: de hecha a cerrada es cerrar.
	if !OpenSubtasksBlock(ReportResolved, ReportClosed, 1) || !OpenSubtasksBlock(ReportClosed, ReportResolved, 1) {
		t.Fatal("pasar de hecha a cerrada (o al revés) con una abierta también es terminarla")
	}
	// Reabrir siempre se puede: es como se sale de un padre que quedó
	// terminado con algo abierto debajo.
	for _, from := range []ReportStatus{ReportResolved, ReportClosed} {
		for _, to := range []ReportStatus{ReportPending, ReportInProgress} {
			if OpenSubtasksBlock(from, to, 3) {
				t.Fatalf("reabrir (%s → %s) no se impide nunca", from, to)
			}
		}
	}
	// Reordenar dentro de su columna no es un cambio de estado.
	if OpenSubtasksBlock(ReportResolved, ReportResolved, 3) || OpenSubtasksBlock(ReportClosed, ReportClosed, 3) {
		t.Fatal("soltar la tarjeta en la columna en la que ya está no termina nada")
	}
	// Los dos vocabularios conviven: una app anterior al renombrado manda
	// «done» donde ahora se dice `resolved`.
	if !OpenSubtasksBlock("open", "done", 1) {
		t.Fatal("una app sin actualizar se saltaría la regla entera")
	}
	if OpenSubtasksBlock("done", ReportResolved, 1) {
		t.Fatal("«done» y `resolved` son el mismo estado: no hay cambio")
	}
}

// La frase del rechazo dice qué se quiso hacer, cuántas quedan y qué hacer.
//
// Roturas que atrapa: una frase sin la cuenta; decir «close» a quien quiso
// marcarla hecha (o al revés); «1 subtasks».
func TestOpenSubtasksReasonNamesTheActAndTheCount(t *testing.T) {
	cases := []struct {
		to   ReportStatus
		open int64
		want string
	}{
		{ReportClosed, 2, "cannot close this task: 2 subtasks are still open — finish or close them first"},
		{ReportResolved, 2, "cannot mark this task done: 2 subtasks are still open — finish or close them first"},
		{"done", 1, "cannot mark this task done: 1 subtask is still open — finish or close it first"},
		{ReportClosed, 1, "cannot close this task: 1 subtask is still open — finish or close it first"},
	}
	for _, c := range cases {
		if got := OpenSubtasksReason(c.to, c.open); got != c.want {
			t.Fatalf("%s con %d: %q", c.to, c.open, got)
		}
	}
}
