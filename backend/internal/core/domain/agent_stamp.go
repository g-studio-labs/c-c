package domain

import (
	"context"
	"strings"
)

// AgentStamp dice qué agente escribió algo con el token personal de su dueño:
// su rol (`builder`, `reviewer`), su modelo y su sesión. Lo manda el MCP
// cuando lo sabe; todo es opcional.
//
// Como `Via`, no es seguridad: quien tiene el token puede mandar lo que
// quiera. Lo que sí no se puede fingir es lo contrario —que lo escribió la
// persona—, porque eso lo decide `ViaToken`, que sólo pone el middleware.
type AgentStamp struct {
	Role    string `gorm:"type:varchar(40)" json:"role,omitempty"    validate:"omitempty,max=40"`
	Model   string `gorm:"type:varchar(60)" json:"model,omitempty"   validate:"omitempty,max=60"`
	Session string `gorm:"type:varchar(80)" json:"session,omitempty" validate:"omitempty,max=80"`
}

// AgentStampFor: con qué se firma lo que escribe quien llama. Nil es la
// persona: una sesión de la app, aunque mande `agent`, firma como ella (un
// cliente no se declara agente para que su texto parezca de otro). Con un
// token personal nunca es nil, aunque el cliente no diga qué agente es: hasta
// command-and-control-522 un handoff escrito por un agente salía firmado con
// el nombre de su dueño, como si lo hubiera escrito él.
func AgentStampFor(c *ClaimsJWT, sent *AgentStamp) *AgentStamp {
	if c == nil || !c.ViaToken {
		return nil
	}
	if sent == nil {
		return &AgentStamp{}
	}
	return &AgentStamp{
		Role:    strings.TrimSpace(sent.Role),
		Model:   strings.TrimSpace(sent.Model),
		Session: strings.TrimSpace(sent.Session),
	}
}

// AgentSignature es cómo se contesta: el agente de quién (`of`, el nombre
// visible del dueño del token) y, si lo dijo, cuál. Su presencia es lo que
// dice «lo escribió un agente»; quien pinta forma con esto «agente de Ana
// (builder, sonnet)».
type AgentSignature struct {
	Of      string `json:"of"`
	Role    string `json:"role,omitempty"`
	Model   string `json:"model,omitempty"`
	Session string `json:"session,omitempty"`
}

// SignedByAgent: la firma de algo guardado con `via`, o nil si lo escribió la
// persona.
func SignedByAgent(via, of string, a AgentStamp) *AgentSignature {
	if via != ViaMCP {
		return nil
	}
	return &AgentSignature{Of: of, Role: a.Role, Model: a.Model, Session: a.Session}
}

type agentKey struct{}

// WithAgent lleva la firma del agente hasta el servicio, como WithVia lleva
// por dónde entró. Nil no cambia nada.
func WithAgent(ctx context.Context, a *AgentStamp) context.Context {
	if a == nil {
		return ctx
	}
	return context.WithValue(ctx, agentKey{}, a)
}

// AgentFrom: el agente que escribe, o nil si es la persona.
func AgentFrom(ctx context.Context) *AgentStamp {
	if ctx == nil {
		return nil
	}
	a, _ := ctx.Value(agentKey{}).(*AgentStamp)
	return a
}
