package service

import (
	"encoding/json"
	"fmt"
	"sort"
	"sync"
	"testing"
	"time"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// El diario numera por persona y sin huecos, aunque añadan muchos a la vez.
// Con un número que se pisa, una de las inserciones choca y su evento se
// pierde; con un número que salta, quien escucha pide para siempre el que no
// existe.
func TestSequencesHaveNoGapsUnderConcurrentAppends(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	j := NewEventJournal(repository.NewUserEventRepository(db), nil)

	const n = 50
	var wg sync.WaitGroup
	errs := make(chan error, n)
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			if _, ok, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification, Data: i}); err != nil || !ok {
				errs <- fmt.Errorf("append %d: ok=%v err=%v", i, ok, err)
			}
		}(i)
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Error(err)
	}

	page, err := j.Read("u-ana", 0, 500, nil, "")
	if err != nil {
		t.Fatal(err)
	}
	if page.Head != n || len(page.Events) != n {
		t.Fatalf("head %d con %d eventos, se esperaban %d y %d", page.Head, len(page.Events), n, n)
	}
	for i, e := range page.Events {
		if e.Seq != int64(i+1) {
			t.Fatalf("el evento %d tiene seq %d: hay un hueco o un repetido", i, e.Seq)
		}
	}
}

// Un webhook reintentado es un evento: la segunda entrega no apunta nada ni
// gasta un número.
func TestARepeatedKeyIsOneEvent(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	j := NewEventJournal(repository.NewUserEventRepository(db), nil)

	var wg sync.WaitGroup
	var mu sync.Mutex
	created := 0
	for i := 0; i < 10; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, ok, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventBasecamp, Key: "bc:comment_created:123"})
			if err != nil {
				t.Error(err)
			}
			if ok {
				mu.Lock()
				created++
				mu.Unlock()
			}
		}()
	}
	wg.Wait()
	// Otra persona con la misma clave sí tiene el suyo.
	if _, ok, err := j.Append(domain.UserEventInput{UserID: "u-bea", Type: domain.UserEventBasecamp, Key: "bc:comment_created:123"}); err != nil || !ok {
		t.Errorf("la misma clave para otra persona no se apuntó: %v %v", ok, err)
	}

	if created != 1 {
		t.Errorf("se apuntaron %d, se esperaba 1", created)
	}
	if head, _, _ := j.Head("u-ana"); head != 1 {
		t.Errorf("la cabecera está en %d: una entrega repetida gastó un número", head)
	}
}

// Lo de una organización de la que ya no es miembro no se le enseña, pero su
// número sí, para que el cursor siga seguido.
func TestTheBacklogIsOnlyOfCurrentOrgs(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	j := NewEventJournal(repository.NewUserEventRepository(db), nil)
	for _, org := range []string{"org-1", "org-2", ""} {
		if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", OrgID: org, Type: domain.UserEventNotification,
			Data: map[string]string{"title": "secreto de " + org}}); err != nil {
			t.Fatal(err)
		}
	}

	page, err := j.Read("u-ana", 0, 10, map[string]bool{"org-1": true}, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Events) != 3 {
		t.Fatalf("se esperaban 3 sobres, hay %d", len(page.Events))
	}
	if page.Events[0].Type != domain.UserEventNotification || page.Events[2].Type != domain.UserEventNotification {
		t.Errorf("lo de su org y lo personal tienen que llegar: %+v", page.Events)
	}
	skipped := page.Events[1]
	raw, _ := json.Marshal(skipped)
	if skipped.Type != domain.UserEventSkipped || skipped.Seq != 2 || skipped.OrgID != "" || string(skipped.Data) != "null" {
		t.Errorf("lo de org-2 no queda en su número y nada más: %s", raw)
	}
}

// Un cursor más viejo que lo que se guarda lo dice, con un sobre que ocupa el
// último número perdido.
func TestAnOldCursorSaysThereIsAGap(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	repo := repository.NewUserEventRepository(db)
	j := NewEventJournal(repo, nil)
	for i := 0; i < 5; i++ {
		if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification}); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := repo.Prune(time.Now().Add(-time.Hour), 2); err != nil {
		t.Fatal(err)
	}

	page, err := j.Read("u-ana", 1, 10, nil, "")
	if err != nil {
		t.Fatal(err)
	}
	if !page.Gap || len(page.Events) != 3 || page.Events[0].Type != domain.UserEventGap || page.Events[0].Seq != 3 {
		t.Fatalf("un cursor viejo no avisa del hueco en el número 3: %+v", page)
	}
	if page.Events[1].Seq != 4 || page.Events[2].Seq != 5 {
		t.Errorf("después del hueco tiene que venir lo que queda: %+v", page.Events)
	}

	page, err = j.Read("u-ana", 3, 10, nil, "")
	if err != nil {
		t.Fatal(err)
	}
	if page.Gap || len(page.Events) != 2 {
		t.Errorf("un cursor al día no tiene hueco: %+v", page)
	}
}

// Un aviso deja su evento en el diario de quien lo recibe, publicado con su
// número; lo que esa persona calló en la campana, no.
func TestNotifyLeavesAnEvent(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	hub := events.NewHub()
	ch, unsubscribe := hub.SubscribePersonal("u-ana")
	defer unsubscribe()
	SetEventJournal(NewEventJournal(repository.NewUserEventRepository(db), hub))
	defer SetEventJournal(nil)
	svc := NewNotificationService(repository.NewNotificationRepository(db))

	prefs := domain.DefaultPrefs("u-ana")
	prefs.DMs = false
	if err := svc.SavePrefs(prefs); err != nil {
		t.Fatal(err)
	}
	svc.Notify(domain.Aviso{UserID: "u-ana", OrgID: "org-1", Kind: "dm:message", Title: "callado", Link: "/dm"})
	svc.Notify(domain.Aviso{UserID: "u-ana", OrgID: "org-1", Kind: "chat:mention", Title: "Te nombraron", Link: "/chat",
		SourceKey: "bc:comment_created:9", Detail: map[string]string{"text": "el texto entero"}})

	page, err := journalOut.Read("u-ana", 0, 10, map[string]bool{"org-1": true}, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Events) != 1 {
		t.Fatalf("se esperaba sólo el aviso que no calló, hay %d", len(page.Events))
	}
	e := page.Events[0]
	var data map[string]any
	_ = json.Unmarshal(e.Data, &data)
	if e.Type != domain.UserEventNotification || !e.Mine || e.OrgID != "org-1" ||
		data["title"] != "Te nombraron" || data["sourceKey"] != "bc:comment_created:9" || data["notificationId"] == "" {
		t.Errorf("el evento no lleva el aviso: %+v %v", e, data)
	}
	if detail, _ := data["detail"].(map[string]any); detail["text"] != "el texto entero" {
		t.Errorf("el detalle no viaja: %v", data["detail"])
	}
	select {
	case ev := <-ch:
		if ev.Seq != 1 || ev.UserID != "u-ana" {
			t.Errorf("se publicó sin número o a otra persona: %+v", ev)
		}
	case <-time.After(time.Second):
		t.Error("el aviso no se publicó en el hub")
	}
}

// La retención: lo viejo y lo que pasa del máximo por persona se va, sin
// tocar la cabecera.
func TestThePruneKeepsTheNewest(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	repo := repository.NewUserEventRepository(db)
	j := NewEventJournal(repo, nil)
	j.now = func() time.Time { return time.Now().Add(-40 * 24 * time.Hour) }
	if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification}); err != nil {
		t.Fatal(err)
	}
	j.now = time.Now
	for i := 0; i < 3; i++ {
		if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification}); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := j.Prune(time.Now()); err != nil {
		t.Fatal(err)
	}
	rows, _, _ := repo.After("u-ana", 0, 10)
	var seqs []int
	for _, r := range rows {
		seqs = append(seqs, int(r.Seq))
	}
	sort.Ints(seqs)
	if fmt.Sprint(seqs) != "[2 3 4]" {
		t.Errorf("quedan %v, se esperaba [2 3 4]", seqs)
	}
	if head, _, _ := j.Head("u-ana"); head != 4 {
		t.Errorf("la cabecera retrocedió a %d", head)
	}
}

func journalDB(t *testing.T) (*gorm.DB, func()) {
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
	const name = "cac_test_event_journal"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	adminSQL, _ := admin.DB()
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	if err := db.AutoMigrate(&domain.Notification{}, &domain.NotificationPrefs{},
		&domain.UserEvent{}, &domain.UserEventHead{}); err != nil {
		t.Fatal(err)
	}
	return db, func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		adminSQL.Close()
	}
}

// Un diario que vuelve a empezar (QA 558, criterio 6). Tras un `TRUNCATE`
// el diario nuevo numera desde el 1, por debajo del cursor de quien escuchaba:
// hay que avisarle con un reset que no ocupe el número de un evento, y darle
// todo lo nuevo desde el 1. Y si el diario nuevo ya pasó del cursor, sólo la
// época lo delata.
func TestAJournalThatStartsAgainIsReadFromOne(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	j := NewEventJournal(repository.NewUserEventRepository(db), nil)
	add := func(n int) {
		for i := 0; i < n; i++ {
			if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification}); err != nil {
				t.Fatal(err)
			}
		}
	}
	seqs := func(p domain.EventPage) []int64 {
		out := []int64{}
		for _, e := range p.Events {
			out = append(out, e.Seq)
		}
		return out
	}
	add(12)
	_, before, _ := j.Head("u-ana")
	if before == "" {
		t.Fatal("un diario nuevo no tiene época")
	}

	if err := db.Exec("TRUNCATE user_events, user_event_heads").Error; err != nil {
		t.Fatal(err)
	}
	add(2)
	_, after, _ := j.Head("u-ana")
	if after == "" || after == before {
		t.Fatalf("vaciar el diario no le dio otra época: %q → %q", before, after)
	}

	// La cabecera (2) por debajo del cursor (12), con o sin época.
	for _, epoch := range []string{before, ""} {
		page, err := j.Read("u-ana", 12, 10, nil, epoch)
		if err != nil {
			t.Fatal(err)
		}
		if len(page.Events) == 0 || !page.Events[0].IsReset() || page.Epoch != after {
			t.Fatalf("época %q: un diario reiniciado no empieza por el reset: %+v", epoch, page)
		}
		if got := fmt.Sprint(seqs(page)); got != "[0 1 2]" {
			t.Fatalf("época %q: tras el reset se esperaba [0 1 2], llegó %s", epoch, got)
		}
		var data map[string]any
		_ = json.Unmarshal(page.Events[0].Data, &data)
		if data["reason"] != domain.GapReasonReset || data["previousCursor"] != float64(12) {
			t.Errorf("el reset no dice por qué ni por dónde se iba: %s", page.Events[0].Data)
		}
	}

	// El diario nuevo ya pasó del cursor: sólo la época lo dice.
	add(3)
	page, err := j.Read("u-ana", 4, 10, nil, before)
	if err != nil {
		t.Fatal(err)
	}
	if got := fmt.Sprint(seqs(page)); got != "[0 1 2 3 4 5]" || !page.Events[0].IsReset() {
		t.Fatalf("con otra época y la cabecera por delante se esperaba el reset y [1..5]: %s %+v", got, page)
	}

	// Con la época de hoy, o sin época, un cursor al día no es un reinicio.
	for _, epoch := range []string{after, ""} {
		page, err := j.Read("u-ana", 4, 10, nil, epoch)
		if err != nil || fmt.Sprint(seqs(page)) != "[5]" || page.Gap {
			t.Errorf("época %q: un cursor del diario de hoy se tomó por reinicio: %+v err=%v", epoch, page, err)
		}
	}
}

// Las filas de cabecera de antes de la columna no tienen época: la reciben al
// apuntar el siguiente evento, sin mover el número.
func TestAHeadWithoutEpochGetsOneOnTheNextAppend(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	j := NewEventJournal(repository.NewUserEventRepository(db), nil)
	if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification}); err != nil {
		t.Fatal(err)
	}
	db.Exec("UPDATE user_event_heads SET epoch = ''")
	if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification}); err != nil {
		t.Fatal(err)
	}
	head, epoch, _ := j.Head("u-ana")
	if head != 2 || epoch == "" {
		t.Fatalf("la cabecera vieja no recibió época: head=%d epoch=%q", head, epoch)
	}
}

// La hora de cada evento se toma con la cabecera bloqueada: `created_at` va en
// el orden de `seq`, y la retención por edad no corta en medio.
func TestCreatedAtFollowsSeqUnderConcurrentAppends(t *testing.T) {
	db, cleanup := journalDB(t)
	defer cleanup()
	j := NewEventJournal(repository.NewUserEventRepository(db), nil)
	var tick int64
	var mu sync.Mutex
	base := time.Now().Add(-time.Hour)
	j.now = func() time.Time {
		mu.Lock()
		defer mu.Unlock()
		tick++
		return base.Add(time.Duration(tick) * time.Millisecond)
	}

	const n = 50
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, _, err := j.Append(domain.UserEventInput{UserID: "u-ana", Type: domain.UserEventNotification}); err != nil {
				t.Error(err)
			}
		}()
	}
	wg.Wait()

	var rows []struct {
		Seq       int64
		CreatedAt time.Time
	}
	if err := db.Raw("SELECT seq, created_at FROM user_events WHERE user_id = ? ORDER BY seq", "u-ana").Scan(&rows).Error; err != nil {
		t.Fatal(err)
	}
	if len(rows) != n {
		t.Fatalf("%d filas, se esperaban %d", len(rows), n)
	}
	for i := 1; i < len(rows); i++ {
		if rows[i].CreatedAt.Before(rows[i-1].CreatedAt) {
			t.Fatalf("seq %d tiene created_at anterior al de seq %d", rows[i].Seq, rows[i-1].Seq)
		}
	}
}
