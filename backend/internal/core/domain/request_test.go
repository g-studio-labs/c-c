package domain

import (
	"errors"
	"strings"
	"testing"
)

// Cada tipo de pedido exige lo que hace falta para contestarlo sin preguntar,
// y en «decidir» la recomendada queda primero.
//
// Roturas que atrapa: aceptar un mensaje sin destino o un «probar» sin lo que
// debe verse; aceptar una sola opción o dos recomendadas; dejar la
// recomendada donde la puso el agente; un tipo desconocido que pasa.
func TestEachKindAsksForWhatAnswersIt(t *testing.T) {
	bad := map[RequestKind]RequestBody{
		RequestApproveMessage: {Draft: "Hola"},
		RequestTest:           {Steps: []string{"abrir"}, Expected: "  "},
		RequestRun:            {Command: "make", Where: "gstd"},
		RequestObtain:         {What: "la llave"},
		RequestDecide:         {Options: []RequestOption{{Label: "A", Consequence: "x"}}},
		RequestKind("other"):  {Draft: "x"},
	}
	for kind, b := range bad {
		if _, err := b.Normalize(kind); !errors.Is(err, ErrRequestBody) {
			t.Errorf("%s: %+v tenía que rechazarse, salió %v", kind, b, err)
		}
	}
	two := RequestBody{Options: []RequestOption{
		{Label: "A", Consequence: "x", Recommended: true}, {Label: "B", Consequence: "y", Recommended: true},
	}}
	if _, err := two.Normalize(RequestDecide); err == nil {
		t.Error("dos recomendadas no pueden pasar")
	}
	got, err := RequestBody{Question: "¿Cuál?", Options: []RequestOption{
		{Label: "A", Consequence: "x"}, {Label: "B", Consequence: "y"}, {Label: "C", Consequence: "z", Recommended: true},
	}}.Normalize(RequestDecide)
	if err != nil {
		t.Fatal(err)
	}
	labels := []string{got.Options[0].Label, got.Options[1].Label, got.Options[2].Label}
	if strings.Join(labels, "") != "CAB" {
		t.Fatalf("la recomendada va primero y las demás en su orden: %v", labels)
	}
	ok, err := RequestBody{Draft: " Hola ", Destination: "Basecamp", Command: "se descarta"}.Normalize(RequestApproveMessage)
	if err != nil || ok.Draft != "Hola" || ok.Command != "" {
		t.Fatalf("se guarda limpio y sólo lo de su tipo: %+v %v", ok, err)
	}
}

// La respuesta se valida contra el pedido: «no funciona» sin nota no vale,
// rechazar pide motivo, la opción tiene que ser una de las ofrecidas, y lo
// aprobado sin editar es el borrador tal cual (y dice si se editó).
//
// Roturas que atrapa: dejar pasar un «no funciona» mudo; un rechazo sin
// motivo; elegir algo que no se ofreció; aprobar sin congelar el texto, o
// marcar como editado lo que no lo está.
func TestAnAnswerIsCheckedAgainstItsRequest(t *testing.T) {
	yes, no := true, false
	draft := RequestBody{Draft: "Texto exacto", Destination: "hilo"}

	if _, err := (RequestAnswer{Works: &no}).Normalize(RequestTest, RequestBody{}); !errors.Is(err, ErrRequestAnswer) {
		t.Errorf("«no funciona» sin nota tenía que rechazarse: %v", err)
	}
	if a, err := (RequestAnswer{Works: &no, Note: " se ve 07:00 "}).Normalize(RequestTest, RequestBody{}); err != nil || a.Note != "se ve 07:00" {
		t.Errorf("con nota vale: %+v %v", a, err)
	}
	if _, err := (RequestAnswer{Approved: &no}).Normalize(RequestApproveMessage, draft); !errors.Is(err, ErrRequestAnswer) {
		t.Errorf("rechazar sin motivo tenía que rechazarse: %v", err)
	}
	a, err := (RequestAnswer{Approved: &yes}).Normalize(RequestApproveMessage, draft)
	if err != nil || a.FinalText != "Texto exacto" || a.Edited {
		t.Errorf("aprobar sin editar congela el borrador: %+v %v", a, err)
	}
	a, err = (RequestAnswer{Approved: &yes, FinalText: "Otro texto"}).Normalize(RequestApproveMessage, draft)
	if err != nil || a.FinalText != "Otro texto" || !a.Edited {
		t.Errorf("editar y aprobar guarda la versión final y lo dice: %+v %v", a, err)
	}
	opts := RequestBody{Options: []RequestOption{{Label: "Sí", Consequence: "x"}, {Label: "No", Consequence: "y"}}}
	if _, err := (RequestAnswer{Option: "Quizá"}).Normalize(RequestDecide, opts); !errors.Is(err, ErrRequestAnswer) {
		t.Errorf("una opción que no se ofreció no vale: %v", err)
	}
	if a, err := (RequestAnswer{Option: "sí"}).Normalize(RequestDecide, opts); err != nil || a.Option != "Sí" {
		t.Errorf("la opción se guarda como se ofreció: %+v %v", a, err)
	}
}

// El ID corto es `req-` y seis letras que no se confunden al dictarlas.
//
// Rotura que atrapa: un alfabeto con 0/o o 1/l, o un ID de otra forma.
func TestARequestCodeCanBeDictated(t *testing.T) {
	for range 200 {
		c := NewRequestCode()
		if !LooksLikeRequestCode(c) || len(c) != 10 || strings.ContainsAny(c[4:], "01oli") {
			t.Fatalf("ID que no se dicta bien: %q", c)
		}
	}
}

// Un pedido lleva ficheros de texto con su contenido (command-and-control-567),
// en cualquier tipo, y el backend pone los límites con su motivo.
//
// Roturas que atrapa: perder los ficheros al normalizar; aceptar uno de más,
// uno demasiado grande, uno que no es texto o un nombre con ruta; aceptar un
// role inventado; tocar el contenido.
func TestARequestCarriesTextFilesWithinLimits(t *testing.T) {
	run := RequestBody{Command: "pegar apply.sql en el SQL Editor", Where: "Supabase", Expected: "UPDATE 3"}
	run.Files = []RequestFile{
		{Name: " apply.sql ", Content: "update t set x = 1;\n", Role: "apply"},
		{Name: "restore.sql", Content: "update t set x = 0;\n", Role: "UNDO"},
	}
	got, err := run.Normalize(RequestRun)
	if err != nil {
		t.Fatal(err)
	}
	want := []RequestFile{
		{Name: "apply.sql", Content: "update t set x = 1;\n", Role: "apply"},
		{Name: "restore.sql", Content: "update t set x = 0;\n", Role: "undo"},
	}
	if len(got.Files) != 2 || got.Files[0] != want[0] || got.Files[1] != want[1] {
		t.Fatalf("los ficheros se guardan con su contenido intacto: %+v", got.Files)
	}
	if got, err := (RequestBody{What: "la llave", From: "Ana", Files: want[:1]}).Normalize(RequestObtain); err != nil || len(got.Files) != 1 {
		t.Fatalf("vale para cualquier tipo: %+v %v", got.Files, err)
	}
	if got, err := (RequestBody{What: "x", From: "y"}).Normalize(RequestObtain); err != nil || got.Files != nil {
		t.Fatalf("un pedido sin ficheros sigue igual: %+v %v", got.Files, err)
	}

	many := make([]RequestFile, MaxRequestFiles+1)
	for i := range many {
		many[i] = RequestFile{Name: string(rune('a'+i)) + ".sql", Content: "select 1;"}
	}
	bad := map[string][]RequestFile{
		"one too many":  many,
		"too big":       {{Name: "big.sql", Content: strings.Repeat("x", MaxRequestFileBytes+1)}},
		"not text":      {{Name: "blob.sql", Content: "abc\xff\xfe"}},
		"nul byte":      {{Name: "blob.sql", Content: "abc\x00def"}},
		"path":          {{Name: "/home/rv/apply.sql", Content: "select 1;"}},
		"windows path":  {{Name: `C:\tmp\apply.sql`, Content: "select 1;"}},
		"dot dot":       {{Name: "..", Content: "select 1;"}},
		"no name":       {{Name: " ", Content: "select 1;"}},
		"unknown role":  {{Name: "a.sql", Content: "select 1;", Role: "run"}},
		"empty":         {{Name: "a.sql", Content: ""}},
		"same name":     {{Name: "a.sql", Content: "1"}, {Name: "a.sql", Content: "2"}},
		"total too big": {{Name: "a", Content: strings.Repeat("x", MaxRequestFileBytes)}, {Name: "b", Content: strings.Repeat("x", MaxRequestFileBytes)}, {Name: "c", Content: strings.Repeat("x", MaxRequestFileBytes)}, {Name: "d", Content: strings.Repeat("x", MaxRequestFileBytes)}, {Name: "e", Content: "x"}},
	}
	for why, files := range bad {
		b := run
		b.Files = files
		_, err := b.Normalize(RequestRun)
		var be *RequestBodyError
		if !errors.As(err, &be) || be.Msg == "" {
			t.Errorf("%s: tenía que rechazarse con motivo, salió %v", why, err)
		}
	}
	exact := run
	exact.Files = []RequestFile{{Name: "a.sql", Content: strings.Repeat("x", MaxRequestFileBytes)}}
	if _, err := exact.Normalize(RequestRun); err != nil {
		t.Errorf("justo en el límite pasa: %v", err)
	}
}
