package domain

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"html"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// La integración con Basecamp (command-and-control-516, fase 1 en 553).
//
// Una organización conecta **una** cuenta de Basecamp por OAuth (Launchpad de
// 37signals). En un espacio se eligen proyectos de esa cuenta: cada uno recibe
// un webhook cuya dirección lleva un secreto propio, porque Basecamp no firma lo
// que manda. Lo que llega (un comentario, un mensaje, un to-do completado) se
// apunta en la tarea de cac enlazada a ese recording y avisa a quien toca; las
// personas de Basecamp se casan con usuarios de cac por correo, y las que no
// casan se quedan para que un admin las asigne.

// Los tipos de recording a los que se suscribe cada webhook.
var BasecampWebhookTypes = []string{"Comment", "Message", "Todo", "Kanban::Card"}

// BasecampLegacyWebhookTypes: con qué tipos se dieron de alta los webhooks de
// la fase 1, que no lo apuntaron. Un proyecto sin `webhook_types` se lee así;
// si BasecampWebhookTypes cambia, el reloj de fondo los pone al día
// (BasecampService.ReconcileWebhooks), sin crear otro webhook.
var BasecampLegacyWebhookTypes = []string{"Comment", "Message", "Todo", "Kanban::Card"}

// BasecampTypesKey: cómo se guardan y se comparan los tipos de un webhook.
func BasecampTypesKey(types []string) string { return strings.Join(types, ",") }

// Las clases de aviso de Basecamp (ver NotificationPrefs.Allows).
const (
	// BasecampMentionKind: te nombraron en Basecamp. No se calla, como una
	// mención del chat: alguien te está hablando a ti.
	BasecampMentionKind = "basecamp:mention"
	// BasecampActivityKind: algo pasó en un proyecto y no hay tarea enlazada.
	BasecampActivityKind = "basecamp:activity"
	// BasecampAuthKind: la conexión dejó de renovarse. Para los admins, y no se
	// calla: sin ella la integración se queda muda.
	BasecampAuthKind = "basecamp:auth"
)

// AnsweredViaBasecamp: el pedido lo contestó un comentario en Basecamp.
const AnsweredViaBasecamp = "basecamp"

var (
	ErrBasecampOff          = errors.New("basecamp-not-configured")
	ErrBasecampNotConnected = errors.New("basecamp-not-connected")
	ErrBasecampBadState     = errors.New("basecamp-bad-state")
	ErrBasecampNoAccount    = errors.New("basecamp-no-account")
	ErrBasecampUnreachable  = errors.New("basecamp-unreachable")
	// Launchpad contestó y dijo que no (el refresh o el code ya no valen):
	// no es un silencio, y lo que arregla es conectar de nuevo.
	ErrBasecampReconnect   = errors.New("basecamp-reconnect")
	ErrBasecampBadURL      = errors.New("basecamp-bad-url")
	ErrBasecampLinkTaken   = errors.New("basecamp-link-taken")
	ErrBasecampUserOutside = errors.New("basecamp-user-outside-org")
)

// BasecampConnection: la cuenta de Basecamp de una organización. Una por org.
//
// Los tokens van cifrados con la misma llave que los secretos de las
// integraciones (`EncryptTelemetry`) y nunca salen en JSON.
type BasecampConnection struct {
	BaseModel
	OrgID       string `gorm:"type:varchar(36);uniqueIndex;not null" json:"orgId"`
	AccountID   int64  `gorm:"not null" json:"accountId"`
	AccountName string `gorm:"type:varchar(200)" json:"accountName"`
	// IdentityName: quién autorizó en Basecamp (su nombre allí).
	IdentityName  string `gorm:"type:varchar(200)" json:"identityName"`
	ConnectedByID string `gorm:"type:varchar(36)" json:"connectedById"`
	AccessToken   []byte `json:"-"`
	RefreshToken  []byte `json:"-"`
	ExpiresAt     time.Time
	// RefreshFailedAt: la última renovación falló. Nil: está sana. Es lo que
	// impide avisar a los admins cada hora del mismo fallo.
	RefreshFailedAt *time.Time `json:"refreshFailedAt,omitempty"`
}

func (BasecampConnection) TableName() string { return "basecamp_connections" }

// BasecampProject: un proyecto de Basecamp enlazado a un espacio de cac, con
// su webhook.
//
// Del secreto de la dirección sólo se guarda el SHA-256: con la base en la
// mano no se puede mandar nada a nombre de Basecamp.
type BasecampProject struct {
	BaseModel
	OrgID      string `gorm:"type:varchar(36);not null;uniqueIndex:idx_basecamp_project_org,priority:1" json:"orgId"`
	SpaceID    string `gorm:"type:varchar(36);index;not null" json:"spaceId"`
	ProjectID  int64  `gorm:"not null;uniqueIndex:idx_basecamp_project_org,priority:2" json:"projectId"`
	Name       string `gorm:"type:varchar(200)" json:"name"`
	AppURL     string `gorm:"type:text" json:"appUrl"`
	WebhookID  int64  `json:"webhookId"`
	SecretHash string `gorm:"type:varchar(64);uniqueIndex;not null" json:"-"`
	// LinkedByID: quien lo enlazó. Lo que pasa sin tarea enlazada ni mención
	// le llega a él: sin alguien a quien avisar, se perdería en silencio.
	LinkedByID string `gorm:"type:varchar(36)" json:"linkedById"`
	// WebhookTypes: los tipos con que quedó dado de alta el webhook
	// (BasecampTypesKey). Vacío: los de la fase 1.
	WebhookTypes string `gorm:"type:varchar(300);not null;default:''" json:"-"`
}

func (BasecampProject) TableName() string { return "basecamp_projects" }

// BasecampPerson: una persona de la cuenta de Basecamp y, si casó, su usuario
// de cac. UserID vacío: queda para que un admin la asigne.
type BasecampPerson struct {
	BaseModel
	OrgID    string `gorm:"type:varchar(36);not null;uniqueIndex:idx_basecamp_person,priority:1" json:"-"`
	PersonID int64  `gorm:"not null;uniqueIndex:idx_basecamp_person,priority:2" json:"personId"`
	SGID     string `gorm:"column:sgid;type:text" json:"-"`
	Name     string `gorm:"type:varchar(200)" json:"name"`
	Email    string `gorm:"type:varchar(255)" json:"email"`
	// Client: es de la empresa cliente (Basecamp lo marca). Lo normal es que
	// no case con nadie del equipo, y la app lo enseña aparte al asignar.
	Client bool   `json:"client"`
	UserID string `gorm:"type:varchar(36);index" json:"userId"`
	// MatchedBy: `email` (automático) o `manual` (un admin). Uno manual no lo
	// pisa una sincronización.
	MatchedBy string `gorm:"type:varchar(10)" json:"matchedBy,omitempty"`
}

func (BasecampPerson) TableName() string { return "basecamp_people" }

// BasecampLink: un recording de Basecamp (to-do, mensaje, tarjeta) enlazado a
// una tarea de cac. Un recording, una tarea.
type BasecampLink struct {
	BaseModel
	OrgID       string `gorm:"type:varchar(36);not null;uniqueIndex:idx_basecamp_link,priority:1" json:"-"`
	RecordingID int64  `gorm:"not null;uniqueIndex:idx_basecamp_link,priority:2" json:"recordingId"`
	ItemID      string `gorm:"type:varchar(36);index;not null" json:"taskId"`
	ProjectID   int64  `json:"projectId"`
	AppURL      string `gorm:"type:text" json:"appUrl"`
}

func (BasecampLink) TableName() string { return "basecamp_links" }

// BasecampAssignment: un responsable que puso Basecamp en una tarea. Sólo
// éstos los quita Basecamp cuando el to-do deja de estar asignado a esa
// persona; a quien puso el equipo no lo toca.
type BasecampAssignment struct {
	ItemID    string    `gorm:"type:varchar(36);primaryKey"`
	UserID    string    `gorm:"type:varchar(36);primaryKey"`
	OrgID     string    `gorm:"type:varchar(36);index;not null"`
	CreatedAt time.Time `gorm:"not null"`
}

func (BasecampAssignment) TableName() string { return "basecamp_assignments" }

// BasecampEvent: un evento ya recibido. Basecamp reintenta lo que no contestó
// a tiempo, y el mismo evento no puede avisar dos veces.
type BasecampEvent struct {
	OrgID      string    `gorm:"type:varchar(36);primaryKey"`
	EventID    int64     `gorm:"primaryKey;autoIncrement:false"`
	ReceivedAt time.Time `gorm:"not null"`
}

func (BasecampEvent) TableName() string { return "basecamp_events" }

// BasecampOAuthState: un enlace de conectar que aún no se ha usado. El state
// firmado lleva su nonce, y la vuelta de Launchpad lo borra: un state vale una
// vez, no los diez minutos que dura. Quien viera la URL de conectar de un
// admin no puede volver con el `code` de su propia cuenta.
type BasecampOAuthState struct {
	Nonce     string    `gorm:"type:varchar(64);primaryKey"`
	OrgID     string    `gorm:"type:varchar(36);not null"`
	ExpiresAt time.Time `gorm:"not null;index"`
}

func (BasecampOAuthState) TableName() string { return "basecamp_oauth_states" }

// ─── Lo que manda Basecamp ───────────────────────────────────────────────────

// BasecampWebhookEvent: el cuerpo de una entrega de webhook (bc3-api,
// sections/webhooks.md).
type BasecampWebhookEvent struct {
	ID        int64              `json:"id"`
	Kind      string             `json:"kind"`
	CreatedAt time.Time          `json:"created_at"`
	Recording BasecampRecording  `json:"recording"`
	Creator   BasecampPersonWire `json:"creator"`
	Details   BasecampDetails    `json:"details"`
}

// BasecampDetails: lo que un evento de asignación dice que cambió. Sólo se
// lee para saber si hay algo que hacer: quién queda asignado se le pregunta
// al to-do, que no depende del orden en que lleguen los eventos.
type BasecampDetails struct {
	AddedPersonIDs   []int64 `json:"added_person_ids"`
	RemovedPersonIDs []int64 `json:"removed_person_ids"`
}

// BasecampAssignable: un to-do o una tarjeta como los devuelve la API (el
// webhook no trae ni las notas ni los asignados).
type BasecampAssignable struct {
	ID          int64                `json:"id"`
	Title       string               `json:"title"`
	Content     string               `json:"content"`
	Description string               `json:"description"`
	AppURL      string               `json:"app_url"`
	Bucket      BasecampBucket       `json:"bucket"`
	Assignees   []BasecampPersonWire `json:"assignees"`
	// Para el evento completo (command-and-control-558): la lista (o la
	// columna) en la que vive, su fecha y si está hecho.
	Parent    *BasecampRecordRef `json:"parent"`
	DueOn     string             `json:"due_on"`
	Completed bool               `json:"completed"`
}

type BasecampRecording struct {
	ID      int64               `json:"id"`
	Type    string              `json:"type"`
	Title   string              `json:"title"`
	AppURL  string              `json:"app_url"`
	Content string              `json:"content"`
	Parent  *BasecampRecordRef  `json:"parent"`
	Bucket  BasecampBucket      `json:"bucket"`
	Creator *BasecampPersonWire `json:"creator"`
}

type BasecampRecordRef struct {
	ID     int64  `json:"id"`
	Title  string `json:"title"`
	Type   string `json:"type"`
	AppURL string `json:"app_url"`
}

type BasecampBucket struct {
	ID   int64  `json:"id"`
	Name string `json:"name"`
}

// BasecampPersonWire: una persona como la escribe Basecamp.
type BasecampPersonWire struct {
	ID             int64  `json:"id"`
	Name           string `json:"name"`
	EmailAddress   string `json:"email_address"`
	AttachableSGID string `json:"attachable_sgid"`
	Client         bool   `json:"client"`
}

// Los eventos que se atienden. El resto se recibe y se ignora.
const (
	BasecampCommentCreated = "comment_created"
	BasecampMessageCreated = "message_created"
	BasecampTodoCompleted  = "todo_completed"
	// Fase 3 (command-and-control-557): un to-do o una tarjeta nuevos o
	// asignados son una tarea enlazada; reabrir un to-do es una línea más.
	BasecampTodoUncompleted = "todo_uncompleted"
	BasecampTodoCreated     = "todo_created"
	BasecampTodoAssignment  = "todo_assignment_changed"
	BasecampCardCreated     = "kanban_card_created"
	BasecampCardAssignment  = "kanban_card_assignment_changed"
)

// BasecampMakesTask: el evento trae (o pone al día) la tarea de su to-do o su
// tarjeta, con sus responsables.
func BasecampMakesTask(kind string) bool {
	switch kind {
	case BasecampTodoCreated, BasecampTodoAssignment, BasecampCardCreated, BasecampCardAssignment:
		return true
	}
	return false
}

// BasecampIsCard: el evento es de una tarjeta (Kanban::Card), que se lee por
// otra ruta de la API y guarda su descripción en `content`.
func BasecampIsCard(kind string) bool {
	return kind == BasecampCardCreated || kind == BasecampCardAssignment
}

// Target: el recording del que trata el evento, el que se enlaza a una
// tarea. Un comentario habla de su padre (el to-do, el mensaje); lo demás, de
// sí mismo.
func (e BasecampWebhookEvent) Target() (int64, string, string) {
	if e.Kind == BasecampCommentCreated && e.Recording.Parent != nil {
		p := e.Recording.Parent
		return p.ID, p.Title, p.AppURL
	}
	return e.Recording.ID, e.Recording.Title, e.Recording.AppURL
}

// ─── Menciones ───────────────────────────────────────────────────────────────

// Una mención en el HTML de Basecamp:
// `<bc-attachment sgid="…" content-type="application/vnd.basecamp.mention">`.
var (
	reBcAttachment = regexp.MustCompile(`(?is)<bc-attachment\b([^>]*)>`)
	reAttrSGID     = regexp.MustCompile(`(?is)\bsgid\s*=\s*"([^"]+)"`)
	reAttrMention  = regexp.MustCompile(`(?is)\bcontent-type\s*=\s*"application/vnd\.basecamp\.mention"`)
	rePersonGID    = regexp.MustCompile(`gid://bc3/Person/(\d+)`)
)

// BasecampMention: a quién nombra una mención. El sgid siempre; el id de la
// persona cuando el sgid lo deja leer (es un id global de Rails firmado, cuya
// parte de datos va en base64).
type BasecampMention struct {
	SGID     string
	PersonID int64
}

// ExtractBasecampMentions: las menciones de un HTML, cada una una vez.
func ExtractBasecampMentions(content string) []BasecampMention {
	var out []BasecampMention
	seen := map[string]bool{}
	for _, m := range reBcAttachment.FindAllStringSubmatch(content, -1) {
		attrs := m[1]
		if !reAttrMention.MatchString(attrs) {
			continue
		}
		sg := reAttrSGID.FindStringSubmatch(attrs)
		if sg == nil {
			continue
		}
		sgid := html.UnescapeString(sg[1])
		if seen[sgid] {
			continue
		}
		seen[sgid] = true
		out = append(out, BasecampMention{SGID: sgid, PersonID: personIDFromSGID(sgid)})
	}
	return out
}

func personIDFromSGID(sgid string) int64 {
	data, _, _ := strings.Cut(sgid, "--")
	for _, enc := range []*base64.Encoding{base64.RawURLEncoding, base64.URLEncoding, base64.StdEncoding, base64.RawStdEncoding} {
		raw, err := enc.DecodeString(data)
		if err != nil {
			continue
		}
		if m := rePersonGID.FindSubmatch(raw); m != nil {
			id, _ := strconv.ParseInt(string(m[1]), 10, 64)
			return id
		}
	}
	return 0
}

var (
	reTags   = regexp.MustCompile(`(?s)<[^>]*>`)
	reBlocks = regexp.MustCompile(`(?i)</?(div|p|br|li|h[1-6]|blockquote|figure)\b[^>]*>`)
	reSpaces = regexp.MustCompile(`[ \t\r\f\v]+`)
	reBlank  = regexp.MustCompile(`\n{3,}`)
	// Una mención entera, con lo que lleva dentro: Basecamp la pinta como un
	// `<figure>` con el nombre en el `<figcaption>`, pero en la frase es una
	// palabra, no un bloque.
	reMentionElement = regexp.MustCompile(`(?is)<bc-attachment\b([^>]*)>(.*?)</bc-attachment>`)
)

// inlineMentions cambia cada mención por el nombre que enseña, para que el
// extracto diga «Ana please review» y no parta la línea en el `<figure>`.
func inlineMentions(content string) string {
	return reMentionElement.ReplaceAllStringFunc(content, func(el string) string {
		m := reMentionElement.FindStringSubmatch(el)
		if !reAttrMention.MatchString(m[1]) {
			return el
		}
		return strings.Join(strings.Fields(html.UnescapeString(reTags.ReplaceAllString(m[2], " "))), " ")
	})
}

// BasecampPlainText: el texto de un HTML de Basecamp, sin etiquetas y cortado
// a `max` runas. Lo que se copia a una tarea o a un aviso es un extracto, no el
// documento.
func BasecampPlainText(content string, max int) string {
	s := reBlocks.ReplaceAllString(inlineMentions(content), "\n")
	s = reTags.ReplaceAllString(s, "")
	s = html.UnescapeString(s)
	s = reSpaces.ReplaceAllString(s, " ")
	lines := strings.Split(s, "\n")
	for i := range lines {
		lines[i] = strings.TrimSpace(lines[i])
	}
	s = strings.TrimSpace(reBlank.ReplaceAllString(strings.Join(lines, "\n"), "\n\n"))
	if r := []rune(s); max > 0 && len(r) > max {
		s = strings.TrimSpace(string(r[:max])) + "…"
	}
	return s
}

// ─── Direcciones ─────────────────────────────────────────────────────────────

// SafeBasecampURL: sólo enlaces https a basecamp.com. Lo que llega no va
// firmado, y un `javascript:` en un aviso no se pinta nunca.
func SafeBasecampURL(raw string) string {
	u, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || u.Scheme != "https" || u.User != nil {
		return ""
	}
	host := strings.ToLower(u.Hostname())
	if host != "basecamp.com" && !strings.HasSuffix(host, ".basecamp.com") {
		return ""
	}
	return u.String()
}

// reBasecampURL: una URL de Basecamp dentro de un texto. Lo que la cierra es
// lo que no puede ir en una dirección de markdown ni de texto corrido.
var reBasecampURL = regexp.MustCompile(`https://[A-Za-z0-9.-]*basecamp\.com/[^\s)<>\]"'` + "`" + `]+`)

// BasecampURLsIn: las URLs de recordings de Basecamp de un texto (una
// descripción), cada una una vez y en el orden en que salen.
func BasecampURLsIn(text string) []string {
	var out []string
	seen := map[int64]bool{}
	for _, raw := range reBasecampURL.FindAllString(text, -1) {
		raw = strings.TrimRight(raw, ".,;:!?")
		_, _, rec, err := ParseBasecampRecordingURL(raw)
		if err != nil || seen[rec] {
			continue
		}
		seen[rec] = true
		out = append(out, raw)
	}
	return out
}

var reRecordingPath = regexp.MustCompile(`^/(\d+)/buckets/(\d+)/(?:[a-z_]+/)+(\d+)(?:\.json)?/?$`)

// ParseBasecampRecordingURL: la cuenta, el proyecto y el recording de una URL
// de Basecamp (`https://3.basecamp.com/<cuenta>/buckets/<proyecto>/todos/<id>`,
// o `…/card_tables/cards/<id>` para una tarjeta).
// Lo que se pega para enlazar una tarea.
func ParseBasecampRecordingURL(raw string) (account, project, recording int64, err error) {
	safe := SafeBasecampURL(raw)
	if safe == "" {
		return 0, 0, 0, ErrBasecampBadURL
	}
	u, _ := url.Parse(safe)
	m := reRecordingPath.FindStringSubmatch(u.Path)
	if m == nil {
		return 0, 0, 0, ErrBasecampBadURL
	}
	account, _ = strconv.ParseInt(m[1], 10, 64)
	project, _ = strconv.ParseInt(m[2], 10, 64)
	recording, _ = strconv.ParseInt(m[3], 10, 64)
	if account == 0 || project == 0 || recording == 0 {
		return 0, 0, 0, ErrBasecampBadURL
	}
	return account, project, recording, nil
}

// BasecampSecretHash: cómo se guarda (y se busca) el secreto de un webhook.
func BasecampSecretHash(secret string) string {
	sum := sha256.Sum256([]byte(secret))
	return hex.EncodeToString(sum[:])
}

// ─── La API para la app (Organización → Integraciones) ───────────────────────

// BasecampStatus: `GET /api/v1/organizations/{id}/basecamp`.
type BasecampStatus struct {
	// Configured: el servidor tiene la app de 37signals. Sin ella, conectar
	// contesta «no configurado» y la app lo dice.
	Configured bool `json:"configured"`
	Connected  bool `json:"connected"`
	// Lo demás sólo con Connected.
	AccountID       int64             `json:"accountId,omitempty"`
	AccountName     string            `json:"accountName,omitempty"`
	IdentityName    string            `json:"identityName,omitempty"`
	ExpiresAt       *time.Time        `json:"expiresAt,omitempty"`
	RefreshFailedAt *time.Time        `json:"refreshFailedAt,omitempty"`
	Projects        []BasecampProject `json:"projects"`
	// Unmatched: cuántas personas quedan sin usuario de cac.
	Unmatched int64 `json:"unmatched"`
	// Awaiting: cuántos hilos esperan respuesta del equipo
	// (`…/basecamp/awaiting`).
	Awaiting int64 `json:"awaiting"`
}

// BasecampConnectResponse: a dónde mandar el navegador para autorizar.
type BasecampConnectResponse struct {
	URL string `json:"url"`
}

// BasecampAvailableProject: un proyecto de la cuenta, y si ya está enlazado.
type BasecampAvailableProject struct {
	ID      int64  `json:"id"`
	Name    string `json:"name"`
	AppURL  string `json:"appUrl"`
	SpaceID string `json:"spaceId,omitempty"`
}

// LinkBasecampProjectRequest: `PUT …/basecamp/projects/{projectId}`.
type LinkBasecampProjectRequest struct {
	SpaceID string `json:"spaceId" validate:"required,max=36"`
}

// MatchBasecampPersonRequest: `PUT …/basecamp/people/{personId}`. UserID
// vacío suelta la persona.
type MatchBasecampPersonRequest struct {
	UserID string `json:"userId" validate:"max=36"`
}

// LinkBasecampRecordingRequest: `POST /api/v1/tasks/{id}/basecamp`.
type LinkBasecampRecordingRequest struct {
	URL string `json:"url" validate:"required,max=500"`
}
