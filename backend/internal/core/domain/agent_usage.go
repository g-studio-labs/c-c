package domain

import (
	"regexp"
	"strings"
	"time"
)

// AgentCall: una llamada de un agente a una herramienta del MCP de cac, o
// algo que un hook le puso delante (`Tool` = `hook:cac-prompt`…).
//
// Para decidir con datos (command-and-control-347): hasta el 11-nov-2026 lo
// único que se medía eran las búsquedas fallidas, y no se sabía qué
// herramientas usan los agentes ni si lo que los hooks ponen delante se lee
// después. Lo segundo es lo que dice si un hook ayuda o es ruido: un
// incidente enseñado y luego abierto con `get_incident` sirvió.
//
// Una fila por llamada, no por petición: una herramienta hace varias (reclamar,
// parchear, releer), y todas llevan el mismo `CallID`, que es único. Se poda
// a los 90 días, como las búsquedas fallidas. Es una declaración del propio
// MCP, como `Via`: sirve para contar, no prueba nada.
type AgentCall struct {
	ID     string `gorm:"type:varchar(36);primaryKey" json:"id"`
	CallID string `gorm:"type:varchar(64);uniqueIndex;not null" json:"callId"`
	UserID string `gorm:"type:varchar(36);index;not null" json:"userId"`
	Tool   string `gorm:"type:varchar(60);index;not null" json:"tool"`
	// Target: lo que tocó (el id o la ref que se le pasó), para cruzar lo
	// enseñado con lo abierto.
	Target    string    `gorm:"type:varchar(80);index" json:"target,omitempty"`
	CreatedAt time.Time `gorm:"index" json:"createdAt"`
}

// Las cabeceras con las que el MCP declara qué herramienta hace la petición.
const (
	HeaderAgentTool   = "X-Cac-Tool"
	HeaderAgentCall   = "X-Cac-Call"
	HeaderAgentTarget = "X-Cac-Target"
)

// AgentCallRetention: lo que se guarda; lo de antes se poda.
const AgentCallRetention = 90 * 24 * time.Hour

// AgentShownWindow: cuánto después de enseñar algo cuenta abrirlo como «lo
// usó». Un prompt y el trabajo que dispara caben de sobra en dos horas.
const AgentShownWindow = 2 * time.Hour

// AgentHookPrefix marca lo que puso delante un hook, no una herramienta.
const AgentHookPrefix = "hook:"

var agentToolName = regexp.MustCompile(`^[a-z][a-z0-9_:-]{0,59}$`)
var agentTarget = regexp.MustCompile(`^[A-Za-z0-9_.:-]{1,80}$`)

// NormalizeAgentTool: un nombre de herramienta o de hook, o vacío si no lo es.
// Lista abierta a propósito (el catálogo crece), pero con forma: nada de texto
// libre en una tabla que se agrupa.
func NormalizeAgentTool(s string) string {
	s = strings.TrimSpace(s)
	if !agentToolName.MatchString(s) {
		return ""
	}
	return s
}

// NormalizeAgentTarget: un id o una ref; lo demás no se guarda.
func NormalizeAgentTarget(s string) string {
	s = strings.TrimSpace(s)
	if !agentTarget.MatchString(s) {
		return ""
	}
	return s
}

// AgentShownRequest: lo que un hook enseñó en una sesión.
type AgentShownRequest struct {
	Hook    string   `json:"hook"`
	Targets []string `json:"targets"`
}

// AgentShownMax: lo más que un hook enseña de una vez; lo demás se ignora.
const AgentShownMax = 20

// AgentToolUsage: cuántas veces se llamó una herramienta y cuántas personas.
type AgentToolUsage struct {
	Tool   string    `json:"tool"`
	Calls  int       `json:"calls"`
	Users  int       `json:"users"`
	LastAt time.Time `json:"lastAt"`
}

// AgentShownUsage: de lo que un hook enseñó, cuánto se abrió después
// (`get_incident`, `get_task`… con ese mismo id o ref, por la misma persona,
// dentro de AgentShownWindow).
type AgentShownUsage struct {
	Hook   string `json:"hook"`
	Shown  int    `json:"shown"`
	Opened int    `json:"opened"`
}

// AgentUsage: la lectura entera.
type AgentUsage struct {
	Since time.Time         `json:"since"`
	Tools []AgentToolUsage  `json:"tools"`
	Shown []AgentShownUsage `json:"shown"`
}
