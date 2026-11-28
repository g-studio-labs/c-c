package service

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/google/uuid"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// La búsqueda del MCP hace menos y en paralelo (command-and-control-570):
// medida contra producción, eran 28–52 consultas por búsqueda, una org detrás
// de otra.

// sqlLog guarda cada consulta que pasa por GORM.
type sqlLog struct {
	mu   sync.Mutex
	sqls []string
}

func (l *sqlLog) LogMode(logger.LogLevel) logger.Interface      { return l }
func (l *sqlLog) Info(context.Context, string, ...interface{})  {}
func (l *sqlLog) Warn(context.Context, string, ...interface{})  {}
func (l *sqlLog) Error(context.Context, string, ...interface{}) {}
func (l *sqlLog) Trace(_ context.Context, _ time.Time, fc func() (string, int64), _ error) {
	s, _ := fc()
	l.mu.Lock()
	l.sqls = append(l.sqls, s)
	l.mu.Unlock()
}

func (l *sqlLog) count(substr string) int {
	l.mu.Lock()
	defer l.mu.Unlock()
	n := 0
	for _, s := range l.sqls {
		if strings.Contains(s, substr) {
			n++
		}
	}
	return n
}

func (l *sqlLog) reset() { l.mu.Lock(); l.sqls = nil; l.mu.Unlock() }

func loggedSearch(db *gorm.DB) (*SearchService, *sqlLog) {
	l := &sqlLog{}
	return NewSearchService(repository.NewSearchRepository(db.Session(&gorm.Session{Logger: l}))), l
}

// Roturas que atrapa: que el MCP siga pidiendo personas, canal o directos
// (que tira), que la app deje de pedirlos, que las notas se busquen una vez
// por org, o que se vuelva a preguntar a information_schema en cada búsqueda.
func TestAnAgentSearchDoesNotAskForPeopleChannelOrDirectMessages(t *testing.T) {
	db, done := searchDB(t)
	defer done()
	s, l := loggedSearch(db)
	orgs := []string{"org-1", "org-2", "org-3"}

	// La app sí: lo mismo con todo.
	if r, err := s.SearchOrgsWith("bea", orgs, "u-ana", 8, domain.SearchForPeople()); err != nil || len(r.People) == 0 {
		t.Fatalf("la app tiene que encontrar a la persona: %+v %v", r.People, err)
	}
	if r, _ := s.SearchOrgsWith("canal", orgs, "u-ana", 8, domain.SearchForPeople()); len(r.Messages) == 0 {
		t.Fatalf("la app tiene que encontrar el mensaje del canal: %+v", r)
	}
	if r, _ := s.SearchOrgsWith("secreto", orgs, "u-bea", 8, domain.SearchForPeople()); len(r.DMs) == 0 {
		t.Fatalf("la app tiene que encontrar el directo: %+v", r)
	}

	for _, q := range []string{"bea", "canal", "secreto"} {
		l.reset()
		r, err := s.SearchOrgsWith(q, orgs, "u-bea", 8, domain.SearchForAgent())
		if err != nil {
			t.Fatal(err)
		}
		for _, table := range []string{"org_memberships", "chat_messages", "dm_conversations"} {
			if n := l.count(table); n > 0 {
				t.Errorf("%q: el MCP preguntó %d veces a %s, que no le llega", q, n, table)
			}
		}
		if len(r.People)+len(r.Messages)+len(r.DMs) > 0 {
			t.Errorf("%q: al MCP le llegaron personas o mensajes: %+v", q, r)
		}
		// La de texto completo (`notes n`); en esta base sin índice cae
		// después a la de LIKE, que no cuenta como otra búsqueda de notas.
		if n := l.count("FROM notes n"); n != 1 {
			t.Errorf("%q: las notas son de la persona y se buscan una vez, no %d", q, n)
		}
		if n := l.count("information_schema"); n > 0 {
			t.Errorf("%q: %d consultas a information_schema en una búsqueda que no es la primera", q, n)
		}
	}
	// Y un id pegado busca la nota una vez, no en cada org.
	l.reset()
	if r, err := s.SearchOrgsWith("0b9a3f6e-1c2d-4e5f-8a7b-9c0d1e2f3a4b", orgs, "u-ana", 8, domain.SearchForAgent()); err != nil || len(r.Notes) != 0 {
		t.Fatalf("un id que no existe: %+v %v", r, err)
	}
	if n := l.count("FROM notes WHERE id"); n != 1 {
		t.Errorf("un id pegado preguntó %d veces por la nota, una por org", n)
	}
	l.reset()
	if _, err := s.SearchWith("canal", "org-1", "u-bea", 8, domain.SearchForAgent()); err != nil {
		t.Fatal(err)
	}
	if n := l.count("chat_messages") + l.count("org_memberships") + l.count("dm_conversations"); n > 0 {
		t.Errorf("con una org, el MCP también pidió lo que tira (%d)", n)
	}
}

// Las orgs a la vez dan lo mismo, y en el mismo orden, que una detrás de otra.
//
// Roturas que atrapa: juntar en el orden en que terminan y no en el de las
// orgs, perder los resultados de alguna, o decidir la segunda vuelta por org.
func TestSearchingOrgsAtOnceGivesWhatSearchingThemInTurnGave(t *testing.T) {
	db, _ := agentSearchDB(t)
	var orgs []string
	for i := 0; i < 7; i++ {
		org := "org-" + string(rune('a'+i))
		orgs = append(orgs, org)
		for j := 0; j < 3; j++ {
			task(t, db, org, "l-back", "Despliegue del módulo "+org+" número "+uuid.NewString()[:4])
		}
		doc := &domain.Doc{OrgID: org, OwnerKind: domain.DocOwnerList, OwnerID: "l-" + org}
		doc.ID = "doc-" + org
		mustCreate(t, db, doc)
		page := &domain.DocPage{DocID: doc.ID, OrgID: org, Title: "Despliegue en " + org, Rank: "0.5", Body: "Cómo se despliega el módulo."}
		page.ID = "page-" + org
		mustCreate(t, db, page)
	}
	inTurn := NewSearchService(repository.NewSearchRepository(db))
	inTurn.orgWorkers = 1
	atOnce := NewSearchService(repository.NewSearchRepository(db))
	for _, q := range []string{"despliegue", "despliegue módulo", "despliegue inexistentexyz"} {
		for _, limit := range []int{3, 20} {
			want, err := inTurn.SearchOrgsWith(q, orgs, "u-1", limit, domain.SearchForAgent())
			if err != nil {
				t.Fatal(err)
			}
			got, err := atOnce.SearchOrgsWith(q, orgs, "u-1", limit, domain.SearchForAgent())
			if err != nil {
				t.Fatal(err)
			}
			if len(want.Tasks) == 0 || len(want.Docs) == 0 {
				t.Fatalf("%q: el caso no encuentra nada, no prueba el orden: %+v", q, want)
			}
			if !reflect.DeepEqual(got, want) {
				t.Errorf("%q (límite %d): a la vez no da lo mismo que en serie\n got: %+v\nwant: %+v", q, limit, got, want)
			}
		}
	}
}

// Roturas que atrapa: buscar las orgs una detrás de otra, buscarlas todas a
// la vez sin tope, devolverlas en el orden en que terminan, o devolver el
// fallo de otra org que no es la primera que falló.
func TestEachOrgRunsAtMostItsWorkersAtOnceAndKeepsTheOrder(t *testing.T) {
	orgs := []string{"o1", "o2", "o3", "o4", "o5", "o6"}
	var inFlight, most atomic.Int32
	started := make(chan string, len(orgs))
	release := make(chan struct{})
	type out struct {
		res []domain.SearchResults
		err error
	}
	done := make(chan out, 1)
	go func() {
		res, err := eachOrg(orgs, 3, func(org string) (domain.SearchResults, error) {
			n := inFlight.Add(1)
			for {
				m := most.Load()
				if n <= m || most.CompareAndSwap(m, n) {
					break
				}
			}
			started <- org
			<-release
			inFlight.Add(-1)
			return domain.SearchResults{Tasks: []domain.SearchHit{{ID: org}}}, nil
		})
		done <- out{res, err}
	}()
	// Tres a la vez antes de soltar ninguna: si fueran en serie, la segunda
	// no empezaría nunca.
	for i := 0; i < 3; i++ {
		select {
		case <-started:
		case <-time.After(10 * time.Second):
			t.Fatalf("sólo empezaron %d orgs a la vez, se esperaban 3", i)
		}
	}
	// Y ninguna más mientras ésas no terminen. Esperar aquí sólo puede dejar
	// pasar un fallo (una cuarta que tarda en arrancar), nunca inventar uno:
	// una cuarta que llega es que no hay tope.
	select {
	case org := <-started:
		t.Errorf("%s empezó con tres en marcha y un tope de 3", org)
	case <-time.After(300 * time.Millisecond):
	}
	close(release)
	r := <-done
	if r.err != nil {
		t.Fatal(r.err)
	}
	if m := most.Load(); m > 3 {
		t.Errorf("llegaron a ir %d orgs a la vez, con un tope de 3", m)
	}
	for i, x := range r.res {
		if len(x.Tasks) != 1 || x.Tasks[0].ID != orgs[i] {
			t.Errorf("el resultado %d no es el de %s: %+v", i, orgs[i], x)
		}
	}

	_, err := eachOrg(orgs, 3, func(org string) (domain.SearchResults, error) {
		switch org {
		case "o2":
			return domain.SearchResults{}, errors.New("falla o2")
		case "o5":
			return domain.SearchResults{}, errors.New("falla o5")
		}
		return domain.SearchResults{}, nil
	})
	if err == nil || err.Error() != "falla o2" {
		t.Errorf("tenía que salir el fallo de la primera org que falló: %v", err)
	}
}

// Las migas de las páginas que salen, en una consulta, y cada una la suya.
//
// Roturas que atrapa: volver a una consulta por página, o mezclar las migas
// de dos páginas, o darlas al revés.
func TestDocBreadcrumbsComeInOneQuery(t *testing.T) {
	db, _ := agentSearchDB(t)
	s, l := loggedSearch(db)
	doc := &domain.Doc{OrgID: "org-1", OwnerKind: domain.DocOwnerList, OwnerID: "l-back"}
	doc.ID = "doc-1"
	mustCreate(t, db, doc)
	page := func(id, parent, title, body string) {
		p := &domain.DocPage{DocID: "doc-1", OrgID: "org-1", Title: title, Rank: "0.5", Body: body}
		p.ID = id
		if parent != "" {
			p.ParentID = &parent
		}
		mustCreate(t, db, p)
	}
	page("p-root", "", "Raíz", "")
	page("p-mid", "p-root", "Medio", "")
	page("p-deep", "p-mid", "Hoja", "El despliegue va aquí.")
	page("p-other", "p-root", "Otra", "Otro despliegue.")
	page("p-top", "", "Arriba", "Un despliegue más.")

	l.reset()
	r, err := s.SearchWith("despliegue", "org-1", "u-1", 8, domain.SearchForAgent())
	if err != nil {
		t.Fatal(err)
	}
	where := map[string]string{}
	for _, h := range r.Docs {
		where[h.ID] = h.Where
	}
	for id, suffix := range map[string]string{"p-deep": " › Raíz › Medio", "p-other": " › Raíz", "p-top": ""} {
		w, ok := where[id]
		if !ok {
			t.Fatalf("%s no salió: %+v", id, r.Docs)
		}
		if !strings.HasSuffix(w, suffix) || strings.Count(w, "›") != strings.Count(suffix, "›") {
			t.Errorf("las migas de %s: %q, se esperaba «…%s»", id, w, suffix)
		}
	}
	if n := l.count("WITH RECURSIVE"); n != 1 {
		t.Errorf("las migas de tres páginas costaron %d consultas, no una", n)
	}
}

// La segunda vuelta («con alguna palabra») tampoco busca las notas en cada
// org: una vez por vuelta.
//
// Rotura que atrapa: buscar las notas de la segunda vuelta en cada org.
func TestTheSecondPassAlsoSearchesNotesOnce(t *testing.T) {
	db, _ := agentSearchDB(t)
	s, l := loggedSearch(db)
	task(t, db, "org-1", "l-back", "Despliegue nocturno")
	if r, err := s.SearchOrgsWith("despliegue inexistentexyz", []string{"org-1", "org-2", "org-3"}, "u-1", 8, domain.SearchForAgent()); err != nil || !r.Partial {
		t.Fatalf("el caso tenía que ir a la segunda vuelta: %+v %v", r, err)
	}
	if n := l.count("FROM notes n"); n != 2 {
		t.Errorf("con segunda vuelta, las notas se buscan dos veces (una por vuelta), no %d", n)
	}
}
