package domain

import "testing"

// La ontología es cerrada: cada relación dice qué une, y lo que no está no se
// guarda (command-and-control-348).
//
// Roturas que atrapa: una relación sin sus extremos (Allows diría que no a
// todo y se perdería en silencio); aceptar una arista al revés (una tarea que
// «se aplica» a una decisión); que un enlace a otra cosa o un id a medias
// cuente como enlace; repetir una cosa nombrada dos veces.
func TestEveryRelationIsDeclared(t *testing.T) {
	for _, r := range []Relation{RelMentions, RelRelatesTo, RelAppliesTo, RelOrigin, RelBlocks, RelParentOf, RelSupersedes, RelBitOn} {
		if ends, ok := RelationEnds[r]; !ok || len(ends.From) == 0 || len(ends.To) == 0 {
			t.Errorf("%s no dice qué une", r)
		}
	}
	if !RelAppliesTo.Allows(EntityDecision, EntityTask) || RelAppliesTo.Allows(EntityTask, EntityDecision) {
		t.Error("applies_to va de una decisión a lo que toca, no al revés")
	}
	if !RelMentions.Allows(EntityPage, EntityTask) || RelMentions.Allows(EntityPage, EntityServer) {
		t.Error("un texto nombra tareas, páginas, decisiones e incidentes; no servidores")
	}
	if Relation("inventada").Allows(EntityTask, EntityTask) {
		t.Error("una relación que no está declarada se acepta")
	}
	got := ExtractEntityLinks("[a](/tasks?task=00000000-0000-0000-0000-00000000000a) " +
		"[b](/tasks?doc=list:x&page=00000000-0000-0000-0000-00000000000b) otra vez /tasks?task=00000000-0000-0000-0000-00000000000a " +
		"/notes/00000000-0000-0000-0000-00000000000c ?task=0000-corto")
	if len(got) != 2 || got[0] != (EntityRef{EntityTask, "00000000-0000-0000-0000-00000000000a"}) ||
		got[1] != (EntityRef{EntityPage, "00000000-0000-0000-0000-00000000000b"}) {
		t.Errorf("enlaces: %+v", got)
	}
}
