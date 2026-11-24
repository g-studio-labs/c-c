package repository

import (
	"strings"
	"testing"
)

// Un enlace abre su página compartida y ninguna otra, y una firma inventada no
// abre nada.
func TestADocShareLinkOpensOnlyItsShare(t *testing.T) {
	link := SignDocShareLink("share-a")
	if id, ok := VerifyDocShareLink(link); !ok || id != "share-a" {
		t.Fatalf("the share's own link does not open: %q %v", id, ok)
	}
	_, sig, _ := strings.Cut(link, ".")
	if _, ok := VerifyDocShareLink("share-b." + sig); ok {
		t.Fatal("one share's signature opens another")
	}
	forged := "share-a." + strings.Repeat("0", len(sig))
	for _, bad := range []string{"", "share-a", "share-a.", "." + sig, forged} {
		if _, ok := VerifyDocShareLink(bad); ok {
			t.Fatalf("%q opens a share", bad)
		}
	}
}

// Una firma de otra etiqueta no pasa por la de un documento compartido: el
// enlace de una llamada con el mismo id no abre un documento.
func TestADocShareIsNotSignedWithAnotherLabel(t *testing.T) {
	if _, ok := VerifyDocShareLink(SignCallInviteLink("same-id")); ok {
		t.Fatal("a call invite link opens a shared doc")
	}
	for name, key := range map[string][]byte{
		"call-invite": callInviteKey(), "call-guest": callGuestKey(),
		"report": reportTokenSecret(), "url-ticket": urlTicketKey(),
	} {
		if string(docShareKey()) == string(key) {
			t.Fatalf("the doc-share key is the %s key", name)
		}
	}
}
