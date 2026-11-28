package handler

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// «Por atender» por páginas (command-and-control-569): un cursor sobre el
// mismo orden de siempre, con filtro de tipo y de texto en la misma consulta.

// sqlCapture guarda la última consulta, para pedirle su EXPLAIN a Postgres.
type sqlCapture struct {
	logger.Interface
	last string
}

func (c *sqlCapture) Trace(_ context.Context, _ time.Time, fc func() (string, int64), _ error) {
	sql, _ := fc()
	c.last = sql
}

type requestPageRig struct {
	db      *gorm.DB
	h       *RequestHandler
	capture *sqlCapture
	repo    *repository.RequestRepository
}

func newRequestPageRig(t *testing.T) *requestPageRig {
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
	name := fmt.Sprintf("cac_test_request_pages_%d", time.Now().UnixNano()%1_000_000)
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
	if err := db.AutoMigrate(&domain.Organization{}, &domain.TaskSpace{}, &domain.TaskList{}, &domain.Item{}, &domain.User{},
		&domain.OrgMembership{}, &domain.ReportProject{}, &domain.Request{}, &domain.RequestVersion{}); err != nil {
		t.Fatal(err)
	}
	if err := repository.EnsureRequestIndexes(db); err != nil {
		t.Fatal(err)
	}
	sp := &domain.TaskSpace{OrgID: "org-1", Name: "Command and control", Key: "cac", Rank: "0.5"}
	sp.ID = "space-1"
	li := &domain.TaskList{SpaceID: "space-1", Name: "App", Rank: "0.5"}
	li.ID = "list-1"
	rows := []any{sp, li}
	for _, u := range []struct{ id, user, name string }{{"u-ana", "ana", "Ana Ruiz"}, {"u-bea", "bea", "Bea Gil"}} {
		usr := &domain.User{Username: u.user, Name: u.name, Password: "x"}
		usr.ID = u.id
		rows = append(rows, usr)
	}
	// it-0 baja, it-1 urgente, it-2 media: el orden de los abiertos sale de aquí.
	for i, p := range []domain.ItemPriority{domain.ItemPriorityLow, domain.ItemPriorityUrgent, domain.ItemPriorityMedium} {
		it := &domain.Item{OrgID: "org-1", SpaceID: "space-1", ListID: "list-1", Seq: 10 + i,
			Title: fmt.Sprintf("Tarea %d", i), Priority: p, Status: domain.ReportInProgress}
		it.ID = fmt.Sprintf("it-%d", i)
		rows = append(rows, it)
	}
	for _, r := range rows {
		if err := db.Create(r).Error; err != nil {
			t.Fatal(err)
		}
	}
	capture := &sqlCapture{Interface: logger.Discard}
	repo := repository.NewRequestRepository(db.Session(&gorm.Session{Logger: capture}))
	svc := service.NewRequestService(repo, repository.NewTaskRepository(db), repository.NewDocRepository(db), nil, nil)
	return &requestPageRig{db: db, h: NewRequestHandler(svc), capture: capture, repo: repo}
}

var requestSeq int

// add: un pedido de `task`, a `of`, creado en `created`; cerrado en `closed`
// (contestado) si se da. Directo a la base: así se repiten las fechas.
func (r *requestPageRig) add(t *testing.T, task, of string, kind domain.RequestKind, summary string, created time.Time, closed *time.Time) string {
	t.Helper()
	requestSeq++
	q := &domain.Request{
		Code: fmt.Sprintf("req-t%05d", requestSeq), OrgID: "org-1", TaskID: task, Kind: kind, Summary: summary,
		Body: []byte(`{}`), RequestedOfID: of, RequestedByID: "u-bea", Status: domain.RequestOpen,
	}
	q.CreatedAt, q.UpdatedAt = created, created
	if closed != nil {
		q.Status, q.AnsweredAt, q.UpdatedAt = domain.RequestAnswered, closed, *closed
	}
	if err := r.db.Create(q).Error; err != nil {
		t.Fatal(err)
	}
	return q.Code
}

func (r *requestPageRig) get(t *testing.T, query url.Values) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/v1/requests?"+query.Encode(), nil)
	req = req.WithContext(context.WithValue(req.Context(), repository.UserContextKey, claims("u-ana", "org-1", domain.OrgRoleMember)))
	rec := httptest.NewRecorder()
	r.h.List(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET %s → %d: %s", query.Encode(), rec.Code, rec.Body.String())
	}
	return rec
}

// walk recorre todas las páginas y devuelve los códigos en orden, y cuántas
// páginas hizo falta pedir.
func (r *requestPageRig) walk(t *testing.T, query url.Values) ([]string, int) {
	t.Helper()
	var codes []string
	query.Set("cursor", "")
	for pages := 1; pages < 50; pages++ {
		page := decode[domain.RequestPage](t, r.get(t, query))
		for _, v := range page.Requests {
			codes = append(codes, v.Code)
		}
		if page.Next == "" {
			return codes, pages
		}
		query.Set("cursor", page.Next)
	}
	t.Fatal("el cursor no se acaba nunca")
	return nil, 0
}

func noRepeats(t *testing.T, what string, codes []string, want []string) {
	t.Helper()
	seen := map[string]int{}
	for _, c := range codes {
		seen[c]++
	}
	for c, n := range seen {
		if n > 1 {
			t.Errorf("%s: %s sale %d veces: %v", what, c, n, codes)
		}
	}
	if fmt.Sprint(codes) != fmt.Sprint(want) {
		t.Errorf("%s:\n got  %v\n want %v", what, codes, want)
	}
}

// Las páginas no repiten ni se saltan nada, aunque varios pedidos tengan la
// misma fecha, y cruzan de abiertos a resueltos sin perder el orden: urgente
// antes que media antes que baja, el más viejo primero; los resueltos, el más
// reciente primero. Sin cursor, contesta como siempre: una lista.
//
// Roturas que atrapa: un cursor sin el id de desempate (con la misma fecha
// repite o salta); comparar el id en el sentido contrario al del orden; perder
// la urgencia al pasar de página; que el cursor de un abierto deje fuera los
// resueltos (o los vuelva a traer); contestar con `{requests,next}` a quien no
// manda cursor; no decir que hay más.
func TestRequestPagesNeitherRepeatNorSkipWhenDatesTie(t *testing.T) {
	r := newRequestPageRig(t)
	base := time.Date(2026, 11, 20, 10, 0, 0, 0, time.UTC)
	same := base.Add(time.Hour)
	closedAt := base.Add(48 * time.Hour)
	older := closedAt.Add(-time.Hour)

	// Abiertos: tres urgentes con la misma fecha, una media, dos bajas (una más vieja).
	u1 := r.add(t, "it-1", "u-ana", domain.RequestTest, "u1", same, nil)
	u2 := r.add(t, "it-1", "u-ana", domain.RequestTest, "u2", same, nil)
	u3 := r.add(t, "it-1", "u-ana", domain.RequestTest, "u3", same, nil)
	m1 := r.add(t, "it-2", "u-ana", domain.RequestTest, "m1", base, nil)
	l2 := r.add(t, "it-0", "u-ana", domain.RequestTest, "l2", same, nil)
	l1 := r.add(t, "it-0", "u-ana", domain.RequestTest, "l1", base, nil)
	// Resueltos: tres cerrados a la vez y uno antes.
	c1 := r.add(t, "it-0", "u-ana", domain.RequestTest, "c1", base, &closedAt)
	c2 := r.add(t, "it-1", "u-ana", domain.RequestTest, "c2", base, &closedAt)
	c3 := r.add(t, "it-2", "u-ana", domain.RequestTest, "c3", base, &closedAt)
	c4 := r.add(t, "it-0", "u-ana", domain.RequestTest, "c4", base, &older)
	// De Bea: en «Míos» de Ana no sale.
	r.add(t, "it-1", "u-bea", domain.RequestTest, "b1", base, nil)

	// Los empates se deshacen por id; el id es un uuid al azar, así que el
	// orden esperado de cada empate se lee de la base, no se inventa.
	byID := func(dir string, codes ...string) []string {
		var out []string
		r.db.Raw("SELECT code FROM requests WHERE code IN ? ORDER BY id "+dir, codes).Scan(&out)
		return out
	}
	wantOpen := append(append(byID("ASC", u1, u2, u3), m1, l1), l2)
	wantClosed := append(byID("DESC", c1, c2, c3), c4)

	for _, limit := range []string{"1", "2", "3", "4"} {
		got, pages := r.walk(t, url.Values{"scope": {"mine"}, "status": {"all"}, "limit": {limit}})
		noRepeats(t, "todo, de "+limit+" en "+limit, got, append(append([]string{}, wantOpen...), wantClosed...))
		if limit == "1" && pages != 10 {
			t.Errorf("de uno en uno son diez páginas, la última sin `next`: %d", pages)
		}
		got, _ = r.walk(t, url.Values{"scope": {"mine"}, "status": {"closed"}, "limit": {limit}})
		noRepeats(t, "resueltos, de "+limit+" en "+limit, got, wantClosed)
	}

	// Sin cursor, la lista de siempre.
	rec := r.get(t, url.Values{"scope": {"mine"}, "status": {"open"}})
	var legacy struct {
		Data []domain.RequestView `json:"data"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &legacy); err != nil || len(legacy.Data) != 6 {
		t.Fatalf("sin cursor contesta una lista de seis: %v %s", err, rec.Body.String())
	}
}

// El filtro va en la misma consulta que el cursor: la página dos de lo
// filtrado no trae lo que no pasa el filtro. El texto encuentra por código,
// por resumen, por título y por la ref de la tarea.
//
// Roturas que atrapa: no mandar `kind` a la consulta; aplicar el filtro sólo
// a la primera página; buscar el código distinguiendo mayúsculas; no
// encontrar por la ref (que no se guarda en ninguna columna); que una ref
// encuentre también los de otras tareas.
func TestRequestFiltersCombineWithTheCursor(t *testing.T) {
	r := newRequestPageRig(t)
	base := time.Date(2026, 11, 20, 10, 0, 0, 0, time.UTC)
	var decides []string
	for i := 0; i < 5; i++ {
		at := base.Add(time.Duration(i) * time.Minute)
		r.add(t, "it-1", "u-ana", domain.RequestTest, "probar", at, nil)
		decides = append(decides, r.add(t, "it-1", "u-ana", domain.RequestDecide, "decidir", at, nil))
	}
	got, pages := r.walk(t, url.Values{"scope": {"mine"}, "kind": {"decide"}, "limit": {"2"}})
	noRepeats(t, "sólo «decidir»", got, decides)
	if pages != 3 {
		t.Errorf("cinco de dos en dos son tres páginas: %d", pages)
	}

	runs := r.add(t, "it-2", "u-ana", domain.RequestRun, "migrar la base", base, nil)
	got, _ = r.walk(t, url.Values{"scope": {"mine"}, "kind": {"run,decide"}, "limit": {"100"}})
	if len(got) != 6 {
		t.Errorf("dos tipos a la vez: %v", got)
	}

	search := func(q string) []string {
		got, _ := r.walk(t, url.Values{"scope": {"mine"}, "q": {q}, "limit": {"2"}})
		return got
	}
	if got := search(strings.ToUpper(runs)); fmt.Sprint(got) != fmt.Sprint([]string{runs}) {
		t.Errorf("por código, en mayúsculas: %v", got)
	}
	if got := search("Migrar"); fmt.Sprint(got) != fmt.Sprint([]string{runs}) {
		t.Errorf("por resumen: %v", got)
	}
	if got := search("cac-12"); fmt.Sprint(got) != fmt.Sprint([]string{runs}) {
		t.Errorf("por la ref de su tarea (it-2 es cac-12): %v", got)
	}
	if got := search("tarea 2"); fmt.Sprint(got) != fmt.Sprint([]string{runs}) {
		t.Errorf("por el título de su tarea: %v", got)
	}
	if got := search("cac-11"); len(got) != 10 {
		t.Errorf("la ref de it-1 trae sus diez y no el de it-2: %v", got)
	}
	if got := search("nada-que-ver"); len(got) != 0 {
		t.Errorf("sin resultados: %v", got)
	}
}

// Una página de resueltos se lee del índice ya ordenada y para en la página,
// en vez de recorrer y ordenar todos los cerrados. Con pocas filas Postgres
// prefiere recorrer y ordenar aunque haya índice, así que se le quitan esas
// dos opciones: si ningún índice da el orden, lo hace igual, y se ve en el plan.
//
// Roturas que atrapa: borrar el índice; cambiar la expresión del orden (o
// ordenar con los CASE de la lista mezclada) de modo que deje de casar con él.
func TestAPageOfResolvedRequestsUsesItsIndex(t *testing.T) {
	r := newRequestPageRig(t)
	base := time.Date(2026, 11, 20, 10, 0, 0, 0, time.UTC)
	for i := 0; i < 30; i++ {
		at := base.Add(time.Duration(i) * time.Minute)
		r.add(t, "it-1", "u-ana", domain.RequestTest, "x", base, &at)
	}
	_, next, err := r.repo.Page(repository.RequestFilter{OrgIDs: []string{"org-1"}, RequestedOf: "u-ana",
		Statuses: []domain.RequestStatus{domain.RequestAnswered, domain.RequestWithdrawn}, Limit: 5}, "")
	if err != nil || next == "" {
		t.Fatalf("primera página: %v %q", err, next)
	}
	if _, _, err := r.repo.Page(repository.RequestFilter{OrgIDs: []string{"org-1"}, RequestedOf: "u-ana",
		Statuses: []domain.RequestStatus{domain.RequestAnswered, domain.RequestWithdrawn}, Limit: 5}, next); err != nil {
		t.Fatal(err)
	}
	sql := r.capture.last
	var plan []string
	err = r.db.Transaction(func(tx *gorm.DB) error {
		for _, off := range []string{"enable_seqscan", "enable_sort"} {
			if err := tx.Exec("SET LOCAL " + off + " = off").Error; err != nil {
				return err
			}
		}
		return tx.Raw("EXPLAIN " + sql).Scan(&plan).Error
	})
	if err != nil {
		t.Fatal(err)
	}
	// Cuál de los dos (persona u organización) lo decide la estadística.
	if text := strings.Join(plan, "\n"); !strings.Contains(text, "idx_requests_resolved_") || strings.Contains(text, "Sort") {
		t.Fatalf("la página no sale ordenada de idx_requests_resolved_*:\n%s\n%s", sql, text)
	}
}
