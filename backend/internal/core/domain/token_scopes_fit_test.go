package domain

import (
	"reflect"
	"regexp"
	"strconv"
	"strings"
	"testing"
)

// Un token con todos los scopes tiene que caber en su columna. Con chat:manage
// (command-and-control-547) la lista pasó de 200 caracteres, que era el
// tamaño de `scopes`: guardar un token con todo fallaba en la base.
//
// Roturas que detecta: devolver la columna a varchar(200), o añadir un scope
// que la desborde sin ensancharla.
func TestEveryScopeFitsInOneToken(t *testing.T) {
	field, _ := reflect.TypeOf(PersonalAccessToken{}).FieldByName("Scopes")
	m := regexp.MustCompile(`varchar\((\d+)\)`).FindStringSubmatch(field.Tag.Get("gorm"))
	if m == nil {
		t.Fatalf("la columna scopes no dice su tamaño: %q", field.Tag.Get("gorm"))
	}
	size, _ := strconv.Atoi(m[1])
	all := strings.Join(AllScopes, ",")
	if len(all) > size {
		t.Fatalf("todos los scopes ocupan %d y la columna es de %d", len(all), size)
	}
	if !ValidScope(ScopeChatManage) {
		t.Fatal("chat:manage tiene que poder pedirse al crear un token")
	}
}
