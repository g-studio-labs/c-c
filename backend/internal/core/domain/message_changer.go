package domain

import "context"

// MessageChanger es quien corrige o retira un mensaje de un canal o de un
// directo, con lo que hace falta para decidir si puede (command-and-control-547).
//
// Un struct y no tres booleanos sueltos: `superadmin` y `viaToken` son del
// mismo tipo, van juntos en cada llamada y cambiados de sitio compilan.
type MessageChanger struct {
	UserID     string
	Superadmin bool
	// ViaToken: entró con un token personal (sólo lo pone el middleware). Un
	// token **es** la persona para los scopes, pero no para saltarse la
	// autoría: sus claims copian `Superadmin`, y sin esto el token de un
	// superadmin corregiría lo que escribió cualquiera.
	ViaToken bool
	// Via: por dónde dice que entró (ver via.go).
	Via string
}

// ChangerFrom arma quién cambia con las claims y el contexto de la petición.
func ChangerFrom(ctx context.Context, c *ClaimsJWT) MessageChanger {
	if c == nil {
		return MessageChanger{Via: ViaFrom(ctx)}
	}
	return MessageChanger{UserID: c.UserID, Superadmin: c.Superadmin, ViaToken: c.ViaToken, Via: ViaFrom(ctx)}
}

// MayChangeOthers: sólo un superadmin **en persona** toca palabras ajenas.
func (c MessageChanger) MayChangeOthers() bool {
	return c.Superadmin && !c.ViaToken
}

// AsAgent: lo que cambia lo cambia un agente. Con un token siempre, aunque el
// cliente no mande la marca: como en AgentStampFor, lo contrario —que lo hizo
// la persona— no se puede fingir.
func (c MessageChanger) AsAgent() bool {
	return c.ViaToken || NormalizeVia(c.Via) == ViaMCP
}

// WrittenVia: con qué marca queda el texto que esta persona deja.
func (c MessageChanger) WrittenVia() string {
	if c.AsAgent() {
		return ViaMCP
	}
	return ViaApp
}

// MayWithdraw: un token sólo retira lo que **creó** un agente (`created_via
// = mcp`), nunca lo que la persona tecleó en la app, aunque después lo haya
// corregido un agente. La autoría se comprueba aparte.
func (c MessageChanger) MayWithdraw(createdVia string) bool {
	return !c.ViaToken || createdVia == ViaMCP
}

// Las clases del aviso «editó su mensaje». Una por familia y no una sola:
// la campana pliega por familia (`chat:` y `dm:`, ver DeriveGroup) y las
// preferencias callan un directo con lo de los directos.
const (
	MessageEditedChat = "chat:edited"
	MessageEditedDM   = "dm:edited"
)
