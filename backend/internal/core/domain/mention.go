package domain

import (
	"regexp"
	"strings"
)

// ─── Mentions ─────────────────────────────────────────────────────────────────

// MentionRef is the link a mention is stored as: `[@jose](cac:user/<uuid>)`.
//
// A scheme of our own rather than a path like `/users/<id>`, for two reasons.
// A real path would collide with a route somebody might add later, and would be
// followed by anything that treats the markdown as ordinary text. And a scheme
// makes extracting mentions from a body unambiguous — the server has to do that
// on every message, and "looks like a link to a user" is not a rule worth
// guessing at.
func MentionRef(userID string) string { return "cac:user/" + userID }

// mentionPattern matches the target of a mention link, and nothing else. Ids
// are uuids, so the shape is pinned rather than left as "anything after the
// slash": a body is written by a person and arrives as free text.
var mentionPattern = regexp.MustCompile(`cac:user/([0-9a-fA-F-]{36})`)

// ExtractMentions returns the user ids named in a body, without duplicates and
// in the order they appear.
//
// Order matters only so the result is stable — two identical bodies produce the
// same list, which keeps events and tests from depending on map iteration.
//
// **These ids are asserted, not verified.** The body comes from whoever typed
// it, so a caller could name anybody in the platform. Deciding who may actually
// be mentioned is the service's job, against the item's organization; this
// function only reads what the text says.
func ExtractMentions(body string) []string {
	matches := mentionPattern.FindAllStringSubmatch(body, -1)
	if len(matches) == 0 {
		return nil
	}
	seen := make(map[string]bool, len(matches))
	out := make([]string, 0, len(matches))
	for _, m := range matches {
		id := m[1]
		if seen[id] {
			continue
		}
		seen[id] = true
		out = append(out, id)
	}
	return out
}

// ─── Menciones escritas a mano ───────────────────────────────────────────────

// MentionTarget is someone a hand-written `@username` may resolve to: a member
// of the body's organization, with the name to show in the link.
type MentionTarget struct {
	ID       string
	Username string
	Name     string
}

// handMention: un `@usuario` escrito en texto. El grupo 1 es lo que va delante,
// porque Go no tiene lookbehind: un correo (`ana@cac.dev`), una ruta
// (`/@ana`), un escape (`\@ana`) o el texto de un enlace (`[@ana](…)`) no son
// una mención, y se reconocen por el carácter anterior.
var handMention = regexp.MustCompile(`(^|[^A-Za-z0-9._%+\-@/\\\[])@([A-Za-z0-9][A-Za-z0-9._-]*)`)

// protectedInline: lo que dentro de una línea de prosa no se toca. Un enlace
// markdown entero (meter un enlace dentro del texto de otro lo rompe) y una URL
// suelta, también dentro de `<…>`.
var protectedInline = regexp.MustCompile(`!?\[[^\]\n]*\]\([^)\n]*\)|[A-Za-z][A-Za-z0-9+.\-]*://[^\s<>]+`)

// HandMentions returns the usernames written as plain `@username` in a body,
// lowercased, without duplicates and in order of appearance. Code (inline or
// fenced), links, URLs and e-mail addresses are skipped.
//
// Like ExtractMentions, these are only what the text says: whether each one is
// a member of the organization is the caller's question.
func HandMentions(body string) []string {
	var out []string
	seen := map[string]bool{}
	rewriteHandMentions(body, func(username string) (string, bool) {
		key := strings.ToLower(username)
		if !seen[key] {
			seen[key] = true
			out = append(out, key)
		}
		return "", false
	})
	return out
}

// LinkHandMentions rewrites each plain `@username` naming one of `members`
// (keyed by lowercased username) as the link the app writes, so it notifies
// and renders like a mention picked from the menu. Anything else is left as
// it was, byte for byte.
func LinkHandMentions(body string, members map[string]MentionTarget) string {
	if len(members) == 0 {
		return body
	}
	return rewriteHandMentions(body, func(username string) (string, bool) {
		t, ok := members[strings.ToLower(username)]
		if !ok {
			return "", false
		}
		return "[@" + mentionLabel(t) + "](" + MentionRef(t.ID) + ")", true
	})
}

// mentionLabel: el nombre para enseñar, sin lo que cerraría el texto del enlace
// antes de tiempo.
func mentionLabel(t MentionTarget) string {
	name := strings.TrimSpace(t.Name)
	if name == "" {
		name = t.Username
	}
	return strings.NewReplacer("[", "", "]", "", "\\", "").Replace(name)
}

// rewriteHandMentions recorre la prosa del cuerpo y le pregunta a `swap` por
// cada `@usuario`; si contesta que sí, lo sustituye. Los bloques de código
// (``` o ~~~) y el código en línea se copian tal cual.
func rewriteHandMentions(body string, swap func(username string) (string, bool)) string {
	var b strings.Builder
	b.Grow(len(body))
	fence := ""
	lines := strings.SplitAfter(body, "\n")
	for _, line := range lines {
		trimmed := strings.TrimLeft(line, " \t")
		if fence != "" {
			if strings.HasPrefix(trimmed, fence) {
				fence = ""
			}
			b.WriteString(line)
			continue
		}
		if f := fenceOpener(trimmed); f != "" {
			fence = f
			b.WriteString(line)
			continue
		}
		rewriteLine(&b, line, swap)
	}
	return b.String()
}

// fenceOpener devuelve la marca que abre un bloque de código (``` o ~~~, de
// tres o más), o vacío si la línea no abre ninguno.
func fenceOpener(trimmed string) string {
	for _, c := range []byte{'`', '~'} {
		n := 0
		for n < len(trimmed) && trimmed[n] == c {
			n++
		}
		if n >= 3 {
			return trimmed[:n]
		}
	}
	return ""
}

// rewriteLine separa el código en línea (una tira de n acentos graves hasta la
// siguiente de n) y reescribe sólo lo de fuera. Una tira sin cierre es texto.
func rewriteLine(b *strings.Builder, line string, swap func(string) (string, bool)) {
	i := 0
	prose := 0
	for i < len(line) {
		if line[i] != '`' {
			i++
			continue
		}
		n := 0
		for i+n < len(line) && line[i+n] == '`' {
			n++
		}
		closeAt := closingTicks(line, i+n, n)
		if closeAt < 0 {
			i += n
			continue
		}
		rewriteProse(b, line[prose:i], swap)
		end := closeAt + n
		b.WriteString(line[i:end])
		i, prose = end, end
	}
	rewriteProse(b, line[prose:], swap)
}

func closingTicks(line string, from, n int) int {
	for j := from; j < len(line); {
		if line[j] != '`' {
			j++
			continue
		}
		m := 0
		for j+m < len(line) && line[j+m] == '`' {
			m++
		}
		if m == n {
			return j
		}
		j += m
	}
	return -1
}

func rewriteProse(b *strings.Builder, text string, swap func(string) (string, bool)) {
	last := 0
	for _, loc := range protectedInline.FindAllStringIndex(text, -1) {
		rewritePlain(b, text[last:loc[0]], swap)
		b.WriteString(text[loc[0]:loc[1]])
		last = loc[1]
	}
	rewritePlain(b, text[last:], swap)
}

func rewritePlain(b *strings.Builder, text string, swap func(string) (string, bool)) {
	last := 0
	for _, m := range handMention.FindAllStringSubmatchIndex(text, -1) {
		at := m[3] // donde empieza la `@`
		name := text[m[4]:m[5]]
		// El punto o el guion final es de la frase, no del usuario:
		// «avisa a @ana.» nombra a «ana».
		name = strings.TrimRight(name, "._-")
		end := m[4] + len(name)
		if name == "" || (end < len(text) && text[end] == '@') {
			continue
		}
		link, ok := swap(name)
		if !ok {
			continue
		}
		b.WriteString(text[last:at])
		b.WriteString(link)
		last = end
	}
	b.WriteString(text[last:])
}
