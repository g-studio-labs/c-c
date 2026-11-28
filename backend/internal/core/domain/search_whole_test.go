package domain

import "testing"

// Desde el MCP, una palabra final de una o dos letras va entera, como las
// demás; la paleta, que se escribe en vivo, la sigue buscando por prefijo
// (command-and-control-570).
//
// Roturas que atrapa: quitarle el prefijo también a la paleta, dejárselo al
// MCP, o quitárselo a una palabra larga o a una referencia.
func TestFromAnAgentAShortLastWordIsWhole(t *testing.T) {
	for q, want := range map[string]string{
		"a b":             "a & b",
		"puerta d":        "puerta:* & d",
		"ci":              "ci",
		"despliegue":      "despliegue:*",
		"de despliegue":   "de & despliegue:*",
		"tds-69":          "tds-69",
		"ver tds-69 y ok": "ver:* & tds-69 & y & ok",
	} {
		if got := SearchTSQueryWhole(q, nil, false); got != want {
			t.Errorf("MCP %q = %q, se esperaba %q", q, got, want)
		}
	}
	if got := SearchTSQueryWhole("a b", nil, true); got != "a | b" {
		t.Errorf("la segunda vuelta del MCP %q", got)
	}
	for q, want := range map[string]string{"a b": "a & b:*", "puerta d": "puerta:* & d:*", "ci": "ci:*"} {
		if got := SearchTSQuery(q); got != want {
			t.Errorf("la paleta %q = %q, se esperaba %q", q, got, want)
		}
	}
}
