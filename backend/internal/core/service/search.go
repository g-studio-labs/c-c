package service

import (
	"strings"
	"sync"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

type SearchService struct {
	repo *repository.SearchRepository
	// orgWorkers: cuántas orgs se buscan a la vez en SearchOrgs.
	orgWorkers int
}

// searchOrgWorkers: el tope de orgs a la vez. Cada una son cuatro o cinco
// consultas seguidas y el pool de la base es de 20 conexiones: con cuatro,
// una búsqueda de un token con muchas orgs no deja sin conexión al resto.
const searchOrgWorkers = 4

func NewSearchService(repo *repository.SearchRepository) *SearchService {
	return &SearchService{repo: repo, orgWorkers: searchOrgWorkers}
}

// Search asks each source separately and hands back the answers still apart.
//
// The caller's identity goes to every source and the organization only to the
// ones the organization governs, plus direct messages: notes answer to their
// owner alone, and a direct message answers to its two participants **and** to
// the org it was written in — the fence is still the participants (the query
// starts from them), the org only narrows it to the one on screen.
//
// A short query returns nothing rather than everything. One or two letters
// match most of a database, which is a slow way to be useless.
func (s *SearchService) Search(query, orgID, userID string, limit int) (domain.SearchResults, error) {
	return s.SearchWith(query, orgID, userID, limit, domain.SearchForPeople())
}

// SearchWith: Search con las fuentes que se piden (ver domain.SearchOptions).
func (s *SearchService) SearchWith(query, orgID, userID string, limit int, opts domain.SearchOptions) (domain.SearchResults, error) {
	if limit <= 0 || limit > 20 {
		limit = 8
	}
	p := s.plan(query)
	out, err := s.searchWith(query, p, orgID, userID, limit, opts, true)
	if err != nil || !out.NothingForAnAgent() || p.loose == "" {
		return out, err
	}
	l, err := s.looseWith(query, p.loose, orgID, userID, limit, true)
	if err != nil || l.NothingForAnAgent() {
		return out, err
	}
	out.Tasks, out.Notes, out.Docs, out.Partial = l.Tasks, l.Notes, l.Docs, true
	return out, nil
}

// searchPlan: cómo se busca una consulta. `id`, si es un id pegado (se busca
// la cosa que se llama así, nada más); `strict`, todas las palabras, cada
// una también por su raíz; `loose`, alguna de ellas, para la segunda vuelta
// (sólo con más de una palabra). Ver domain.SearchTSQueryWith.
type searchPlan struct{ id, strict, loose string }

func (s *SearchService) plan(query string) searchPlan {
	if len(strings.TrimSpace(query)) < 2 {
		return searchPlan{}
	}
	if id, ok := domain.SearchID(query); ok {
		return searchPlan{id: id}
	}
	stems := s.repo.Stems(domain.SearchStemWords(query))
	p := searchPlan{strict: domain.SearchTSQueryWith(query, stems, false)}
	if domain.SearchWordCount(query) > 1 {
		p.loose = domain.SearchTSQueryWith(query, stems, true)
	}
	return p
}

// searchWith: la primera vuelta en una organización. `notes`: si se buscan
// las notas aquí; SearchOrgs las busca una vez aparte, porque son de la
// persona y no de la org.
func (s *SearchService) searchWith(query string, p searchPlan, orgID, userID string, limit int, opts domain.SearchOptions, notes bool) (domain.SearchResults, error) {
	var out domain.SearchResults
	out.Tasks, out.Notes = []domain.SearchHit{}, []domain.SearchHit{}
	out.People, out.Messages, out.DMs = []domain.SearchHit{}, []domain.SearchHit{}, []domain.SearchHit{}
	out.Docs = []domain.SearchHit{}

	if len(strings.TrimSpace(query)) < 2 {
		return out, nil
	}
	if p.id != "" {
		uid := userID
		if !notes {
			uid = ""
		}
		out.Tasks, out.Notes, out.Docs = s.repo.ByID(p.id, orgID, uid)
		stampOrg(orgID, out.Tasks, out.Docs)
		return out, nil
	}

	var err error
	if out.Tasks, err = s.repo.TasksTS(query, p.strict, orgID, limit); err != nil {
		return out, err
	}
	if notes {
		if out.Notes, err = s.repo.NotesTS(query, p.strict, userID, limit); err != nil {
			return out, err
		}
	}
	if opts.People {
		if out.People, err = s.repo.People(query, orgID, userID, limit); err != nil {
			return out, err
		}
	}
	if opts.Messages {
		if out.Messages, err = s.repo.Messages(query, orgID, limit); err != nil {
			return out, err
		}
	}
	if opts.DMs {
		if out.DMs, err = s.repo.DMs(query, orgID, userID, limit); err != nil {
			return out, err
		}
	}
	if out.Docs, err = s.repo.DocsTS(query, p.strict, orgID, limit); err != nil {
		return out, err
	}
	stampOrg(orgID, out.Tasks, out.People, out.Messages, out.Docs)
	return out, nil
}

// looseWith: la segunda vuelta, con alguna de las palabras, sólo en lo que
// le llega a un agente (tareas, notas y docs).
func (s *SearchService) looseWith(query, tsq, orgID, userID string, limit int, notes bool) (domain.SearchResults, error) {
	var out domain.SearchResults
	var err error
	if out.Tasks, err = s.repo.TasksTS(query, tsq, orgID, limit); err != nil {
		return out, err
	}
	if notes {
		if out.Notes, err = s.repo.NotesTS(query, tsq, userID, limit); err != nil {
			return out, err
		}
	}
	if out.Docs, err = s.repo.DocsTS(query, tsq, orgID, limit); err != nil {
		return out, err
	}
	stampOrg(orgID, out.Tasks, out.Docs)
	return out, nil
}

// stampOrg: todo lo que sale filtrado por la org es de esa org. Los directos
// traen la suya de la consulta, y las notas no son de ninguna.
func stampOrg(orgID string, lists ...[]domain.SearchHit) {
	for _, hits := range lists {
		for i := range hits {
			hits[i].OrgID = orgID
		}
	}
}

// SearchOrgs hace la misma búsqueda en varias organizaciones y junta lo que
// sale, sin repetir y sin pasar del límite por tipo. Busca **sólo** en las que
// se le pasan: quién decide cuáles son es quien llama (el handler, con las
// membresías del token).
func (s *SearchService) SearchOrgs(query string, orgIDs []string, userID string, limit int) (domain.SearchResults, error) {
	return s.SearchOrgsWith(query, orgIDs, userID, limit, domain.SearchForPeople())
}

// SearchOrgsWith: SearchOrgs con las fuentes que se piden.
//
// Las orgs se buscan a la vez, como mucho `orgWorkers`, y se juntan en el
// orden en que llegaron: el resultado es el mismo que buscándolas una detrás
// de otra (command-and-control-570; antes iban en serie y era lo que más
// sumaba con muchas orgs).
//
// Las notas son de la persona y no de una org: se buscan una vez, no en cada
// org para quedarse con las de la primera.
//
// La segunda vuelta («con alguna palabra») se decide sobre **todas** las
// orgs juntas, no en cada una: si una org casa con todas las palabras, lo
// parcial de otra no se mezcla con eso.
func (s *SearchService) SearchOrgsWith(query string, orgIDs []string, userID string, limit int, opts domain.SearchOptions) (domain.SearchResults, error) {
	var out domain.SearchResults
	out.Tasks, out.Notes = []domain.SearchHit{}, []domain.SearchHit{}
	out.People, out.Messages, out.DMs = []domain.SearchHit{}, []domain.SearchHit{}, []domain.SearchHit{}
	out.Docs = []domain.SearchHit{}
	if limit <= 0 || limit > 20 {
		limit = 8
	}
	merge := func(dst *[]domain.SearchHit, seen map[string]bool, src []domain.SearchHit) {
		for _, h := range src {
			// Con el enlace: dos pestañas, o dos páginas, de un mismo doc
			// comparten nodo y no son el mismo acierto.
			key := string(h.Kind) + ":" + h.ID + "|" + h.Link
			if len(*dst) < limit && !seen[key] {
				seen[key] = true
				*dst = append(*dst, h)
			}
		}
	}
	seen := map[string]map[string]bool{}
	for _, k := range []string{"tasks", "notes", "people", "messages", "dms", "docs"} {
		seen[k] = map[string]bool{}
	}
	p := s.plan(query)
	if len(orgIDs) == 0 || len(strings.TrimSpace(query)) < 2 {
		return out, nil
	}
	notes, err := s.searchWith(query, p, "", userID, limit, domain.SearchForAgent(), true)
	if err != nil {
		return out, err
	}
	merge(&out.Notes, seen["notes"], notes.Notes)
	results, err := eachOrg(orgIDs, s.orgWorkers, func(org string) (domain.SearchResults, error) {
		return s.searchWith(query, p, org, userID, limit, opts, false)
	})
	if err != nil {
		return out, err
	}
	for _, r := range results {
		merge(&out.Tasks, seen["tasks"], r.Tasks)
		merge(&out.People, seen["people"], r.People)
		merge(&out.Messages, seen["messages"], r.Messages)
		merge(&out.DMs, seen["dms"], r.DMs)
		merge(&out.Docs, seen["docs"], r.Docs)
	}
	if !out.NothingForAnAgent() || p.loose == "" {
		return out, nil
	}
	loose, err := s.looseWith(query, p.loose, "", userID, limit, true)
	if err != nil {
		return out, err
	}
	merge(&out.Notes, seen["notes"], loose.Notes)
	results, err = eachOrg(orgIDs, s.orgWorkers, func(org string) (domain.SearchResults, error) {
		return s.looseWith(query, p.loose, org, userID, limit, false)
	})
	if err != nil {
		return out, err
	}
	for _, r := range results {
		merge(&out.Tasks, seen["tasks"], r.Tasks)
		merge(&out.Docs, seen["docs"], r.Docs)
	}
	out.Partial = !out.NothingForAnAgent()
	return out, nil
}

// eachOrg corre `search` en cada org, como mucho `workers` a la vez, y
// devuelve los resultados en el orden de `orgIDs`. Con un fallo, el de la
// primera org que falló, como cuando iban en serie.
func eachOrg(orgIDs []string, workers int, search func(org string) (domain.SearchResults, error)) ([]domain.SearchResults, error) {
	if workers < 1 {
		workers = 1
	}
	results := make([]domain.SearchResults, len(orgIDs))
	errs := make([]error, len(orgIDs))
	slots := make(chan struct{}, workers)
	var wg sync.WaitGroup
	for i, org := range orgIDs {
		wg.Add(1)
		slots <- struct{}{}
		go func(i int, org string) {
			defer func() { <-slots; wg.Done() }()
			results[i], errs[i] = search(org)
		}(i, org)
	}
	wg.Wait()
	for _, err := range errs {
		if err != nil {
			return nil, err
		}
	}
	return results, nil
}

// RecordMiss: ver domain.SearchMiss. Una consulta demasiado corta para buscar
// no cuenta: no buscó, así que tampoco falló.
func (s *SearchService) RecordMiss(userID, orgID, query string) error {
	if domain.SearchTSQuery(query) == "" || len([]rune(strings.TrimSpace(query))) < 2 {
		return nil
	}
	return s.repo.RecordMiss(userID, orgID, strings.TrimSpace(query))
}

// Misses: ver SearchRepository.Misses. `days` entre 1 y 90 (lo que se guarda).
func (s *SearchService) Misses(orgID string, unscoped bool, days int) ([]domain.SearchMissSummary, error) {
	if days <= 0 || days > 90 {
		days = 30
	}
	return s.repo.Misses(orgID, unscoped, time.Now().AddDate(0, 0, -days), 200)
}
