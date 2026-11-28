package domain

import (
	"crypto/rand"
	"encoding/json"
	"errors"
	"math/big"
	"strings"
	"time"
)

// Un pedido es lo que un agente deja para una persona y no puede hacer solo:
// aprobar un mensaje, probar algo, decidir, correr algo a mano o conseguir lo
// que falta (command-and-control-500). Vive ligado a una tarea pero aparte de
// ella: la tarea no cambia de estado por tener pedidos abiertos, sólo enseña
// que espera a alguien. Lo decidió José el 24-11: un estado nuevo describe mal
// una tarea con dos pedidos o con trabajo en paralelo, y tocaría las columnas
// de todas las listas, también las de los clientes.
//
// Va a **una sola persona**. Si hacen falta dos, son dos pedidos: un «lo
// contesta cualquiera de estos» deja sin saber de quién es el pedido.

type RequestKind string

const (
	RequestApproveMessage RequestKind = "approve_message"
	RequestTest           RequestKind = "test"
	RequestDecide         RequestKind = "decide"
	RequestRun            RequestKind = "run"
	RequestObtain         RequestKind = "obtain"
)

func ValidRequestKind(k RequestKind) bool {
	switch k {
	case RequestApproveMessage, RequestTest, RequestDecide, RequestRun, RequestObtain:
		return true
	}
	return false
}

type RequestStatus string

const (
	RequestOpen      RequestStatus = "open"
	RequestAnswered  RequestStatus = "answered"
	RequestWithdrawn RequestStatus = "withdrawn"
)

// Por dónde llegó la respuesta. «session» es una sesión de Claude Code que la
// apuntó porque la persona lo dijo en la terminal: la respuesta es de ella
// igual, pero quien lea el pedido tiene que saber que no pulsó el botón.
const (
	AnsweredViaApp     = "app"
	AnsweredViaSession = "session"
)

// Request es el pedido. `Body` y `Answer` son JSON con la forma de su tipo
// (RequestBody, RequestAnswer), validada al entrar: guardarlos como columnas
// sueltas serían veinte columnas casi siempre vacías.
type Request struct {
	BaseModel
	// Code es el ID que se copia y se dice en la terminal: `req-` y seis letras.
	// Corto a propósito, para dictarlo; el uuid sigue siendo la clave.
	Code    string      `gorm:"type:varchar(16);uniqueIndex;not null" json:"code"`
	OrgID   string      `gorm:"type:varchar(36);index;not null" json:"orgId"`
	TaskID  string      `gorm:"type:varchar(36);index;not null" json:"taskId"`
	Kind    RequestKind `gorm:"type:varchar(20);not null" json:"kind"`
	Summary string      `gorm:"type:text;not null" json:"summary"`
	Body    []byte      `gorm:"type:jsonb;not null" json:"-"`
	// Version sube con cada `update_request`; las anteriores, en RequestVersion.
	Version       int           `gorm:"not null;default:1" json:"version"`
	RequestedOfID string        `gorm:"type:varchar(36);index:idx_request_inbox,priority:1;not null" json:"requestedOfId"`
	RequestedByID string        `gorm:"type:varchar(36);not null" json:"requestedById"`
	Via           string        `gorm:"type:varchar(10)" json:"via,omitempty"`
	Status        RequestStatus `gorm:"type:varchar(12);index:idx_request_inbox,priority:2;not null" json:"status"`

	Answer       []byte     `gorm:"type:jsonb" json:"-"`
	AnsweredByID string     `gorm:"type:varchar(36)" json:"answeredById,omitempty"`
	AnsweredAt   *time.Time `json:"answeredAt,omitempty"`
	AnsweredVia  string     `gorm:"type:varchar(10)" json:"answeredVia,omitempty"`
	// PublishedURL: dónde publicó el agente el mensaje aprobado. El texto
	// aprobado queda congelado en la respuesta; esto dice dónde acabó.
	PublishedURL string     `gorm:"type:text" json:"publishedUrl,omitempty"`
	WithdrawnAt  *time.Time `json:"withdrawnAt,omitempty"`
}

func (Request) TableName() string { return "requests" }

// RequestVersion: una versión anterior de un pedido que se rehízo.
type RequestVersion struct {
	BaseModel
	RequestID string      `gorm:"type:varchar(36);index;not null" json:"-"`
	Version   int         `gorm:"not null" json:"version"`
	Kind      RequestKind `gorm:"type:varchar(20);not null" json:"kind"`
	Summary   string      `gorm:"type:text;not null" json:"summary"`
	Body      []byte      `gorm:"type:jsonb;not null" json:"-"`
	// Quién la escribió (el que pidió esa versión), no quién la sustituyó.
	ByID string `gorm:"type:varchar(36)" json:"-"`
}

func (RequestVersion) TableName() string { return "request_versions" }

// ─── El ID corto ─────────────────────────────────────────────────────────────

// Sin 0/o, 1/l/i: se dicta y se lee en una terminal, y ésas se confunden.
const requestCodeAlphabet = "23456789abcdefghjkmnpqrstuvwxyz"

const RequestCodePrefix = "req-"

func NewRequestCode() string {
	var b strings.Builder
	b.WriteString(RequestCodePrefix)
	n := big.NewInt(int64(len(requestCodeAlphabet)))
	for range 6 {
		i, err := rand.Int(rand.Reader, n)
		if err != nil {
			panic(err)
		}
		b.WriteByte(requestCodeAlphabet[i.Int64()])
	}
	return b.String()
}

// LooksLikeRequestCode: si lo que llega es un ID corto y no un uuid.
func LooksLikeRequestCode(s string) bool {
	return strings.HasPrefix(strings.ToLower(strings.TrimSpace(s)), RequestCodePrefix)
}

// ─── El cuerpo según el tipo ─────────────────────────────────────────────────

// RequestBody junta los campos de los cinco tipos. Cada tipo usa los suyos y
// Validate rechaza lo que le falte.
type RequestBody struct {
	// approve_message: el texto exacto y a dónde va.
	Draft          string `json:"draft,omitempty"`
	Destination    string `json:"destination,omitempty"`
	DestinationURL string `json:"destinationUrl,omitempty"`
	// test: la pantalla real, pasos cortos y lo que debe verse.
	URL      string   `json:"url,omitempty"`
	Steps    []string `json:"steps,omitempty"`
	Expected string   `json:"expected,omitempty"`
	// decide: de 2 a 4 opciones, la recomendada primero.
	Question string          `json:"question,omitempty"`
	Options  []RequestOption `json:"options,omitempty"`
	// run: qué correr, dónde, qué debe salir (Expected) y cómo deshacerlo.
	Command string `json:"command,omitempty"`
	Where   string `json:"where,omitempty"`
	Undo    string `json:"undo,omitempty"`
	// obtain: qué falta y quién lo tiene.
	What string `json:"what,omitempty"`
	From string `json:"from,omitempty"`
}

type RequestOption struct {
	Label       string `json:"label"`
	Consequence string `json:"consequence"`
	Recommended bool   `json:"recommended,omitempty"`
}

var ErrRequestBody = errors.New("request-body-invalid")

type RequestBodyError struct{ Msg string }

func (e *RequestBodyError) Error() string { return e.Msg }
func (e *RequestBodyError) Unwrap() error { return ErrRequestBody }

func bodyErr(msg string) error { return &RequestBodyError{Msg: msg} }

// Normalize valida el cuerpo para su tipo y lo deja limpio: sin espacios
// sobrantes, sin pasos vacíos, con la opción recomendada primero.
func (b RequestBody) Normalize(kind RequestKind) (RequestBody, error) {
	t := strings.TrimSpace
	var out RequestBody
	switch kind {
	case RequestApproveMessage:
		out = RequestBody{Draft: t(b.Draft), Destination: t(b.Destination), DestinationURL: t(b.DestinationURL)}
		if out.Draft == "" || out.Destination == "" {
			return out, bodyErr("approve_message needs draft (the exact text) and destination (where it goes)")
		}
	case RequestTest:
		out = RequestBody{URL: t(b.URL), Expected: t(b.Expected)}
		for _, s := range b.Steps {
			if s = t(s); s != "" {
				out.Steps = append(out.Steps, s)
			}
		}
		if len(out.Steps) == 0 || out.Expected == "" {
			return out, bodyErr("test needs steps (at least one) and expected (what should be seen)")
		}
	case RequestDecide:
		out = RequestBody{Question: t(b.Question)}
		recommended := 0
		for _, o := range b.Options {
			o = RequestOption{Label: t(o.Label), Consequence: t(o.Consequence), Recommended: o.Recommended}
			if o.Label == "" || o.Consequence == "" {
				return out, bodyErr("every option needs a label and its consequence")
			}
			if o.Recommended {
				recommended++
			}
			out.Options = append(out.Options, o)
		}
		if len(out.Options) < 2 || len(out.Options) > 4 {
			return out, bodyErr("decide needs 2 to 4 options")
		}
		if recommended > 1 {
			return out, bodyErr("at most one option is recommended")
		}
		seen := map[string]bool{}
		for _, o := range out.Options {
			if seen[strings.ToLower(o.Label)] {
				return out, bodyErr("two options share the label " + o.Label)
			}
			seen[strings.ToLower(o.Label)] = true
		}
		// La recomendada primero: es la que se lee antes y la que se pulsa sin
		// pensar, así que tiene que ser la que el agente defiende.
		for i, o := range out.Options {
			if o.Recommended && i > 0 {
				rest := append([]RequestOption{o}, out.Options[:i]...)
				out.Options = append(rest, out.Options[i+1:]...)
				break
			}
		}
	case RequestRun:
		out = RequestBody{Command: t(b.Command), Where: t(b.Where), Expected: t(b.Expected), Undo: t(b.Undo)}
		if out.Command == "" || out.Where == "" || out.Expected == "" {
			return out, bodyErr("run needs command, where (which machine or directory) and expected (what should come out)")
		}
	case RequestObtain:
		out = RequestBody{What: t(b.What), From: t(b.From)}
		if out.What == "" || out.From == "" {
			return out, bodyErr("obtain needs what (what is missing) and from (who has it)")
		}
	default:
		return out, bodyErr("unknown kind: use approve_message, test, decide, run or obtain")
	}
	return out, nil
}

// ─── La respuesta ────────────────────────────────────────────────────────────

// RequestAnswer: lo que contestó la persona. Como el cuerpo, un solo tipo con
// los campos de todos.
type RequestAnswer struct {
	// approve_message: aprobado (con el texto final) o rechazado (con motivo).
	Approved  *bool  `json:"approved,omitempty"`
	FinalText string `json:"finalText,omitempty"`
	// Edited: el texto final no es el borrador. Se calcula, no se cree.
	Edited bool   `json:"edited,omitempty"`
	Reason string `json:"reason,omitempty"`
	// test: funciona o no; si no, la nota es obligatoria.
	Works         *bool  `json:"works,omitempty"`
	ScreenshotURL string `json:"screenshotUrl,omitempty"`
	// decide: la etiqueta de la opción elegida.
	Option string `json:"option,omitempty"`
	// run: lo que salió, pegado.
	Output string `json:"output,omitempty"`
	Note   string `json:"note,omitempty"`
}

var ErrRequestAnswer = errors.New("request-answer-invalid")

type RequestAnswerError struct{ Msg string }

func (e *RequestAnswerError) Error() string { return e.Msg }
func (e *RequestAnswerError) Unwrap() error { return ErrRequestAnswer }

func answerErr(msg string) error { return &RequestAnswerError{Msg: msg} }

// Normalize valida la respuesta contra el pedido y devuelve lo que se guarda.
// Lo que no es de su tipo se descarta: una respuesta a un «probar» no lleva
// texto aprobado aunque alguien lo mande.
func (a RequestAnswer) Normalize(kind RequestKind, body RequestBody) (RequestAnswer, error) {
	t := strings.TrimSpace
	note := t(a.Note)
	switch kind {
	case RequestApproveMessage:
		if a.Approved == nil {
			return a, answerErr("approve_message is answered with approved true or false")
		}
		if !*a.Approved {
			reason := t(a.Reason)
			if reason == "" {
				reason = note
			}
			if reason == "" {
				return a, answerErr("a rejection needs its reason")
			}
			return RequestAnswer{Approved: a.Approved, Reason: reason}, nil
		}
		// Sin texto final, se aprueba el borrador tal cual. Lo que se aprueba
		// queda congelado aquí: es lo único que el agente puede publicar.
		final := t(a.FinalText)
		if final == "" {
			final = body.Draft
		}
		return RequestAnswer{Approved: a.Approved, FinalText: final, Edited: final != body.Draft, Note: note}, nil
	case RequestTest:
		if a.Works == nil {
			return a, answerErr("test is answered with works true or false")
		}
		if !*a.Works && note == "" {
			return a, answerErr("«doesn't work» needs a note saying what was seen")
		}
		return RequestAnswer{Works: a.Works, Note: note, ScreenshotURL: t(a.ScreenshotURL)}, nil
	case RequestDecide:
		chosen := t(a.Option)
		for _, o := range body.Options {
			if strings.EqualFold(o.Label, chosen) {
				return RequestAnswer{Option: o.Label, Note: note}, nil
			}
		}
		labels := make([]string, 0, len(body.Options))
		for _, o := range body.Options {
			labels = append(labels, o.Label)
		}
		return a, answerErr("option must be one of: " + strings.Join(labels, ", "))
	case RequestRun:
		return RequestAnswer{Output: strings.TrimRight(a.Output, " \n\t"), Note: note}, nil
	case RequestObtain:
		return RequestAnswer{Note: note}, nil
	}
	return a, answerErr("unknown kind")
}

// ─── Peticiones y respuestas de la API ───────────────────────────────────────

type CreateRequestRequest struct {
	// TaskID: el id de la tarea o su ref (`proteus-12`).
	TaskID  string          `json:"taskId" validate:"required"`
	Kind    RequestKind     `json:"kind" validate:"required"`
	Summary string          `json:"summary" validate:"required,max=1000"`
	Body    json.RawMessage `json:"body"`
	// RequestedOf: usuario, nombre o id de un miembro. Vacío: quien llama.
	RequestedOf string `json:"requestedOf"`
}

// UpdateRequestRequest rehace un pedido abierto; lo que no se manda se queda.
type UpdateRequestRequest struct {
	Kind        *RequestKind    `json:"kind"`
	Summary     *string         `json:"summary" validate:"omitempty,max=1000"`
	Body        json.RawMessage `json:"body"`
	RequestedOf *string         `json:"requestedOf"`
}

type PublishedRequest struct {
	URL string `json:"url" validate:"required,max=2000"`
}

// RequestPerson: alguien en un pedido, con su nombre para enseñar.
type RequestPerson struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

// RequestTaskRef: la tarea de un pedido, lo justo para enlazarla.
type RequestTaskRef struct {
	ID       string `json:"id"`
	Ref      string `json:"ref,omitempty"`
	Title    string `json:"title"`
	Priority string `json:"priority"`
	Status   string `json:"status"`
	Link     string `json:"link"`
}

// RequestView es un pedido tal como lo leen la app, la nativa y el MCP.
type RequestView struct {
	ID          string         `json:"id"`
	Code        string         `json:"code"`
	OrgID       string         `json:"orgId"`
	Task        RequestTaskRef `json:"task"`
	Kind        RequestKind    `json:"kind"`
	Summary     string         `json:"summary"`
	Body        RequestBody    `json:"body"`
	Version     int            `json:"version"`
	Status      RequestStatus  `json:"status"`
	RequestedOf RequestPerson  `json:"requestedOf"`
	RequestedBy RequestPerson  `json:"requestedBy"`
	Via         string         `json:"via,omitempty"`
	CreatedAt   time.Time      `json:"createdAt"`
	UpdatedAt   time.Time      `json:"updatedAt"`
	Answer      *RequestAnswer `json:"answer,omitempty"`
	AnsweredBy  *RequestPerson `json:"answeredBy,omitempty"`
	AnsweredAt  *time.Time     `json:"answeredAt,omitempty"`
	AnsweredVia string         `json:"answeredVia,omitempty"`
	Published   string         `json:"publishedUrl,omitempty"`
	WithdrawnAt *time.Time     `json:"withdrawnAt,omitempty"`
	// CanAnswer: quien mira es a quien se le pidió y sigue abierto. En
	// «Equipo» se ven los de los demás sin poder contestarlos por ellos.
	CanAnswer bool   `json:"canAnswer"`
	Link      string `json:"link"`
	// Sólo en el detalle (GET /requests/{id}): lo que una sesión necesita para
	// seguir desde la terminal.
	History []RequestVersionView `json:"history,omitempty"`
	Handoff *RequestHandoff      `json:"handoff,omitempty"`
}

type RequestVersionView struct {
	Version   int           `json:"version"`
	Kind      RequestKind   `json:"kind"`
	Summary   string        `json:"summary"`
	Body      RequestBody   `json:"body"`
	By        RequestPerson `json:"by"`
	CreatedAt time.Time     `json:"createdAt"`
}

type RequestHandoff struct {
	Text string     `json:"text"`
	At   *time.Time `json:"at,omitempty"`
	By   string     `json:"by,omitempty"`
}

// RequestLink: dónde se abre un pedido en la app.
func RequestLink(code string) string { return "/requests?open=" + code }

// TaskOpenRequests: lo que una tarea dice de sus pedidos abiertos, en el
// detalle y en get_context. Con esto se ve «espera a Ana» sin otra consulta.
type TaskOpenRequests struct {
	Count     int      `json:"count"`
	WaitingOn []string `json:"waitingOn"`
	Codes     []string `json:"codes"`
}

// MyOpenRequests: los pedidos abiertos de quien pregunta, para que una sesión
// que arranca diga «tienes N pendientes».
type MyOpenRequests struct {
	Count int                 `json:"count"`
	Top   []MyOpenRequestLine `json:"top"`
}

type MyOpenRequestLine struct {
	Code    string      `json:"code"`
	Kind    RequestKind `json:"kind"`
	Summary string      `json:"summary"`
	TaskRef string      `json:"taskRef,omitempty"`
	Link    string      `json:"link"`
}

// RequestPage: una página de la bandeja, cuando se pide con cursor. `Next`
// vacío: no hay más.
type RequestPage struct {
	Requests []RequestView `json:"requests"`
	Next     string        `json:"next,omitempty"`
}

// RequestWait: lo que contesta la espera larga.
type RequestWait struct {
	Settled bool         `json:"settled"`
	Request *RequestView `json:"request"`
}

// RequestOpened: lo que contesta pedir que la app abra un pedido.
type RequestOpened struct {
	Delivered bool   `json:"delivered"`
	Link      string `json:"link"`
}

// Los eventos del stream. `request:open` y `request:opened` van a una sola
// persona; los demás, a la organización (la bandeja «Equipo» también los ve).
const (
	RequestCreatedEvent   = "request:created"
	RequestUpdatedEvent   = "request:updated"
	RequestAnsweredEvent  = "request:answered"
	RequestWithdrawnEvent = "request:withdrawn"
	RequestOpenEvent      = "request:open"
	RequestOpenedEvent    = "request:opened"
)

// RequestNotifyKind: la clase del aviso en la campana. Se calla con lo mismo
// que el resto del trabajo que te llega.
const RequestNotifyKind = "request:new"
