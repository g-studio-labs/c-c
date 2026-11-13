package service

import (
	"errors"
	"slices"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// ErrGraphNotFound: la cosa no existe o es de una org de la que no se es
// miembro (no se distingue: no se dice qué existe fuera).
var ErrGraphNotFound = errors.New("not found")

// GraphService: lo relacionado con cualquier cosa (command-and-control-348).
type GraphService struct {
	graph *repository.GraphRepository
	tasks *repository.TaskRepository
	// rules resuelve una regla (`R-012`) a su sección. Opcional.
	rules *repository.SpaceSourceRepository
}

// WithRules deja pedir lo relacionado con una regla por su nombre.
func (s *GraphService) WithRules(rules *repository.SpaceSourceRepository) *GraphService {
	s.rules = rules
	return s
}

// KindRule: no es un tipo del grafo, es cómo se pide una sección por la regla
// que es (`/graph/rule/R-012`): quien la cita no sabe el id de su sección.
const KindRule domain.EntityKind = "rule"

// ErrGraphSeveral: varios registros tienen esa regla; se pide por su sección.
var ErrGraphSeveral = errors.New("several")

func NewGraphService(graph *repository.GraphRepository, tasks *repository.TaskRepository) *GraphService {
	return &GraphService{graph: graph, tasks: tasks}
}

// Similar: lo vigente que se parece a una decisión o un incidente (ver
// GraphRepository.Similar). Sólo de una org de quien pregunta.
func (s *GraphService) Similar(kind domain.EntityKind, id string, orgs []string, any bool) ([]domain.SimilarEntry, error) {
	org := s.graph.OrgOf(kind, id)
	if (kind != domain.EntityDecision && kind != domain.EntityIncident) || org == "" || (!any && !slices.Contains(orgs, org)) {
		return nil, ErrGraphNotFound
	}
	return s.graph.Similar(kind, id)
}

// Related: la cosa y sus vecinos. Una tarea se pide también por su ref
// (`proteus-12`), que es como la nombra todo el mundo. `orgs` son las de quien
// pregunta; vacío con `any` (un superadmin) mira en todas.
func (s *GraphService) Related(kind domain.EntityKind, id string, orgs []string, any bool) (*domain.Related, error) {
	if kind == KindRule && s.rules != nil {
		secs := s.rules.FindRule(orgs, any, id)
		if len(secs) > 1 {
			return nil, ErrGraphSeveral
		}
		if len(secs) == 1 {
			kind, id = domain.EntitySection, secs[0]
		}
	}
	if !domain.ValidEntityKind(string(kind)) {
		return nil, ErrGraphNotFound
	}
	if kind == domain.EntityTask {
		if refs := domain.ExtractTaskRefs(id, false); len(refs) == 1 {
			if it, err := s.tasks.FindByRef(orgs, refs[0]); err == nil {
				id = it.ID
			}
		}
	}
	org := s.graph.OrgOf(kind, id)
	if org == "" || (!any && !slices.Contains(orgs, org)) {
		return nil, ErrGraphNotFound
	}
	title, _ := s.graph.Describe(kind, id)
	ns, truncated, err := s.graph.Neighbors(org, kind, id)
	if err != nil {
		return nil, err
	}
	return &domain.Related{Kind: kind, ID: id, Title: title, Neighbors: ns, Truncated: truncated}, nil
}
