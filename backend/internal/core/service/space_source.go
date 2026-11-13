package service

import (
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// SpaceSourceService: las fuentes de un espacio (ver domain/space_source.go).
type SpaceSourceService struct {
	repo *repository.SpaceSourceRepository
}

func NewSpaceSourceService(repo *repository.SpaceSourceRepository) *SpaceSourceService {
	return &SpaceSourceService{repo: repo}
}

func (s *SpaceSourceService) List(spaceID string) ([]domain.SpaceSource, error) {
	return s.repo.List(spaceID)
}

// Replace valida la lista entera antes de tocar nada: una fuente mala no deja
// el espacio con media configuración.
func (s *SpaceSourceService) Replace(sp *domain.TaskSpace, req domain.SpaceSourcesRequest) ([]domain.SpaceSource, error) {
	if len(req.Sources) > domain.SpaceSourcesMax {
		return nil, domain.ErrSourceTooMany
	}
	clean := make([]domain.SpaceSourceInput, 0, len(req.Sources))
	seen := map[[3]string]bool{}
	for _, in := range req.Sources {
		x, err := in.Normalize()
		if err != nil {
			return nil, err
		}
		k := [3]string{string(x.Role), string(x.System), x.Ref}
		if seen[k] {
			return nil, domain.ErrSourceRepeated
		}
		seen[k] = true
		// El registro es una página de esta organización: la de otra no se
		// lee desde aquí, y un id inventado no nombraría nada sin decirlo.
		if x.Role == domain.SourceRules && !s.repo.PageInOrg(sp.OrgID, x.Ref) {
			return nil, domain.ErrSourcePage
		}
		clean = append(clean, x)
	}
	return s.repo.Replace(sp.OrgID, sp.ID, clean)
}

// syncSkew: lo que puede adelantar el reloj de quien avisa.
const syncSkew = 5 * time.Minute

// MarkSynced apunta hasta cuándo se leyó una fuente. Una hora que aún no ha
// pasado dejaría sin leer lo que llegue hasta entonces.
func (s *SpaceSourceService) MarkSynced(spaceID, sourceID string, at time.Time) (*domain.SpaceSource, error) {
	if at.IsZero() {
		at = time.Now()
	}
	if at.After(time.Now().Add(syncSkew)) {
		return nil, domain.ErrSourceSyncFuture
	}
	return s.repo.MarkSynced(spaceID, sourceID, at)
}
