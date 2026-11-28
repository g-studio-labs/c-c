package domain

import "strings"

// SearchOptions: qué fuentes contesta una búsqueda, además de tareas, notas y
// docs, que van siempre.
//
// Personas, canal y directos se buscan con `LOWER(body) LIKE '%x%'`, que no
// usa índice, y al MCP no le llegan: el servidor MCP los tira
// (`search_results` en `mcp/src/server.rs`). Calcularlos para un agente era
// trabajo para nada en cada org que recorría (command-and-control-570).
type SearchOptions struct {
	People, Messages, DMs bool
}

// SearchForPeople: la paleta de la app, con todo.
func SearchForPeople() SearchOptions {
	return SearchOptions{People: true, Messages: true, DMs: true}
}

// SearchForAgent: lo que le llega a un agente por el MCP.
func SearchForAgent() SearchOptions { return SearchOptions{} }

// WithKinds: las fuentes que nombra `kinds` (`tasks,notes,docs,people,
// messages,dms`, separadas por comas). Tareas, notas y docs van siempre, así
// que sólo cuenta si nombra las otras tres.
func (o SearchOptions) WithKinds(kinds string) SearchOptions {
	o.People, o.Messages, o.DMs = false, false, false
	for _, k := range strings.Split(kinds, ",") {
		switch strings.ToLower(strings.TrimSpace(k)) {
		case "people":
			o.People = true
		case "messages":
			o.Messages = true
		case "dms":
			o.DMs = true
		}
	}
	return o
}
