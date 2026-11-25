package handler

import (
	"errors"
	"io"
	"net/http"
	"strconv"

	"github.com/go-chi/chi/v5"

	"github.com/guz-studio/cac/backend/internal/adapters/mediastore"
	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

// DocShareHandler: compartir una página con un enlace (command-and-control-496).
//
// Dos mitades, como CallHandler. La de los miembros va detrás del JWT y pide lo
// mismo que leer el documento. La pública no tiene JWT: la abre el token de la
// cabecera `X-Doc-Share` —nunca la URL, que acaba en historiales y registros—,
// lleva su propio límite de peticiones y contesta **el mismo 404** a todo lo
// que no abre.
type DocShareHandler interface {
	Create(http.ResponseWriter, *http.Request)
	List(http.ResponseWriter, *http.Request)
	Revoke(http.ResponseWriter, *http.Request)
	PublicDoc(http.ResponseWriter, *http.Request)
	PublicAttachment(http.ResponseWriter, *http.Request)
}

// DocShareHeader: la cabecera que lleva el token.
const DocShareHeader = "X-Doc-Share"

type docShareHandler struct {
	svc     *service.DocShareService
	docs    *service.DocService
	store   *mediastore.Store
	limiter *ingestLimiter
}

func NewDocShareHandler(svc *service.DocShareService, docs *service.DocService, store *mediastore.Store) DocShareHandler {
	return &docShareHandler{svc: svc, docs: docs, store: store, limiter: newIngestLimiter()}
}

// Más holgado que la puerta de las llamadas: abrir un documento son una
// petición por el árbol y una por cada imagen que cita, y una página con
// treinta capturas no puede quedarse a medias a la segunda recarga.
const (
	publicDocPerIPPerHour    = 600
	publicDocPerSharePerHour = 2400
)

// ─── Miembros ────────────────────────────────────────────────────────────────

// readableDoc: el documento de la URL, si quien pregunta puede leerlo. 404 y
// no 403, como `resolveDoc`: a quien no es de la organización no se le
// confirma que existe.
func (h *docShareHandler) readableDoc(w http.ResponseWriter, r *http.Request, orgOf func() (string, bool)) (*domain.ClaimsJWT, bool) {
	user, ok := currentUser(r)
	if !ok {
		SendErrorResponse(w, http.StatusUnauthorized, "Unauthorized", "no-claims")
		return nil, false
	}
	orgID, found := orgOf()
	if !found {
		SendErrorResponse(w, http.StatusNotFound, "Not found", "not-found")
		return nil, false
	}
	if _, member := user.RoleInOrg(orgID); !member && !user.Superadmin {
		SendErrorResponse(w, http.StatusNotFound, "Not found", "not-found")
		return nil, false
	}
	return user, true
}

// doc: el documento del nodo de la URL, con la misma valla que `GetPage`: el
// nodo existe, quien pregunta es de su organización, y el nodo tiene documento.
func (h *docShareHandler) doc(w http.ResponseWriter, r *http.Request) (*domain.Doc, *domain.ClaimsJWT, bool) {
	kind := chi.URLParam(r, "kind")
	if !domain.ValidDocOwnerKind(kind) {
		SendErrorResponse(w, http.StatusBadRequest, "Unknown document owner", "bad-kind")
		return nil, nil, false
	}
	ownerID := chi.URLParam(r, "ownerId")
	user, ok := h.readableDoc(w, r, func() (string, bool) {
		orgID, err := h.docs.OwnerOrg(domain.DocOwnerKind(kind), ownerID)
		return orgID, err == nil
	})
	if !ok {
		return nil, nil, false
	}
	d, err := h.docs.Get(domain.DocOwnerKind(kind), ownerID)
	if err != nil || d == nil {
		SendErrorResponse(w, http.StatusNotFound, "Not found", "not-found")
		return nil, nil, false
	}
	return d, user, true
}

func shareError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, service.ErrDocShareTTL):
		SendErrorResponse(w, http.StatusBadRequest, "A link lasts 24, 168 or 720 hours", "bad-ttl")
	case errors.Is(err, repository.ErrDocPageNotFound), errors.Is(err, repository.ErrDocShareNotFound):
		SendErrorResponse(w, http.StatusNotFound, "Not found", "not-found")
	default:
		SendErrorResponse(w, http.StatusInternalServerError, "Failed to share the page", err.Error())
	}
}

func (h *docShareHandler) Create(w http.ResponseWriter, r *http.Request) {
	d, user, ok := h.doc(w, r)
	if !ok {
		return
	}
	// Compartir es publicar fuera de la org: no lo hace un lector. Revocar sí
	// (ver Revoke).
	if refuseReader(w, user, d.OrgID) {
		return
	}
	req, err := ValidateRequest[domain.CreateDocShareRequest](r)
	if err != nil {
		SendErrorResponse(w, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}
	out, err := h.svc.Create(d, chi.URLParam(r, "pageId"), user.UserID, req.TTLHours)
	if err != nil {
		shareError(w, err)
		return
	}
	SendResult(w, http.StatusCreated, domain.APIResponse[*domain.DocShareResponse]{Success: true, Data: out})
}

func (h *docShareHandler) List(w http.ResponseWriter, r *http.Request) {
	d, _, ok := h.doc(w, r)
	if !ok {
		return
	}
	out, err := h.svc.List(d, chi.URLParam(r, "pageId"))
	if err != nil {
		shareError(w, err)
		return
	}
	SendResult(w, http.StatusOK, domain.APIResponse[[]domain.DocShareResponse]{Success: true, Data: out})
}

// Revoke: cualquiera que pueda leer el documento, no sólo quien lo compartió.
// Un enlace que se filtró lo tiene que poder cerrar quien se entere primero.
// También un lector, a propósito: cerrar no cambia la documentación, y es la
// excepción que nombra `TestAReaderCannotChangeDocumentation`.
func (h *docShareHandler) Revoke(w http.ResponseWriter, r *http.Request) {
	var sh *domain.DocShare
	_, ok := h.readableDoc(w, r, func() (string, bool) {
		var err error
		sh, err = h.svc.Find(chi.URLParam(r, "id"))
		if err != nil {
			return "", false
		}
		return sh.OrgID, true
	})
	if !ok {
		return
	}
	if err := h.svc.Revoke(sh.ID); err != nil {
		shareError(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// ─── Público ─────────────────────────────────────────────────────────────────

// allow: el límite de la puerta, por IP y, con una firma buena, por enlace.
func (h *docShareHandler) allow(w http.ResponseWriter, r *http.Request, token string) bool {
	ok := h.limiter.allow("ip:"+clientIP(r), publicDocPerIPPerHour)
	if ok {
		if id, valid := repository.VerifyDocShareLink(token); valid {
			ok = h.limiter.allow("share:"+id, publicDocPerSharePerHour)
		}
	}
	if !ok {
		SendErrorResponse(w, http.StatusTooManyRequests, "Too many requests", "rate-limited")
	}
	return ok
}

func publicShareError(w http.ResponseWriter, err error) {
	if errors.Is(err, service.ErrDocShareInvalid) {
		SendErrorResponse(w, http.StatusNotFound, "This link does not open anything", "share-invalid")
		return
	}
	SendErrorResponse(w, http.StatusInternalServerError, "Failed to open the link", "share-failed")
}

func (h *docShareHandler) PublicDoc(w http.ResponseWriter, r *http.Request) {
	token := r.Header.Get(DocShareHeader)
	if !h.allow(w, r, token) {
		return
	}
	out, err := h.svc.Public(token)
	if err != nil {
		publicShareError(w, err)
		return
	}
	w.Header().Set("Cache-Control", "private, no-store")
	SendResult(w, http.StatusOK, domain.APIResponse[*domain.PublicSharedDoc]{Success: true, Data: out})
}

func (h *docShareHandler) PublicAttachment(w http.ResponseWriter, r *http.Request) {
	token := r.Header.Get(DocShareHeader)
	if !h.allow(w, r, token) {
		return
	}
	att, err := h.svc.PublicAttachment(token, chi.URLParam(r, "attachmentId"))
	if err != nil {
		publicShareError(w, err)
		return
	}
	if h.store == nil || !h.store.Enabled() {
		SendErrorResponse(w, http.StatusServiceUnavailable, "Attachment storage unavailable", "store-disabled")
		return
	}
	obj, err := h.store.Get(r.Context(), att.Path)
	if err != nil {
		SendErrorResponse(w, http.StatusBadGateway, "Failed to fetch attachment", "store-failed")
		return
	}
	defer obj.Body.Close()
	ct := obj.ContentType
	if ct == "" {
		ct = att.ContentType
	}
	w.Header().Set("Cache-Control", "private, no-store")
	setFileHeaders(w, ct, att.FileName)
	if obj.Size > 0 {
		w.Header().Set("Content-Length", strconv.FormatInt(obj.Size, 10))
	}
	w.WriteHeader(http.StatusOK)
	io.Copy(w, obj.Body)
}
