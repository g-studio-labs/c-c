package service

import (
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// Un `@usuario` escrito a mano avisa como una mención de la app y se guarda
// como su enlace (command-and-control-550). Antes sólo contaba la forma
// enlace, y un mensaje del MCP con `@Diegotsx` no le llegaba a Diego.
//
// ana, bea y caro son de org-1; dani, de org-2 (ver chatFollowDB). Se añade
// «Eli», con mayúscula como los usuarios de antes de la regla de minúsculas y
// con nombre, para ver que el enlace lleva el nombre para enseñar.
func TestAHandWrittenMentionNotifiesAndIsStoredAsALink(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	const eli = "55555555-5555-4555-8555-555555555555"
	ahora := time.Now()
	if err := db.Exec(`INSERT INTO users (id, username, name, email, password, created_at, updated_at)
		VALUES (?, 'Eli', 'Elena Ruiz', 'e@x.io', 'x', ?, ?)`, eli, ahora, ahora).Error; err != nil {
		t.Fatal(err)
	}
	if err := db.Exec(`INSERT INTO org_memberships (org_id, user_id, role, created_at)
		VALUES ('org-1', ?, 'member', ?)`, eli, ahora).Error; err != nil {
		t.Fatal(err)
	}
	espia := &notificadorEspia{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(espia)

	body := "@eli y @Bea: revisad esto. A @dani no (otra org), ni a caro@x.io ni a `@caro`."
	m, err := svc.ReplyVia(domain.ViaMCP, "esp-1", "org-1", ana, body, "")
	if err != nil {
		t.Fatal(err)
	}
	want := "[@Elena Ruiz](cac:user/" + eli + ") y [@bea](cac:user/" + bea + "): revisad esto. " +
		"A @dani no (otra org), ni a caro@x.io ni a `@caro`."
	if m.Body != want {
		t.Errorf("se guarda con las menciones como enlace y lo demás igual:\n got %q\nwant %q", m.Body, want)
	}
	var stored string
	db.Raw(`SELECT body FROM chat_messages WHERE id = ?`, m.ID).Scan(&stored)
	if stored != want {
		t.Errorf("en la base:\n got %q\nwant %q", stored, want)
	}
	got := espia.paraQuien("chat:mention")
	slices.Sort(got)
	if want := []string{bea, eli}; !slices.Equal(got, want) {
		t.Errorf("avisa como mención a eli y a bea, y a nadie más; %v", got)
	}

	// Corregir el mensaje también: la mención añadida al editar se guarda
	// como enlace.
	if err := svc.Edit(m.ID, ana, false, "y ahora @caro"); err != nil {
		t.Fatal(err)
	}
	db.Raw(`SELECT body FROM chat_messages WHERE id = ?`, m.ID).Scan(&stored)
	if stored != "y ahora [@caro](cac:user/"+caro+")" {
		t.Errorf("al editar: %q", stored)
	}
}

// La mención que escribe la app con el menú de `@` (la nativa y la de Tauri
// mandan la forma enlace, `[@usuario](cac:user/<id>)`) sigue avisando igual y
// se guarda tal cual: la normalización no la vuelve a tocar.
func TestAMentionPickedInTheAppStillNotifies(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	espia := &notificadorEspia{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(espia)

	body := "hola [@bea](cac:user/" + bea + ") ¿vienes?"
	m, err := svc.Post("esp-1", "org-1", ana, body)
	if err != nil {
		t.Fatal(err)
	}
	if m.Body != body {
		t.Errorf("una mención que ya es enlace no cambia: %q", m.Body)
	}
	if got := espia.paraQuien("chat:mention"); !slices.Equal(got, []string{bea}) {
		t.Errorf("avisa a bea una vez; %v", got)
	}
	if strings.Count(m.Body, "cac:user/") != 1 {
		t.Errorf("ni se duplica: %q", m.Body)
	}
}

// Una línea del sistema la escribe cac, no una persona: su texto (el título de
// una grabación, por ejemplo) no se reinterpreta, y un `@bea` ahí no avisa.
func TestASystemLineIsNotRewritten(t *testing.T) {
	db, cleanup := chatFollowDB(t)
	defer cleanup()
	espia := &notificadorEspia{}
	svc := NewChatService(repository.NewChatRepository(db), nil).WithNotifier(espia)

	m, err := svc.PostSystem("esp-1", "org-1", ana, "Grabación «repaso con @bea» lista", "Grabación lista")
	if err != nil {
		t.Fatal(err)
	}
	if m.Body != "Grabación «repaso con @bea» lista" {
		t.Errorf("la línea del sistema se guarda como la escribió cac: %q", m.Body)
	}
	if got := espia.paraQuien("chat:mention"); len(got) != 0 {
		t.Errorf("y no menciona a nadie; %v", got)
	}
}
