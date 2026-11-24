package service

import (
	"errors"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

var (
	// ErrDocShareTTL: la duración no es 1, 7 ni 30 días.
	ErrDocShareTTL = errors.New("doc-share-bad-ttl")
	// ErrDocShareInvalid: el enlace no abre nada. Firma mala, caducado,
	// revocado, página borrada o adjunto que no cita el árbol contestan lo
	// mismo, a propósito: distinguirlos le diría a quien prueba enlaces cuáles
	// existen o existieron.
	ErrDocShareInvalid = errors.New("doc-share-invalid")
)

// DocShareService: compartir una página con quien no tiene cuenta. Ver
// domain/doc_share.go.
type DocShareService struct {
	repo *repository.DocShareRepository
	docs *repository.DocRepository
	now  func() time.Time
}

func NewDocShareService(repo *repository.DocShareRepository, docs *repository.DocRepository) *DocShareService {
	return &DocShareService{repo: repo, docs: docs, now: func() time.Time { return time.Now().UTC() }}
}

// ─── Lo que hace un miembro ──────────────────────────────────────────────────
// Quien llama ya comprobó que puede leer el documento.

// Create abre un enlace a una página viva del documento.
func (s *DocShareService) Create(doc *domain.Doc, pageID, userID string, ttlHours int) (*domain.DocShareResponse, error) {
	if !domain.ValidDocShareTTL(ttlHours) {
		return nil, ErrDocShareTTL
	}
	if _, err := s.docs.FindPage(doc.ID, pageID); err != nil {
		return nil, err
	}
	sh := &domain.DocShare{
		OrgID: doc.OrgID, DocID: doc.ID, PageID: pageID, CreatedBy: userID,
		ExpiresAt: s.now().Add(time.Duration(ttlHours) * time.Hour),
	}
	if err := s.repo.Create(sh); err != nil {
		return nil, err
	}
	out := s.respond(*sh)
	return &out, nil
}

// List: los enlaces vivos de una página.
func (s *DocShareService) List(doc *domain.Doc, pageID string) ([]domain.DocShareResponse, error) {
	if _, err := s.docs.FindPage(doc.ID, pageID); err != nil {
		return nil, err
	}
	rows, err := s.repo.ListLive(doc.ID, pageID, s.now())
	if err != nil {
		return nil, err
	}
	out := make([]domain.DocShareResponse, 0, len(rows))
	for _, sh := range rows {
		out = append(out, s.respond(sh))
	}
	return out, nil
}

func (s *DocShareService) Find(id string) (*domain.DocShare, error) {
	return s.repo.FindByID(id)
}

func (s *DocShareService) Revoke(id string) error {
	return s.repo.Revoke(id, s.now())
}

func (s *DocShareService) respond(sh domain.DocShare) domain.DocShareResponse {
	return domain.DocShareResponse{
		ID: sh.ID, PageID: sh.PageID, ExpiresAt: sh.ExpiresAt,
		CreatedBy: sh.CreatedBy, CreatedByName: s.docs.AuthorName(sh.CreatedBy),
		Link: repository.SignDocShareLink(sh.ID),
	}
}

// ─── La puerta de quien no tiene cuenta ──────────────────────────────────────

// live: el enlace de un token, si la firma es buena y todavía abre algo.
func (s *DocShareService) live(token string) (*domain.DocShare, error) {
	id, ok := repository.VerifyDocShareLink(token)
	if !ok {
		return nil, ErrDocShareInvalid
	}
	sh, err := s.repo.FindByID(id)
	if errors.Is(err, repository.ErrDocShareNotFound) {
		return nil, ErrDocShareInvalid
	}
	if err != nil {
		return nil, err
	}
	if !sh.Live(s.now()) {
		return nil, ErrDocShareInvalid
	}
	return sh, nil
}

// Public: el árbol compartido, tal y como está ahora.
func (s *DocShareService) Public(token string) (*domain.PublicSharedDoc, error) {
	sh, err := s.live(token)
	if err != nil {
		return nil, err
	}
	pages, err := s.repo.SharedTree(sh.DocID, sh.PageID)
	if err != nil {
		return nil, err
	}
	if len(pages) == 0 {
		return nil, ErrDocShareInvalid
	}
	out := &domain.PublicSharedDoc{
		Title: pages[0].Title, ExpiresAt: sh.ExpiresAt,
		Pages: make([]domain.PublicSharedPage, 0, len(pages)),
	}
	for i, p := range pages {
		parent := p.ParentID
		if i == 0 {
			parent = nil
		}
		out.Pages = append(out.Pages, domain.PublicSharedPage{ID: p.ID, ParentID: parent, Title: p.Title, Body: p.Body})
	}
	return out, nil
}

// PublicAttachment: un adjunto, sólo si es del documento compartido **y** lo
// cita alguna página del árbol. Un adjunto del mismo documento que sólo cita
// otra página no se abre con este enlace.
func (s *DocShareService) PublicAttachment(token, attachmentID string) (*domain.DocAttachment, error) {
	sh, err := s.live(token)
	if err != nil {
		return nil, err
	}
	att, err := s.docs.FindAttachment(attachmentID)
	if err != nil || att.DocID != sh.DocID {
		return nil, ErrDocShareInvalid
	}
	cited, err := s.repo.CitedInTree(sh.DocID, sh.PageID, att.ID)
	if err != nil {
		return nil, err
	}
	if !cited {
		return nil, ErrDocShareInvalid
	}
	return att, nil
}
