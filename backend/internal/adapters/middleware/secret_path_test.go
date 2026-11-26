package middleware

import (
	"strings"
	"testing"
)

// El secreto de la ruta del webhook de Basecamp no llega al log; su hermana
// `setup` se sigue leyendo, con su query tapada. El recorrido del router
// entero está en `TestEverySecretInAPathIsRedacted` (adapters/http).
//
// Roturas que atrapa: tapar también `setup` (el log ya no diría qué ruta
// fue); dejar de tapar la query al tapar el path.
func TestTheBasecampWebhookSecretIsRedacted(t *testing.T) {
	cases := map[string][]string{
		"/webhooks/basecamp/S3CRET":                         {"/webhooks/basecamp/REDACTED"},
		"/webhooks/basecamp/S3CRET/x?a=b":                   {"/webhooks/basecamp/REDACTED"},
		"/webhooks/basecamp/setup?code=S3CRET&state=S3CRET": {"/webhooks/basecamp/setup?", "code=REDACTED"},
		"/webhooks/basecamp/":                               {"/webhooks/basecamp/"},
	}
	for in, wants := range cases {
		got := redactURI(in)
		if strings.Contains(got, "S3CRET") {
			t.Errorf("%s → %s: el secreto sigue ahí", in, got)
		}
		for _, w := range wants {
			if !strings.Contains(got, w) {
				t.Errorf("%s → %s, se esperaba %q", in, got, w)
			}
		}
	}
}
