package http

import (
	"bytes"
	"log"
	"net/http"
	"net/http/httptest"
	"os"
	"regexp"
	"sort"
	"strings"
	"sync"
	"testing"

	"github.com/go-chi/chi/v5"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"
)

// Ninguna credencial que viaje en el path de una ruta llega al log (QA
// command-and-control-555). El log de peticiones escribía la ruta entera, y el
// webhook de Basecamp lleva su secreto en ella: quien leyera los logs del
// clúster podía inventar comentarios y menciones en ese proyecto.
//
// Se recorre el router **entero** (`InitRoutes`) y cada parámetro de ruta
// tiene que estar clasificado: o no es secreto (un id, que sin sesión no abre
// nada), o es secreto y entonces una petición de verdad —por el método de la
// ruta, por otro y con un tramo de más— no deja el valor en ningún log: ni el
// de peticiones ni el del handler. Un parámetro nuevo sin clasificar falla
// aquí, que es justo cuando alguien tiene que pensar si es un secreto.
//
// Roturas que atrapa: quitar Basecamp de `middleware.SecretPaths`; que el
// logger vuelva a escribir `RequestURI` tal cual; colgar una ruta con un
// parámetro nuevo sin decir qué es.
func TestEverySecretInAPathIsRedacted(t *testing.T) {
	// Una base que no contesta: montar el router no consulta nada, y lo que
	// pregunte un handler falla rápido (puerto 1, conexión rechazada).
	db, err := gorm.Open(postgres.New(postgres.Config{
		DSN: "host=127.0.0.1 port=1 user=x dbname=x sslmode=disable connect_timeout=1",
	}), &gorm.Config{DisableAutomaticPing: true, Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}

	// Los relojes de fondo que arranca el router también escriben en el log.
	logs := &lockedBuffer{}
	log.SetOutput(logs)
	t.Cleanup(func() { log.SetOutput(os.Stderr) })

	r := InitRoutes(db)

	param := regexp.MustCompile(`\{([^}:]+)(?::[^}]*)?\}`)
	unknown := map[string][]string{}
	var secretRoutes []string
	seen := 0
	err = chi.Walk(r, func(method, route string, _ http.Handler, _ ...func(http.Handler) http.Handler) error {
		seen++
		secret := false
		for _, m := range param.FindAllStringSubmatch(route, -1) {
			switch {
			case secretPathParams[m[1]]:
				secret = true
			case !publicPathParams[m[1]]:
				unknown[m[1]] = append(unknown[m[1]], method+" "+route)
			}
		}
		if secret {
			secretRoutes = append(secretRoutes, method+" "+route)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	// Sin esto, un recorrido que no viera el router pasaría sin mirar nada.
	if seen < 300 {
		t.Fatalf("el recorrido vio sólo %d rutas", seen)
	}
	if len(secretRoutes) == 0 {
		t.Fatal("ninguna ruta con un secreto en el path: ¿se fue el webhook de Basecamp?")
	}
	names := make([]string, 0, len(unknown))
	for n := range unknown {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, n := range names {
		t.Errorf("parámetro de ruta {%s} sin clasificar (%s): si lleva una credencial, va en secretPathParams "+
			"y su ruta en middleware.SecretPaths; si no, en publicPathParams", n, unknown[n][0])
	}

	const sentinel = "Sentinel-Secret-0123456789abcdefXYZ"
	for _, mr := range secretRoutes {
		method, route, _ := strings.Cut(mr, " ")
		path := param.ReplaceAllStringFunc(route, func(p string) string {
			if secretPathParams[param.FindStringSubmatch(p)[1]] {
				return sentinel
			}
			return "x"
		})
		path = strings.TrimSuffix(path, "/*")
		other := http.MethodGet
		if method == http.MethodGet {
			other = http.MethodPost
		}
		for _, try := range [][2]string{
			{method, path},
			{method, path + "?code=c&state=s"},
			{other, path},
			{method, path + "/more"},
		} {
			logs.Reset()
			req := httptest.NewRequest(try[0], try[1], strings.NewReader(`{"id":1,"kind":"comment_created"}`))
			r.ServeHTTP(httptest.NewRecorder(), req)
			got := logs.String()
			if strings.Contains(got, sentinel) || strings.Contains(got, sentinel[:20]) {
				t.Errorf("%s %s deja el secreto en el log:\n%s", try[0], route, got)
			}
			if got == "" {
				t.Errorf("%s %s no escribió nada en el log: la prueba no estaría mirando", try[0], route)
			}
		}
	}
}

// secretPathParams: parámetros de ruta que son una credencial.
var secretPathParams = map[string]bool{
	"secret": true,
}

// publicPathParams: parámetros de ruta que no abren nada por sí solos (ids,
// nombres, refs). Lo que protege esas rutas es la sesión (o el pase del agente
// en /agent/v1), no el valor. El enlace público de una llamada no está aquí:
// su pase va en el cuerpo, no en la ruta.
var publicPathParams = map[string]bool{
	"id":             true,
	"kind":           true,
	"ownerId":        true,
	"pageId":         true,
	"tab":            true,
	"versionId":      true,
	"linkId":         true,
	"attachmentId":   true,
	"commentId":      true,
	"messageId":      true,
	"mid":            true,
	"orgId":          true,
	"userId":         true,
	"projectId":      true,
	"personId":       true,
	"recordingId":    true,
	"ref":            true,
	"repoId":         true,
	"installationId": true,
	"invitationId":   true,
	"did":            true,
	"depId":          true,
	"iid":            true,
	"rid":            true,
	"rqid":           true,
	"tid":            true,
	"tplId":          true,
	"seq":            true,
	"sourceId":       true,
	"bid":            true,
	"jid":            true,
	"imageId":        true,
	"deviceId":       true,
	"guestId":        true,
	"identity":       true,
}

type lockedBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *lockedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *lockedBuffer) Reset() {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.buf.Reset()
}

func (b *lockedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}
