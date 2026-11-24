package repository

import (
	"crypto/hmac"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"strings"
)

// El enlace de una página compartida es `<shareID>.<firma>`, como el de una
// invitación a una llamada (call_token.go) y con su propia etiqueta: un enlace
// de llamada no abre un documento ni al revés, aunque los dos firmen un id.

func docShareKey() []byte { return callKey("doc-share") }

func signDocShare(shareID string) string {
	mac := hmac.New(sha256.New, docShareKey())
	mac.Write([]byte(shareID))
	return hex.EncodeToString(mac.Sum(nil))
}

// SignDocShareLink: el token de un enlace compartido.
func SignDocShareLink(shareID string) string {
	return shareID + "." + signDocShare(shareID)
}

// VerifyDocShareLink: de qué enlace es un token, si la firma es buena. Que siga
// vivo lo dice la fila.
func VerifyDocShareLink(token string) (string, bool) {
	id, sig, ok := strings.Cut(token, ".")
	if !ok || id == "" || sig == "" {
		return "", false
	}
	if subtle.ConstantTimeCompare([]byte(signDocShare(id)), []byte(sig)) != 1 {
		return "", false
	}
	return id, true
}
