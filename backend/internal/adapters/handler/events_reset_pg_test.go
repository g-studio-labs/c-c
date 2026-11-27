package handler

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// El escenario de la QA 558 (criterio 6) contra el diario de verdad: ana en
// cursor 12, `TRUNCATE user_events, user_event_heads`, y la cliente comenta
// (dos eventos nuevos). Con la conexión viva y al reconectar, llega el reset
// y después el 1 y el 2; antes, el vivo los callaba y el backlog contestaba
// un `cac:gap` con el `seq` 2 que tapaba los dos.
func TestAJournalTruncatedUnderAListenerLosesNothingNew(t *testing.T) {
	withTokenAuth(t, nil)
	original := latido
	latido = time.Hour // lo tiene que ver el vivo, no el latido
	defer func() { latido = original }()

	db := resetDB(t)
	hub := events.NewHub()
	j := service.NewEventJournal(repository.NewUserEventRepository(db), hub)
	h := NewEventsHandler(hub, nil, j)
	add := func() {
		t.Helper()
		if _, _, err := j.Append(domain.UserEventInput{UserID: tokenOwner, OrgID: "org-1", Type: domain.UserEventNotification}); err != nil {
			t.Fatal(err)
		}
	}
	for i := 0; i < 12; i++ {
		add()
	}
	_, oldEpoch, _ := j.Head(tokenOwner)

	r, _ := openStream(t, h, goodToken, "12")
	time.Sleep(50 * time.Millisecond)
	if err := db.Exec("TRUNCATE user_events, user_event_heads").Error; err != nil {
		t.Fatal(err)
	}
	add()
	add()

	if got := r.next(t); !got.env.IsReset() {
		t.Fatalf("con la conexión viva, el reinicio se calló: %+v", got)
	}
	for want := int64(1); want <= 2; want++ {
		if got := r.next(t); got.env.Seq != want || got.env.Type != domain.UserEventNotification {
			t.Fatalf("con la conexión viva se esperaba el %d del diario nuevo, llegó %+v", want, got)
		}
	}

	// Al reconectar: desde el 12 con la época vieja o sin ella (la cabecera
	// lo dice), y desde el 1 con la vieja (sólo la época lo dice).
	for _, ask := range []struct {
		after int
		epoch string
	}{{12, oldEpoch}, {12, ""}, {1, oldEpoch}} {
		epoch := ask.epoch
		rec := httptest.NewRecorder()
		claims := &domain.ClaimsJWT{UserID: tokenOwner, ViaToken: true, Scopes: []string{domain.ScopeEventsRead},
			Orgs: []domain.OrgMembershipClaim{{OrgID: "org-1"}}}
		req := httptest.NewRequest(http.MethodGet, fmt.Sprintf("/api/v1/events/backlog?after=%d&epoch=%s", ask.after, epoch), nil)
		h.Backlog(rec, req.WithContext(context.WithValue(req.Context(), repository.UserContextKey, claims)))
		var body domain.APIResponse[domain.EventPage]
		if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
			t.Fatalf("%d %s", rec.Code, rec.Body.String())
		}
		var got []string
		for _, e := range body.Data.Events {
			got = append(got, fmt.Sprintf("%d:%s", e.Seq, e.Type))
		}
		want := "[0:cac:gap 1:notification:new 2:notification:new]"
		if fmt.Sprint(got) != want || !body.Data.Events[0].IsReset() || body.Data.Epoch == "" || body.Data.Epoch == oldEpoch {
			t.Fatalf("desde %d, época %q: al reconectar se esperaba %s con la época nueva, llegó %v (epoch %q)", ask.after, epoch, want, got, body.Data.Epoch)
		}
	}
}

// El caso de todos los días: el diario se vacía y el primer evento nuevo llega
// varios latidos después. Tras el TRUNCATE no hay cabecera, así que el reset
// sale sin época. Roturas que atrapa: que la conexión se quede con la época
// vieja (el latido de después del primer evento ve otra y anuncia el reinicio
// otra vez, con el 1 repetido: medido en la revisión 558-G como
// `[0:reset 1 0:reset 1 2]`).
func TestAJournalTruncatedBeforeItsFirstNewEventRestartsOnce(t *testing.T) {
	withTokenAuth(t, nil)
	original := latido
	latido = 30 * time.Millisecond
	defer func() { latido = original }()

	db := resetDB(t)
	hub := events.NewHub()
	j := service.NewEventJournal(repository.NewUserEventRepository(db), hub)
	h := NewEventsHandler(hub, nil, j)
	add := func() {
		t.Helper()
		if _, _, err := j.Append(domain.UserEventInput{UserID: tokenOwner, OrgID: "org-1", Type: domain.UserEventNotification}); err != nil {
			t.Fatal(err)
		}
	}
	for i := 0; i < 12; i++ {
		add()
	}
	_, oldEpoch, _ := j.Head(tokenOwner)

	r, _ := openStreamAt(t, h, goodToken, "12", "epoch="+oldEpoch)
	time.Sleep(50 * time.Millisecond)
	if err := db.Exec("TRUNCATE user_events, user_event_heads").Error; err != nil {
		t.Fatal(err)
	}
	if got := r.next(t); !got.env.IsReset() {
		t.Fatalf("el latido no vio el reinicio: %+v", got)
	}
	// Varios latidos con el diario vacío y sin época: nada más que decir.
	r.quiet(t, 200*time.Millisecond)
	add()
	add()
	for want := int64(1); want <= 2; want++ {
		if got := r.next(t); got.env.Seq != want || got.env.Type != domain.UserEventNotification {
			t.Fatalf("se esperaba el %d del diario nuevo una vez, llegó %+v", want, got)
		}
	}
	// Los latidos de después ven la época nueva: no es otro reinicio.
	r.quiet(t, 200*time.Millisecond)
}

func resetDB(t *testing.T) *gorm.DB {
	t.Helper()
	if repository.GetEnv("DB_HOST", "") == "" {
		t.Skip("no database configured")
	}
	dsn := func(name string) string {
		return fmt.Sprintf("host=%s port=%s user=%s password=%s dbname=%s sslmode=%s",
			repository.GetEnv("DB_HOST", "localhost"), repository.GetEnv("DB_PORT", "5432"),
			repository.GetEnv("DB_USER", "postgres"), repository.GetEnv("DB_PASSWORD", ""),
			name, repository.GetEnv("DB_SSLMODE", "disable"))
	}
	admin, err := gorm.Open(postgres.Open(dsn(repository.GetEnv("DB_NAME", "cac"))), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatalf("DB_HOST is set but the database does not answer (a skipped test is not a green one): %v", err)
	}
	const name = "cac_test_events_reset"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	adminSQL, _ := admin.DB()
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		adminSQL.Close()
	})
	if err := db.AutoMigrate(&domain.UserEvent{}, &domain.UserEventHead{}); err != nil {
		t.Fatal(err)
	}
	return db
}
