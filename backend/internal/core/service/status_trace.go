package service

import (
	"fmt"

	"github.com/google/uuid"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

// newStatusTrace: la línea que deja en el hilo un cambio de estado, para
// quien corresponda leerla.
//
// Hasta el 17-nov-2026 sólo la dejaba una tarea que un cliente ve; en una
// interna el estado cambiaba sin rastro. Lo decidió jose ese día: «creo que es
// lo correcto para que quede a modo de logs, y quién lo hizo sería un plus
// útil en x casos, o en caso de que más users se unan a mi forma de trabajar»
// (command-and-control-383).
//
// Son dos líneas distintas según quién lee, y por eso se decide aquí y no en
// cada llamante (los dos caminos que cambian un estado, el tablero y la
// pantalla de reportes, tienen que dejar lo mismo):
//
//   - **Tarea interna**: línea interna, con quien lo hizo como autor de la
//     fila. `actorUserID` vacío es «no fue una persona» (un tenant por su
//     llave, una automatización) y la línea va sin autor.
//   - **Tarea que ve un cliente**: la pública de siempre, tal cual y **sin
//     autor**. El cliente no tiene por qué saber qué compañero la movió, y lo
//     que su API devuelve de un comentario incluye `authorUserId` y
//     `authorName`: una línea pública con autor se los enseñaría. Tampoco se
//     añade una segunda línea interna con el quién: serían dos renglones por
//     cada cambio en el mismo hilo.
//
// El texto es el mismo en las dos (`status: <antes> → <después>`): las apps lo
// pintan como llega. Quién lo hizo viaja como autor, no dentro del texto.
// `via` es por dónde entró (`domain.ViaFrom`): distingue lo que movió una
// persona de lo que movió un agente con su token.
func newStatusTrace(item *domain.Item, actorUserID, via string, from, to domain.ReportStatus) *domain.ItemComment {
	body := fmt.Sprintf("status: %s → %s", from, to)
	if item.IsVisibleToChannel() {
		return newSystemComment(item.ID, body)
	}
	c := &domain.ItemComment{
		ItemID:     item.ID,
		Kind:       domain.CommentKindSystem,
		Visibility: domain.VisibilityInternal,
		Body:       body,
	}
	c.ID = uuid.NewString()
	if actorUserID != "" {
		c.AuthorUserID = &actorUserID
		c.Via = via
	}
	return c
}
