package domain

import "time"

// Compartir una página de documentación (con sus subpáginas) con alguien sin
// cuenta, por un enlace firmado que caduca y se revoca. Calca «Llamar con
// invitados» (call_invite.go): el enlace es `<id>.<firma>` y la vida del enlace
// vive en la fila, no en el token.
//
// Lo que abre es **el subárbol en vivo**: lo que se cuelgue debajo después de
// compartir también se ve, y lo que se tire a la papelera deja de verse. Lo
// decidió jose (command-and-control-496).

// DocShare es un enlace que abre una página y lo que cuelga de ella.
type DocShare struct {
	BaseModel
	OrgID     string `gorm:"type:varchar(36);index;not null" json:"orgId"`
	DocID     string `gorm:"type:varchar(36);index;not null" json:"docId"`
	PageID    string `gorm:"type:varchar(36);index;not null" json:"pageId"`
	CreatedBy string `gorm:"type:varchar(36);not null" json:"createdBy"`

	// Como en CallInvite: revocar es una escritura y no una lista negra, y el
	// enlace se puede volver a copiar sin guardar ningún secreto.
	ExpiresAt time.Time  `gorm:"index;not null" json:"expiresAt"`
	RevokedAt *time.Time `json:"revokedAt,omitempty"`
}

func (DocShare) TableName() string { return "doc_shares" }

// Live: si el enlace todavía abre algo.
func (s DocShare) Live(now time.Time) bool {
	return s.RevokedAt == nil && now.Before(s.ExpiresAt)
}

// ValidDocShareTTL: 1, 7 o 30 días, y nada más. Lo que no sea eso es un 400:
// el cliente ofrece esas tres, y un número libre dejaría colar un enlace de
// diez años.
func ValidDocShareTTL(hours int) bool {
	switch hours {
	case 24, 168, 720:
		return true
	}
	return false
}

// ─── Peticiones y respuestas ─────────────────────────────────────────────────

type CreateDocShareRequest struct {
	TTLHours int `json:"ttlHours"`
}

// DocShareResponse es el enlace para un miembro.
type DocShareResponse struct {
	ID            string    `json:"id"`
	PageID        string    `json:"pageId"`
	ExpiresAt     time.Time `json:"expiresAt"`
	CreatedBy     string    `json:"createdBy"`
	CreatedByName string    `json:"createdByName"`
	// Link es el token, no una URL: dónde se abre lo decide el cliente.
	Link string `json:"link"`
}

// OrgDocShareResponse: un enlace vivo en la lista de toda la organización
// (command-and-control-577), para tenerlos controlados desde un sitio. Lleva
// lo de la lista por página y, además, de dónde cuelga.
//
// `Mode` y `ResponseCount` existen antes que las respuestas a propósito: hasta
// que lleguen salen `read` y `0`, y así la app nativa no cambia de contrato
// después.
type OrgDocShareResponse struct {
	ID            string       `json:"id"`
	PageID        string       `json:"pageId"`
	PageTitle     string       `json:"pageTitle"`
	DocKind       DocOwnerKind `json:"docKind"`
	OwnerID       string       `json:"ownerId"`
	OwnerName     string       `json:"ownerName"`
	SpaceID       string       `json:"spaceId"`
	SpaceName     string       `json:"spaceName"`
	ExpiresAt     time.Time    `json:"expiresAt"`
	CreatedBy     string       `json:"createdBy"`
	CreatedByName string       `json:"createdByName"`
	Mode          string       `json:"mode"`
	ResponseCount int          `json:"responseCount"`
	Link          string       `json:"link"`
}

// DocShareModeRead: el enlace sólo deja leer, que hoy es lo único que hay.
const DocShareModeRead = "read"

// PublicSharedDoc es lo que ve alguien de fuera.
//
// **Sin un solo id de organización, de doc ni de persona**, como
// PublicCallInvite. Los de las páginas sí van: hacen falta para pintar el árbol
// y no dicen nada que el propio enlace no abra ya.
type PublicSharedDoc struct {
	Title     string             `json:"title"`
	ExpiresAt time.Time          `json:"expiresAt"`
	Pages     []PublicSharedPage `json:"pages"`
}

// PublicSharedPage: una página del árbol compartido. La raíz va con
// `parentId: null` aunque en su documento cuelgue de otra: de fuera, el árbol
// empieza en ella.
type PublicSharedPage struct {
	ID       string  `json:"id"`
	ParentID *string `json:"parentId"`
	Title    string  `json:"title"`
	Body     string  `json:"body"`
}
