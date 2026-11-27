package service

import (
	"bytes"
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/events"
	lg "github.com/guz-studio/cac/backend/internal/core/logger"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

// La integración con Basecamp (ver domain/basecamp.go).

// BasecampConfig: la app de 37signals de este servidor. Sin client id y
// secret, conectar contesta «no configurado» y el resto del servidor arranca
// igual (la postura de GitHub).
type BasecampConfig struct {
	ClientID     string
	ClientSecret string
	// RedirectURI: la que se registró en launchpad.37signals.com.
	RedirectURI string
	// LaunchpadURL y APIURL: las de 37signals; otras sólo en las pruebas.
	LaunchpadURL string
	APIURL       string
	// WebhookBase: dónde recibe este servidor los webhooks. Vacío: el origen
	// de RedirectURI, que es este mismo servidor.
	WebhookBase string
}

func (c BasecampConfig) Configured() bool { return c.ClientID != "" && c.ClientSecret != "" }

func (c BasecampConfig) withDefaults() BasecampConfig {
	if c.RedirectURI == "" {
		c.RedirectURI = "https://cac.guz-studio.dev/webhooks/basecamp/setup"
	}
	if c.LaunchpadURL == "" {
		c.LaunchpadURL = "https://launchpad.37signals.com"
	}
	if c.APIURL == "" {
		c.APIURL = "https://3.basecampapi.com"
	}
	if c.WebhookBase == "" {
		if u, err := url.Parse(c.RedirectURI); err == nil {
			c.WebhookBase = u.Scheme + "://" + u.Host
		}
	}
	c.LaunchpadURL = strings.TrimRight(c.LaunchpadURL, "/")
	c.APIURL = strings.TrimRight(c.APIURL, "/")
	c.WebhookBase = strings.TrimRight(c.WebhookBase, "/")
	return c
}

const (
	// basecampStateTTL: lo que dura el enlace de autorizar desde que se pide.
	basecampStateTTL = 10 * time.Minute
	// basecampRefreshMargin: un token que caduca antes de esto se renueva al
	// usarlo. El de Basecamp dura dos semanas.
	basecampRefreshMargin = 24 * time.Hour
	// basecampRefreshAhead: el reloj de fondo renueva lo que caduca antes de
	// esto, para que no dependa de que alguien lo use.
	basecampRefreshAhead = 72 * time.Hour
	// basecampExcerpt: lo que se copia de un texto de Basecamp.
	basecampExcerpt = 280
	// basecampUserAgent: Basecamp pide uno que identifique la app.
	basecampUserAgent = "cac (https://cac.guz-studio.dev)"
	// maxBasecampPages: las páginas de proyectos que se leen. Cien proyectos
	// por página de sobra para una cuenta de agencia.
	maxBasecampPages = 10
)

var ErrBasecampUnknownHook = errors.New("basecamp-unknown-hook")

type BasecampService struct {
	repo     *repository.BasecampRepository
	cfg      BasecampConfig
	hub      *events.Hub
	inbox    Notifier
	requests *RequestService
	http     *http.Client
	now      func() time.Time
	// refreshing: una renovación por org a la vez. Basecamp rota el token y
	// dos renovaciones cruzadas guardarían la que perdió.
	mu         sync.Mutex
	refreshing map[string]*sync.Mutex
	// journal: el diario del evento completo; nil, el del proceso
	// (journalOut). enrichTimeout: el plazo para completarlo.
	journal       *EventJournal
	enrichTimeout time.Duration
}

func NewBasecampService(repo *repository.BasecampRepository, cfg BasecampConfig, hub *events.Hub,
	inbox Notifier, requests *RequestService) *BasecampService {
	return &BasecampService{repo: repo, cfg: cfg.withDefaults(), hub: hub, inbox: inbox, requests: requests,
		http: &http.Client{Timeout: 15 * time.Second}, now: func() time.Time { return time.Now().UTC() },
		refreshing: map[string]*sync.Mutex{}}
}

// WithClock: otro reloj, para probar la caducidad del state y del token.
func (s *BasecampService) WithClock(now func() time.Time) *BasecampService {
	s.now = now
	return s
}

func (s *BasecampService) Configured() bool { return s.cfg.Configured() }

// ─── Conectar (OAuth de Launchpad) ───────────────────────────────────────────
//
// La app manda al navegador a Launchpad con un `state` firmado que lleva la
// org, quién conecta y un nonce de un solo uso; Launchpad lo devuelve a
// `/webhooks/basecamp/setup` con el `code`. Sin la firma, cualquiera podría
// atar su cuenta a la org que quisiera; sin el nonce, quien viera la URL de un
// admin podría volver con su propio `code` mientras no caduque.

func (s *BasecampService) ConnectURL(orgID, userID string) (string, error) {
	if !s.Configured() {
		return "", domain.ErrBasecampOff
	}
	raw := make([]byte, 24)
	if _, err := rand.Read(raw); err != nil {
		return "", err
	}
	nonce := base64.RawURLEncoding.EncodeToString(raw)
	exp := s.now().Add(basecampStateTTL)
	if err := s.repo.SaveState(&domain.BasecampOAuthState{Nonce: nonce, OrgID: orgID, ExpiresAt: exp}, s.now()); err != nil {
		return "", err
	}
	q := url.Values{"type": {"web_server"}, "client_id": {s.cfg.ClientID}, "redirect_uri": {s.cfg.RedirectURI},
		"state": {s.signState(orgID, userID, nonce, exp)}}
	return s.cfg.LaunchpadURL + "/authorization/new?" + q.Encode(), nil
}

func (s *BasecampService) stateMAC(orgID, userID, nonce string, exp int64) string {
	mac := hmac.New(sha256.New, []byte(s.cfg.ClientSecret))
	fmt.Fprintf(mac, "basecamp-link:%s:%s:%s:%d", orgID, userID, nonce, exp)
	return hex.EncodeToString(mac.Sum(nil))
}

func (s *BasecampService) signState(orgID, userID, nonce string, exp time.Time) string {
	e := exp.Unix()
	enc := base64.RawURLEncoding.EncodeToString
	return enc([]byte(orgID)) + "." + enc([]byte(userID)) + "." + nonce + "." + strconv.FormatInt(e, 10) + "." +
		s.stateMAC(orgID, userID, nonce, e)
}

// verifyState comprueba la firma y la caducidad y gasta el nonce: un state
// sirve una sola vez.
func (s *BasecampService) verifyState(state string) (orgID, userID string, err error) {
	parts := strings.Split(state, ".")
	if len(parts) != 5 {
		return "", "", domain.ErrBasecampBadState
	}
	org, err1 := base64.RawURLEncoding.DecodeString(parts[0])
	user, err2 := base64.RawURLEncoding.DecodeString(parts[1])
	nonce := parts[2]
	exp, err3 := strconv.ParseInt(parts[3], 10, 64)
	if err1 != nil || err2 != nil || err3 != nil || len(org) == 0 || nonce == "" {
		return "", "", domain.ErrBasecampBadState
	}
	if !hmac.Equal([]byte(parts[4]), []byte(s.stateMAC(string(org), string(user), nonce, exp))) || s.now().Unix() > exp {
		return "", "", domain.ErrBasecampBadState
	}
	fresh, err := s.repo.ConsumeState(nonce, string(org), s.now())
	if err != nil {
		return "", "", err
	}
	if !fresh {
		return "", "", domain.ErrBasecampBadState
	}
	return string(org), string(user), nil
}

type launchpadToken struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresIn    int64  `json:"expires_in"`
}

type launchpadAuthorization struct {
	Identity struct {
		FirstName string `json:"first_name"`
		LastName  string `json:"last_name"`
	} `json:"identity"`
	Accounts []struct {
		Product string `json:"product"`
		ID      int64  `json:"id"`
		Name    string `json:"name"`
	} `json:"accounts"`
}

// Setup: la vuelta de Launchpad. Cambia el `code` por los tokens, lee qué
// cuenta de Basecamp autorizó y guarda la conexión cifrada.
func (s *BasecampService) Setup(code, state string) error {
	if !s.Configured() {
		return domain.ErrBasecampOff
	}
	// El code antes que el state: verificarlo lo gasta, y un callback sin code
	// dejaría inservible el enlace bueno que llegue después.
	if strings.TrimSpace(code) == "" {
		return domain.ErrBasecampBadState
	}
	orgID, userID, err := s.verifyState(state)
	if err != nil {
		return err
	}
	tok, err := s.launchpadToken(url.Values{"type": {"web_server"}, "code": {code}})
	if err != nil {
		return err
	}
	if tok.RefreshToken == "" {
		return domain.ErrBasecampUnreachable
	}
	var auth launchpadAuthorization
	if err := s.getJSON(s.cfg.LaunchpadURL+"/authorization.json", tok.AccessToken, &auth); err != nil {
		return err
	}
	conn := &domain.BasecampConnection{OrgID: orgID, ConnectedByID: userID,
		IdentityName: strings.TrimSpace(auth.Identity.FirstName + " " + auth.Identity.LastName),
		ExpiresAt:    s.expiry(tok.ExpiresIn)}
	for _, a := range auth.Accounts {
		if a.Product == "bc3" {
			conn.AccountID, conn.AccountName = a.ID, a.Name
			break
		}
	}
	if conn.AccountID == 0 {
		return domain.ErrBasecampNoAccount
	}
	if conn.AccessToken, err = repository.EncryptTelemetry([]byte(tok.AccessToken)); err != nil {
		return err
	}
	if conn.RefreshToken, err = repository.EncryptTelemetry([]byte(tok.RefreshToken)); err != nil {
		return err
	}
	if err := s.dropOtherAccount(orgID, conn.AccountID); err != nil {
		return err
	}
	return s.repo.SaveConnection(conn)
}

// dropOtherAccount: conectar otra cuenta deja huérfanos los proyectos de la
// vieja. Sus webhooks se quitan con el token de la vieja, mientras aún es el
// guardado (contra la nueva, esos ids no existen o son de otro), y sus
// proyectos se borran: su secreto deja de valer aunque Basecamp no conteste.
func (s *BasecampService) dropOtherAccount(orgID string, accountID int64) error {
	old, err := s.repo.Connection(orgID)
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil
	}
	if err != nil {
		return err
	}
	if old.AccountID == accountID {
		return nil
	}
	projects, err := s.repo.Projects(orgID)
	if err != nil {
		return err
	}
	for _, p := range projects {
		s.dropWebhook(orgID, &p)
	}
	return s.repo.DeleteProjects(orgID)
}

func (s *BasecampService) expiry(seconds int64) time.Time {
	if seconds <= 0 {
		seconds = int64((14 * 24 * time.Hour).Seconds())
	}
	return s.now().Add(time.Duration(seconds) * time.Second)
}

// launchpadToken: el client id y el secret van en el cuerpo, no en la URL,
// para que no acaben en el log de nadie.
func (s *BasecampService) launchpadToken(form url.Values) (*launchpadToken, error) {
	form.Set("client_id", s.cfg.ClientID)
	form.Set("client_secret", s.cfg.ClientSecret)
	form.Set("redirect_uri", s.cfg.RedirectURI)
	req, err := http.NewRequest(http.MethodPost, s.cfg.LaunchpadURL+"/authorization/token", strings.NewReader(form.Encode()))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("User-Agent", basecampUserAgent)
	res, err := s.http.Do(req)
	if err != nil {
		return nil, domain.ErrBasecampUnreachable
	}
	defer res.Body.Close()
	if res.StatusCode == http.StatusBadRequest || res.StatusCode == http.StatusUnauthorized {
		// `invalid_grant` y compañía: Launchpad sí contestó.
		return nil, fmt.Errorf("%w: launchpad token %d", domain.ErrBasecampReconnect, res.StatusCode)
	}
	if res.StatusCode/100 != 2 {
		return nil, fmt.Errorf("%w: launchpad token %d", domain.ErrBasecampUnreachable, res.StatusCode)
	}
	var tok launchpadToken
	if err := json.NewDecoder(io.LimitReader(res.Body, 1<<16)).Decode(&tok); err != nil || tok.AccessToken == "" {
		return nil, domain.ErrBasecampUnreachable
	}
	return &tok, nil
}

// ─── El token: renovarlo solo, y avisar si no se puede ──────────────────────

// accessToken: el token de la org, renovado si está por caducar.
func (s *BasecampService) accessToken(orgID string) (*domain.BasecampConnection, string, error) {
	conn, err := s.repo.Connection(orgID)
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, "", domain.ErrBasecampNotConnected
	}
	if err != nil {
		return nil, "", err
	}
	if conn.ExpiresAt.Sub(s.now()) < basecampRefreshMargin {
		fresh, err := s.refresh(orgID, false)
		switch {
		case err == nil:
			conn = fresh
		case conn.ExpiresAt.After(s.now()):
			// Aún vale: se usa mientras dure. El fallo ya quedó apuntado.
		default:
			return nil, "", err
		}
	}
	plain, err := repository.DecryptTelemetry(conn.AccessToken)
	if err != nil {
		return nil, "", err
	}
	return conn, string(plain), nil
}

func (s *BasecampService) orgLock(orgID string) *sync.Mutex {
	s.mu.Lock()
	defer s.mu.Unlock()
	m, ok := s.refreshing[orgID]
	if !ok {
		m = &sync.Mutex{}
		s.refreshing[orgID] = m
	}
	return m
}

// refresh renueva el token de la org. Si falla, lo apunta y avisa a los
// admins la primera vez: una conexión que caduca en silencio deja de traer
// nada y nadie se entera.
func (s *BasecampService) refresh(orgID string, force bool) (*domain.BasecampConnection, error) {
	lock := s.orgLock(orgID)
	lock.Lock()
	defer lock.Unlock()
	conn, err := s.repo.Connection(orgID)
	if err != nil {
		return nil, err
	}
	// Otra llamada ya la renovó mientras ésta esperaba.
	if !force && conn.ExpiresAt.Sub(s.now()) >= basecampRefreshMargin {
		return conn, nil
	}
	refresh, err := repository.DecryptTelemetry(conn.RefreshToken)
	var tok *launchpadToken
	if err == nil {
		tok, err = s.launchpadToken(url.Values{"type": {"refresh"}, "refresh_token": {string(refresh)}})
	}
	if err != nil {
		s.refreshFailed(conn)
		return nil, err
	}
	access, err := repository.EncryptTelemetry([]byte(tok.AccessToken))
	if err != nil {
		return nil, err
	}
	var newRefresh []byte
	if tok.RefreshToken != "" {
		if newRefresh, err = repository.EncryptTelemetry([]byte(tok.RefreshToken)); err != nil {
			return nil, err
		}
	}
	if err := s.repo.SaveTokens(orgID, access, newRefresh, s.expiry(tok.ExpiresIn)); err != nil {
		return nil, err
	}
	return s.repo.Connection(orgID)
}

func (s *BasecampService) refreshFailed(conn *domain.BasecampConnection) {
	lg.Warn("basecamp: renewing the token of org " + conn.OrgID + " failed")
	first, err := s.repo.MarkRefreshFailed(conn.OrgID, s.now())
	if err != nil || !first || s.inbox == nil {
		return
	}
	admins, err := s.repo.AdminIDs(conn.OrgID)
	if err != nil {
		return
	}
	for _, uid := range admins {
		s.inbox.Notify(domain.Aviso{UserID: uid, OrgID: conn.OrgID, Kind: domain.BasecampAuthKind,
			TitleKey: "notify.basecamp.auth_failed", Body: conn.AccountName, Link: "/organization",
			Group: domain.BasecampAccountGroup(conn.AccountID), Label: conn.AccountName})
	}
}

// RefreshExpiring renueva lo que está por caducar. Lo llama el reloj de fondo.
func (s *BasecampService) RefreshExpiring() {
	if !s.Configured() {
		return
	}
	conns, err := s.repo.ExpiringConnections(s.now().Add(basecampRefreshAhead))
	if err != nil {
		lg.Error("basecamp: listing expiring connections: " + err.Error())
		return
	}
	for _, c := range conns {
		_, _ = s.refresh(c.OrgID, true)
	}
}

// Start: el reloj de fondo, cada hora.
func (s *BasecampService) Start(ctx context.Context) {
	if !s.Configured() {
		return
	}
	go func() {
		t := time.NewTicker(time.Hour)
		defer t.Stop()
		for {
			s.RefreshExpiring()
			s.ReconcileWebhooks()
			select {
			case <-ctx.Done():
				return
			case <-t.C:
			}
		}
	}()
}

// ─── La API de Basecamp ──────────────────────────────────────────────────────

func (s *BasecampService) getJSON(u, token string, out any) error {
	_, err := s.do(http.MethodGet, u, token, nil, out)
	return err
}

func (s *BasecampService) do(method, u, token string, body any, out any) (http.Header, error) {
	return s.doCtx(context.Background(), method, u, token, body, out)
}

func (s *BasecampService) doCtx(ctx context.Context, method, u, token string, body any, out any) (http.Header, error) {
	var rd io.Reader
	if body != nil {
		raw, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		rd = bytes.NewReader(raw)
	}
	req, err := http.NewRequestWithContext(ctx, method, u, rd)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("User-Agent", basecampUserAgent)
	req.Header.Set("Accept", "application/json")
	if body != nil {
		req.Header.Set("Content-Type", "application/json; charset=utf-8")
	}
	res, err := s.http.Do(req)
	if err != nil {
		return nil, domain.ErrBasecampUnreachable
	}
	defer res.Body.Close()
	if res.StatusCode == http.StatusNotFound {
		return res.Header, gorm.ErrRecordNotFound
	}
	if res.StatusCode/100 != 2 {
		return res.Header, basecampStatusError{method: method, code: res.StatusCode}
	}
	if out != nil && res.StatusCode != http.StatusNoContent {
		if err := json.NewDecoder(io.LimitReader(res.Body, 4<<20)).Decode(out); err != nil {
			return res.Header, domain.ErrBasecampUnreachable
		}
	}
	return res.Header, nil
}

// basecampStatusError: Basecamp contestó con un estado que no es 2xx ni 404.
// Para quien llama sigue siendo ErrBasecampUnreachable; basecampRefused
// distingue el «no» que ningún reintento cambia.
type basecampStatusError struct {
	method string
	code   int
}

func (e basecampStatusError) Error() string {
	return fmt.Sprintf("%s: %s %d", domain.ErrBasecampUnreachable, e.method, e.code)
}

func (e basecampStatusError) Unwrap() error { return domain.ErrBasecampUnreachable }

// basecampRefused: Basecamp contestó que no (4xx), y no es «espera» (408,
// 429): reintentar el mismo evento daría lo mismo.
func basecampRefused(err error) bool {
	var se basecampStatusError
	if !errors.As(err, &se) {
		return false
	}
	return se.code/100 == 4 && se.code != http.StatusRequestTimeout && se.code != http.StatusTooManyRequests
}

// api: una llamada a la cuenta de la org, con el token renovado si hace falta.
func (s *BasecampService) api(orgID, method, path string, body, out any) (http.Header, error) {
	conn, token, err := s.accessToken(orgID)
	if err != nil {
		return nil, err
	}
	return s.do(method, s.accountURL(conn, path), token, body, out)
}

func (s *BasecampService) accountURL(conn *domain.BasecampConnection, path string) string {
	return s.cfg.APIURL + "/" + strconv.FormatInt(conn.AccountID, 10) + path
}

var reNextLink = regexp.MustCompile(`<([^>]+)>;\s*rel="next"`)

type basecampProjectWire struct {
	ID     int64  `json:"id"`
	Name   string `json:"name"`
	AppURL string `json:"app_url"`
}

// ─── Estado, proyectos y personas (Organización → Integraciones) ────────────

func (s *BasecampService) Status(orgID string) (*domain.BasecampStatus, error) {
	out := &domain.BasecampStatus{Configured: s.Configured(), Projects: []domain.BasecampProject{}}
	conn, err := s.repo.Connection(orgID)
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return out, nil
	}
	if err != nil {
		return nil, err
	}
	exp := conn.ExpiresAt
	out.Connected, out.AccountID, out.AccountName, out.IdentityName = true, conn.AccountID, conn.AccountName, conn.IdentityName
	out.ExpiresAt, out.RefreshFailedAt = &exp, conn.RefreshFailedAt
	if out.Projects, err = s.repo.Projects(orgID); err != nil {
		return nil, err
	}
	out.Unmatched = s.repo.CountUnmatched(orgID)
	out.Awaiting = s.repo.CountAwaiting(orgID)
	return out, nil
}

// Disconnect: quita los webhooks que pueda (Basecamp puede no contestar: no
// por eso se queda la conexión) y borra la conexión y sus proyectos.
func (s *BasecampService) Disconnect(orgID string) error {
	projects, err := s.repo.Projects(orgID)
	if err != nil {
		return err
	}
	for _, p := range projects {
		s.dropWebhook(orgID, &p)
	}
	return s.repo.DeleteConnection(orgID)
}

func (s *BasecampService) dropWebhook(orgID string, p *domain.BasecampProject) {
	if p.WebhookID == 0 {
		return
	}
	path := fmt.Sprintf("/buckets/%d/webhooks/%d.json", p.ProjectID, p.WebhookID)
	if _, err := s.api(orgID, http.MethodDelete, path, nil, nil); err != nil && !errors.Is(err, gorm.ErrRecordNotFound) {
		lg.Warn(fmt.Sprintf("basecamp: removing webhook %d of project %d: %v", p.WebhookID, p.ProjectID, err))
	}
}

// AvailableProjects: los proyectos de la cuenta, con el espacio de los que ya
// están enlazados.
func (s *BasecampService) AvailableProjects(orgID string) ([]domain.BasecampAvailableProject, error) {
	conn, token, err := s.accessToken(orgID)
	if err != nil {
		return nil, err
	}
	linked, err := s.repo.Projects(orgID)
	if err != nil {
		return nil, err
	}
	spaceOf := map[int64]string{}
	for _, p := range linked {
		spaceOf[p.ProjectID] = p.SpaceID
	}
	out := []domain.BasecampAvailableProject{}
	next := s.accountURL(conn, "/projects.json")
	for page := 0; next != "" && page < maxBasecampPages; page++ {
		var batch []basecampProjectWire
		h, err := s.do(http.MethodGet, next, token, nil, &batch)
		if err != nil {
			return nil, err
		}
		for _, p := range batch {
			out = append(out, domain.BasecampAvailableProject{ID: p.ID, Name: p.Name,
				AppURL: domain.SafeBasecampURL(p.AppURL), SpaceID: spaceOf[p.ID]})
		}
		next = ""
		if m := reNextLink.FindStringSubmatch(h.Get("Link")); m != nil && strings.HasPrefix(m[1], s.cfg.APIURL+"/") {
			next = m[1]
		}
	}
	return out, nil
}

// LinkProject enlaza un proyecto a un espacio: le da de alta un webhook con un
// secreto propio, apunta Basecamp como fuente del espacio y trae a su gente.
// Volver a enlazarlo (a otro espacio) cambia el webhook y el secreto.
func (s *BasecampService) LinkProject(orgID, userID string, projectID int64, spaceID string) (*domain.BasecampProject, error) {
	if !s.Configured() {
		return nil, domain.ErrBasecampOff
	}
	if projectID <= 0 {
		return nil, gorm.ErrRecordNotFound
	}
	if !s.repo.SpaceInOrg(spaceID, orgID) {
		return nil, ErrSpaceOutsideOrg
	}
	var wire basecampProjectWire
	if _, err := s.api(orgID, http.MethodGet, fmt.Sprintf("/projects/%d.json", projectID), nil, &wire); err != nil {
		return nil, err
	}
	secret, err := newBasecampSecret()
	if err != nil {
		return nil, err
	}
	var hook struct {
		ID int64 `json:"id"`
	}
	if _, err := s.api(orgID, http.MethodPost, fmt.Sprintf("/buckets/%d/webhooks.json", projectID), map[string]any{
		"payload_url": s.cfg.WebhookBase + "/webhooks/basecamp/" + secret,
		"types":       domain.BasecampWebhookTypes,
	}, &hook); err != nil {
		return nil, err
	}
	old, _ := s.repo.Project(orgID, projectID)
	p := &domain.BasecampProject{OrgID: orgID, SpaceID: spaceID, ProjectID: projectID, Name: wire.Name,
		AppURL: domain.SafeBasecampURL(wire.AppURL), WebhookID: hook.ID,
		SecretHash: domain.BasecampSecretHash(secret), LinkedByID: userID,
		WebhookTypes: domain.BasecampTypesKey(domain.BasecampWebhookTypes)}
	if err := s.repo.SaveProject(p); err != nil {
		// El webhook nuevo no tiene fila que lo reciba: fuera, para que no
		// quede uno huérfano mandando a una dirección muerta. El viejo sigue.
		s.dropWebhook(orgID, &domain.BasecampProject{ProjectID: projectID, WebhookID: hook.ID})
		return nil, err
	}
	// El viejo, sólo cuando el nuevo ya está guardado.
	if old != nil {
		s.dropWebhook(orgID, old)
	}
	s.syncPeople(orgID, projectID)
	return p, nil
}

func newBasecampSecret() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}

// syncPeople trae a la gente del proyecto y casa por correo a quien pueda. Si
// Basecamp no contesta, la gente llega igual con su primer evento.
func (s *BasecampService) syncPeople(orgID string, projectID int64) {
	var people []domain.BasecampPersonWire
	if _, err := s.api(orgID, http.MethodGet, fmt.Sprintf("/projects/%d/people.json", projectID), nil, &people); err != nil {
		lg.Warn(fmt.Sprintf("basecamp: reading the people of project %d: %v", projectID, err))
		return
	}
	for _, p := range people {
		if _, err := s.repo.UpsertPerson(orgID, p); err != nil {
			lg.Warn("basecamp: saving a person: " + err.Error())
		}
	}
}

func (s *BasecampService) UnlinkProject(orgID string, projectID int64) error {
	p, err := s.repo.Project(orgID, projectID)
	if err != nil {
		return err
	}
	s.dropWebhook(orgID, p)
	return s.repo.DeleteProject(p)
}

func (s *BasecampService) People(orgID string) ([]domain.BasecampPerson, error) {
	return s.repo.People(orgID)
}

// MatchPerson: un admin casa a una persona con un miembro de la org.
func (s *BasecampService) MatchPerson(orgID string, personID int64, userID string) (*domain.BasecampPerson, error) {
	userID = strings.TrimSpace(userID)
	if userID != "" && !s.repo.IsMember(orgID, userID) {
		return nil, domain.ErrBasecampUserOutside
	}
	return s.repo.MatchPerson(orgID, personID, userID)
}

// ─── Enlazar una tarea a un recording ────────────────────────────────────────

// LinkTask enlaza la tarea al recording de la URL pegada. La URL tiene que ser
// de la cuenta conectada.
func (s *BasecampService) LinkTask(item *domain.Item, rawURL string) (*domain.BasecampLink, error) {
	account, project, recording, err := domain.ParseBasecampRecordingURL(rawURL)
	if err != nil {
		return nil, err
	}
	conn, err := s.repo.Connection(item.OrgID)
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, domain.ErrBasecampNotConnected
	}
	if err != nil {
		return nil, err
	}
	if conn.AccountID != account {
		return nil, domain.ErrBasecampBadURL
	}
	l := &domain.BasecampLink{OrgID: item.OrgID, ItemID: item.ID, RecordingID: recording, ProjectID: project,
		AppURL: domain.SafeBasecampURL(rawURL)}
	if err := s.repo.CreateLink(l); err != nil {
		return nil, err
	}
	return l, nil
}

func (s *BasecampService) TaskLinks(itemID string) ([]domain.BasecampLink, error) {
	return s.repo.LinksOfTask(itemID)
}

func (s *BasecampService) UnlinkTask(itemID string, recordingID int64) error {
	return s.repo.DeleteLink(itemID, recordingID)
}

// ─── El webhook ──────────────────────────────────────────────────────────────

// HandleWebhook atiende una entrega. El secreto de la dirección es lo único
// que dice que viene de Basecamp: uno que no existe es ErrBasecampUnknownHook,
// y el handler contesta 404 sin decir más.
//
// Un evento se apunta antes de atenderlo: un reintento de Basecamp (o una
// entrega repetida) no avisa dos veces. Si atenderlo falla, se suelta, para
// que el reintento sí entre.
func (s *BasecampService) HandleWebhook(secret string, body []byte) error {
	proj, err := s.repo.ProjectBySecret(secret)
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return ErrBasecampUnknownHook
	}
	if err != nil {
		return err
	}
	var ev domain.BasecampWebhookEvent
	if err := json.Unmarshal(body, &ev); err != nil || ev.ID == 0 {
		// Nada que reintentar: un cuerpo así no mejora.
		return nil
	}
	switch {
	case ev.Kind == domain.BasecampCommentCreated, ev.Kind == domain.BasecampMessageCreated,
		ev.Kind == domain.BasecampTodoCompleted, ev.Kind == domain.BasecampTodoUncompleted,
		domain.BasecampMakesTask(ev.Kind):
	default:
		return nil
	}
	if ev.Recording.Bucket.ID != proj.ProjectID {
		// El secreto de un proyecto no habla por otro; y un cuerpo que no dice
		// de qué proyecto es no se cree.
		return nil
	}
	first, err := s.repo.ClaimEvent(proj.OrgID, ev.ID)
	if err != nil {
		return err
	}
	if !first {
		return nil
	}
	if err := s.process(proj, ev); err != nil {
		_ = s.repo.ReleaseEvent(proj.OrgID, ev.ID)
		return err
	}
	return nil
}

// basecampNote: lo que se apunta y se avisa de un evento.
type basecampNote struct {
	who      string
	titleKey string
	excerpt  string
	url      string
	// key: la del evento del diario, que el aviso lleva como SourceKey.
	key string
}

func (s *BasecampService) process(proj *domain.BasecampProject, ev domain.BasecampWebhookEvent) error {
	creator := ev.Creator
	if creator.ID == 0 && ev.Recording.Creator != nil {
		creator = *ev.Recording.Creator
	}
	actorID := ""
	var person *domain.BasecampPerson
	if creator.ID != 0 {
		p, err := s.repo.UpsertPerson(proj.OrgID, creator)
		if err != nil {
			return err
		}
		person, actorID = p, p.UserID
	}
	// El hilo, con tarea y sin ella, y antes que nada que pueda salir antes.
	author := s.authorOf(proj.OrgID, creator, person)
	full := basecampFull{proj: proj, ev: ev, key: basecampEventKey(ev), author: author,
		thread: s.trackThread(proj, ev, author)}
	targetID, targetTitle, targetURL := ev.Target()
	note := basecampNote{who: strings.TrimSpace(creator.Name), url: domain.SafeBasecampURL(ev.Recording.AppURL), key: full.key}
	if note.url == "" {
		note.url = domain.SafeBasecampURL(targetURL)
	}
	if note.who == "" {
		note.who = "Basecamp"
	}
	note.excerpt = domain.BasecampPlainText(ev.Recording.Content, basecampExcerpt)
	switch ev.Kind {
	case domain.BasecampCommentCreated:
		note.titleKey = "notify.basecamp.comment"
	case domain.BasecampMessageCreated:
		note.titleKey = "notify.basecamp.message"
	case domain.BasecampTodoCompleted, domain.BasecampTodoUncompleted:
		note.titleKey = "notify.basecamp.todo_done"
		if ev.Kind == domain.BasecampTodoUncompleted {
			note.titleKey = "notify.basecamp.todo_reopened"
		}
		if note.excerpt == "" {
			note.excerpt = strings.TrimSpace(ev.Recording.Title)
		}
	}
	if domain.BasecampMakesTask(ev.Kind) {
		return s.processAssignable(full, note.who, actorID)
	}
	if note.excerpt == "" {
		note.excerpt = strings.TrimSpace(targetTitle)
	}

	// A quién nombra: sólo personas casadas, y nunca quien escribe.
	var mentioned []string
	seen := map[string]bool{}
	for _, m := range domain.ExtractBasecampMentions(ev.Recording.Content) {
		p, err := s.repo.PersonForMention(proj.OrgID, m)
		if err != nil || p.UserID == "" || p.UserID == actorID || seen[p.UserID] {
			continue
		}
		seen[p.UserID] = true
		mentioned = append(mentioned, p.UserID)
	}

	// Un enlace a un recording de otro proyecto no es de este secreto.
	var item *domain.Item
	link, err := s.repo.LinkFor(proj.OrgID, targetID)
	switch {
	case err == nil && link.ProjectID == proj.ProjectID:
		if it, err := s.repo.Item(link.ItemID); err == nil {
			item = it
		}
	case err != nil && !errors.Is(err, gorm.ErrRecordNotFound):
		return err
	}
	full.item = item
	// El evento completo va antes que cualquier aviso.
	s.publishEvent(full)
	if item == nil {
		s.notifyUnlinked(proj, actorID, mentioned, note)
		return nil
	}

	created, err := s.repo.AddSourcedComment(newSourcedComment(item.ID, basecampLineKey(ev),
		basecampLine(ev.Kind, note, targetTitle, targetURL)))
	if err != nil {
		return err
	}
	if !created {
		// Ya estaba: lo trajo otra entrega del mismo recording.
		return nil
	}
	if s.hub != nil {
		s.hub.Publish(events.Event{Type: "task:comment", OrgID: item.OrgID,
			Data: map[string]string{"listId": item.ListID, "taskId": item.ID}})
	}
	s.notifyLinked(item, actorID, mentioned, note)
	if ev.Kind == domain.BasecampCommentCreated && actorID != "" && s.requests != nil {
		answer := note.excerpt
		if note.url != "" {
			answer += "\n\n" + note.url
		}
		answered, open, err := s.requests.AnswerFromBasecamp(item.OrgID, item.ID, actorID, answer)
		if err != nil {
			lg.Warn("basecamp: answering a request from a comment: " + err.Error())
		}
		if !answered && open > 1 && s.inbox != nil {
			// No se adivina a cuál contesta: se le dice que lo haga en cac.
			s.inbox.Notify(domain.Aviso{UserID: actorID, OrgID: item.OrgID, Kind: domain.RequestNotifyKind,
				TitleKey: "notify.basecamp.request_ambiguous", TitleArgs: map[string]string{"count": strconv.Itoa(open)},
				Body: item.Title, Link: "/tasks?task=" + item.ID, Group: domain.ItemGroup(item.ID), Label: item.Title,
				SourceKey: note.key})
		}
	}
	return nil
}

// basecampLineKey: la procedencia de la línea, una por cosa que pasó. Un
// comentario o un mensaje nacen una vez, así que basta su recording (otra
// entrega del mismo no lo repite); un to-do se puede reabrir y completar otra
// vez, y cada completada es otra línea: lleva su hora.
func basecampLineKey(ev domain.BasecampWebhookEvent) string {
	key := fmt.Sprintf("bc:%s:%d", ev.Kind, ev.Recording.ID)
	if ev.Kind != domain.BasecampTodoCompleted && ev.Kind != domain.BasecampTodoUncompleted {
		return key
	}
	if ev.CreatedAt.IsZero() {
		return key + ":e" + strconv.FormatInt(ev.ID, 10)
	}
	return key + ":" + strconv.FormatInt(ev.CreatedAt.Unix(), 10)
}

// basecampLine: la línea que queda en la tarea. Interna: lo de Basecamp lo ve
// el equipo, no el cliente que mira la tarea en su tablero.
func basecampLine(kind string, n basecampNote, title, targetURL string) string {
	link := mdText(strings.TrimSpace(title))
	if link == "" {
		link = "Basecamp"
	}
	if u := domain.SafeBasecampURL(targetURL); u != "" {
		link = "[" + link + "](" + mdURL(u) + ")"
	}
	who := "**" + mdText(n.who) + "**"
	var head string
	switch kind {
	case domain.BasecampMessageCreated:
		head = who + " posted " + link + " in Basecamp"
	case domain.BasecampTodoCompleted:
		return who + " completed " + link + " in Basecamp"
	case domain.BasecampTodoUncompleted:
		return who + " reopened " + link + " in Basecamp"
	default:
		head = who + " commented on " + link + " in Basecamp"
	}
	if n.excerpt == "" {
		return head
	}
	lines := strings.Split(n.excerpt, "\n")
	for i, l := range lines {
		lines[i] = "> " + l
	}
	return head + ":\n\n" + strings.Join(lines, "\n")
}

func mdURL(u string) string {
	return strings.NewReplacer("(", "%28", ")", "%29", " ", "%20").Replace(u)
}

// notifyLinked: a quien nombra, como mención; a los responsables de la tarea,
// como comentario. Quien está en las dos listas recibe sólo la mención.
func (s *BasecampService) notifyLinked(item *domain.Item, actorID string, mentioned []string, n basecampNote) {
	if s.inbox == nil {
		return
	}
	link := "/tasks?task=" + item.ID
	told := map[string]bool{actorID: true}
	for _, uid := range mentioned {
		told[uid] = true
		s.inbox.Notify(domain.Aviso{UserID: uid, OrgID: item.OrgID, Kind: domain.BasecampMentionKind,
			TitleKey: "notify.basecamp.mention", TitleArgs: map[string]string{"who": n.who},
			Body: n.excerpt, Link: link, Group: domain.ItemGroup(item.ID), Label: item.Title, SourceKey: n.key})
	}
	assignees, err := s.repo.Assignees(item.ID)
	if err != nil {
		return
	}
	for _, uid := range assignees {
		if uid == "" || told[uid] {
			continue
		}
		told[uid] = true
		s.inbox.Notify(domain.Aviso{UserID: uid, OrgID: item.OrgID, Kind: "task:comment",
			TitleKey: n.titleKey, TitleArgs: map[string]string{"who": n.who},
			Body: n.excerpt, Link: link, Group: domain.ItemGroup(item.ID), Label: item.Title, SourceKey: n.key})
	}
}

// notifyUnlinked: sin tarea enlazada sólo se avisa, con el enlace a Basecamp.
// A quien nombra; y si no nombra a nadie, a quien enlazó el proyecto, para
// que no se pierda en silencio.
func (s *BasecampService) notifyUnlinked(proj *domain.BasecampProject, actorID string, mentioned []string, n basecampNote) {
	if s.inbox == nil {
		return
	}
	group := domain.BasecampProjectGroup(proj.ProjectID)
	for _, uid := range mentioned {
		s.inbox.Notify(domain.Aviso{UserID: uid, OrgID: proj.OrgID, Kind: domain.BasecampMentionKind,
			TitleKey: "notify.basecamp.mention", TitleArgs: map[string]string{"who": n.who},
			Body: n.excerpt, Link: n.url, Group: group, Label: proj.Name, SourceKey: n.key})
	}
	if len(mentioned) > 0 || proj.LinkedByID == "" || proj.LinkedByID == actorID {
		return
	}
	s.inbox.Notify(domain.Aviso{UserID: proj.LinkedByID, OrgID: proj.OrgID, Kind: domain.BasecampActivityKind,
		TitleKey: n.titleKey, TitleArgs: map[string]string{"who": n.who},
		Body: n.excerpt, Link: n.url, Group: group, Label: proj.Name, SourceKey: n.key})
}

// Task: la tarea, para que el handler compruebe su org.
func (s *BasecampService) Task(id string) (*domain.Item, error) {
	return s.repo.Item(id)
}
