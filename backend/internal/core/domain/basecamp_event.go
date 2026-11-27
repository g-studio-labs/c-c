package domain

import (
	"html"
	"regexp"
	"strings"
	"time"
)

// Basecamp «por contestar» y el evento completo (command-and-control-558, D).
//
// Cada hilo de Basecamp (el recording del que trata un evento: el to-do, el
// mensaje o la tarjeta de un comentario) sabe si espera respuesta del equipo:
// el último que escribió en él es alguien de fuera y nadie del equipo
// contestó después. «Del equipo» es una persona casada con un miembro de la
// organización y que Basecamp no marca como cliente; todo lo demás, también
// una persona sin casar, es de fuera.

// BasecampThread: el estado de un hilo. Uno por organización y recording.
type BasecampThread struct {
	BaseModel
	OrgID       string `gorm:"type:varchar(36);not null;uniqueIndex:idx_basecamp_thread,priority:1"`
	RecordingID int64  `gorm:"not null;uniqueIndex:idx_basecamp_thread,priority:2"`
	ProjectID   int64  `gorm:"not null;index"`
	Title       string `gorm:"type:varchar(300)"`
	AppURL      string `gorm:"type:text"`
	Type        string `gorm:"type:varchar(40)"`
	// LastEventAt: la hora (de Basecamp) del último comentario o mensaje que
	// se aplicó. Uno más viejo que llega tarde no mueve el estado: Basecamp no
	// entrega en orden.
	LastEventAt         *time.Time
	LastExternalAt      *time.Time
	LastExternalName    string `gorm:"type:varchar(200)"`
	LastExternalClient  bool   `gorm:"not null;default:false"`
	LastExternalExcerpt string `gorm:"type:text"`
	LastTeamReplyAt     *time.Time
	LastTeamReplyByID   string `gorm:"type:varchar(36)"`
	AwaitingReply       bool   `gorm:"not null;default:false;index"`
	AwaitingSince       *time.Time
}

func (BasecampThread) TableName() string { return "basecamp_threads" }

// BasecampThreadWrite: lo que un evento le cuenta a su hilo.
type BasecampThreadWrite struct {
	OrgID       string
	RecordingID int64
	ProjectID   int64
	Title       string
	AppURL      string
	Type        string
	// Said: el evento es algo que alguien escribió en el hilo (un comentario o
	// un mensaje). Sólo eso mueve «por contestar».
	Said     bool
	At       time.Time
	External bool
	// Del autor: su nombre y si es cliente (de fuera), o su usuario (equipo).
	AuthorName   string
	AuthorClient bool
	AuthorUserID string
	Excerpt      string
}

// Apply: el hilo después de este evento. Lo de Basecamp llega fuera de orden,
// así que algo dicho antes que lo último ya aplicado no cambia nada: un
// comentario viejo del cliente no reabre lo que el equipo ya contestó, ni una
// respuesta vieja cierra lo que el cliente dijo después.
func (t *BasecampThread) Apply(w BasecampThreadWrite) {
	if w.Title != "" {
		t.Title = w.Title
	}
	if w.AppURL != "" {
		t.AppURL = w.AppURL
	}
	if w.Type != "" {
		t.Type = w.Type
	}
	if w.ProjectID != 0 {
		t.ProjectID = w.ProjectID
	}
	if !w.Said || (t.LastEventAt != nil && w.At.Before(*t.LastEventAt)) {
		return
	}
	at := w.At
	t.LastEventAt = &at
	if w.External {
		t.LastExternalAt = &at
		t.LastExternalName, t.LastExternalClient, t.LastExternalExcerpt = w.AuthorName, w.AuthorClient, w.Excerpt
		if !t.AwaitingReply {
			t.AwaitingReply, t.AwaitingSince = true, &at
		}
		return
	}
	t.LastTeamReplyAt, t.LastTeamReplyByID = &at, w.AuthorUserID
	t.AwaitingReply, t.AwaitingSince = false, nil
}

// AnsweredBy: el último del equipo que contestó es esa persona, y contestó
// después de lo último de fuera.
func (t *BasecampThread) AnsweredBy(userID string) bool {
	if t == nil || userID == "" || t.LastTeamReplyByID != userID || t.LastTeamReplyAt == nil {
		return false
	}
	return t.LastExternalAt == nil || !t.LastTeamReplyAt.Before(*t.LastExternalAt)
}

// BasecampAwaitingExternal: lo último que dijo alguien de fuera en un hilo.
type BasecampAwaitingExternal struct {
	Name    string     `json:"name"`
	Client  bool       `json:"client"`
	At      *time.Time `json:"at"`
	Excerpt string     `json:"excerpt"`
}

// BasecampAwaiting: un hilo por contestar,
// `GET /api/v1/organizations/{id}/basecamp/awaiting`.
type BasecampAwaiting struct {
	ProjectID     int64                    `json:"projectId"`
	ProjectName   string                   `json:"projectName"`
	SpaceID       string                   `json:"spaceId"`
	RecordingID   int64                    `json:"recordingId"`
	Type          string                   `json:"type"`
	Title         string                   `json:"title"`
	AppURL        string                   `json:"appUrl"`
	AwaitingSince *time.Time               `json:"awaitingSince"`
	LastExternal  BasecampAwaitingExternal `json:"lastExternal"`
	TaskID        string                   `json:"taskId,omitempty"`
	TaskRef       string                   `json:"taskRef,omitempty"`
}

// ─── El texto de un evento ───────────────────────────────────────────────────

var (
	reAttrHref     = regexp.MustCompile(`(?is)\bhref\s*=\s*"([^"]*)"`)
	reAttrURL      = regexp.MustCompile(`(?is)\burl\s*=\s*"([^"]*)"`)
	reAttrFilename = regexp.MustCompile(`(?is)\bfilename\s*=\s*"([^"]*)"`)
	reAttrCaption  = regexp.MustCompile(`(?is)\bcaption\s*=\s*"([^"]*)"`)
	reAttrType     = regexp.MustCompile(`(?is)\bcontent-type\s*=\s*"([^"]*)"`)
	reAnchor       = regexp.MustCompile(`(?is)<a\b([^>]*)>(.*?)</a>`)
	reStrong       = regexp.MustCompile(`(?is)</?(strong|b)\b[^>]*>`)
	reEm           = regexp.MustCompile(`(?is)</?(em|i)\b[^>]*>`)
	reListItem     = regexp.MustCompile(`(?is)<li\b[^>]*>`)
	reListItemEnd  = regexp.MustCompile(`(?is)</li\s*>`)
	// Un adjunto que no es mención, entero o sin cierre.
	reAttachmentElement = regexp.MustCompile(`(?is)<bc-attachment\b([^>]*)>(?:.*?</bc-attachment>)?`)
)

// BasecampAttachment: un fichero adjunto a un texto de Basecamp.
type BasecampAttachment struct {
	Name        string `json:"name"`
	URL         string `json:"url"`
	ContentType string `json:"contentType"`
}

// BasecampAttachments: los adjuntos de un HTML de Basecamp. Sólo los que
// apuntan a Basecamp (SafeBasecampURL): lo que llega no va firmado, y un
// enlace a otro sitio, o un `javascript:`, no viaja como si fuera un fichero.
func BasecampAttachments(content string) []BasecampAttachment {
	out := []BasecampAttachment{}
	for _, m := range reAttachmentElement.FindAllStringSubmatch(content, -1) {
		attrs := m[1]
		if reAttrMention.MatchString(attrs) {
			continue
		}
		raw := attrValue(reAttrHref, attrs)
		if raw == "" {
			raw = attrValue(reAttrURL, attrs)
		}
		u := SafeBasecampURL(raw)
		if u == "" {
			continue
		}
		name := attrValue(reAttrFilename, attrs)
		if name == "" {
			name = attrValue(reAttrCaption, attrs)
		}
		out = append(out, BasecampAttachment{Name: name, URL: u, ContentType: attrValue(reAttrType, attrs)})
	}
	return out
}

func attrValue(re *regexp.Regexp, attrs string) string {
	m := re.FindStringSubmatch(attrs)
	if m == nil {
		return ""
	}
	return strings.TrimSpace(html.UnescapeString(m[1]))
}

// BasecampMarkdown: el texto entero de un HTML de Basecamp en markdown, con
// cada mención como «@Nombre». Un enlace que no es de Basecamp se queda en su
// texto y su dirección a la vista, no como enlace pulsable.
func BasecampMarkdown(content string) string {
	s := reMentionElement.ReplaceAllStringFunc(content, func(el string) string {
		m := reMentionElement.FindStringSubmatch(el)
		if !reAttrMention.MatchString(m[1]) {
			return el
		}
		name := strings.Join(strings.Fields(html.UnescapeString(reTags.ReplaceAllString(m[2], " "))), " ")
		if name == "" {
			return ""
		}
		return "@" + name
	})
	// Los adjuntos van aparte (BasecampAttachments): en el texto, su nombre.
	s = reAttachmentElement.ReplaceAllStringFunc(s, func(el string) string {
		m := reAttachmentElement.FindStringSubmatch(el)
		if name := attrValue(reAttrFilename, m[1]); name != "" {
			return "[" + name + "]"
		}
		return ""
	})
	s = reAnchor.ReplaceAllStringFunc(s, func(el string) string {
		m := reAnchor.FindStringSubmatch(el)
		text := strings.TrimSpace(html.UnescapeString(reTags.ReplaceAllString(m[2], "")))
		href := attrValue(reAttrHref, m[1])
		if href == "" || href == text {
			return text
		}
		if text == "" {
			return href
		}
		return text + " (" + href + ")"
	})
	s = reStrong.ReplaceAllString(s, "**")
	s = reEm.ReplaceAllString(s, "_")
	s = reListItemEnd.ReplaceAllString(s, "")
	s = reListItem.ReplaceAllString(s, "\n- ")
	return BasecampPlainText(s, 0)
}
