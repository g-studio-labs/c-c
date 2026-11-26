package domain

import (
	"slices"
	"testing"
)

// Un `@usuario` escrito a mano (por el MCP, o tecleado sin el menú) se vuelve
// el enlace que escribe la app. Ver command-and-control-550: un mensaje del MCP
// con `@Diegotsx @itzramn` salió como texto y no avisó a nadie.
func TestHandMentionsBecomeLinks(t *testing.T) {
	const bea = "22222222-2222-4222-8222-222222222222"
	members := map[string]MentionTarget{
		"bea":      {ID: bea, Username: "bea", Name: "Bea Ruiz"},
		"diegotsx": {ID: "44444444-4444-4444-8444-444444444444", Username: "diegotsx", Name: ""},
	}
	link := "[@Bea Ruiz](" + MentionRef(bea) + ")"

	cases := []struct{ name, in, want string }{
		{"una mención en prosa", "hola @bea, mira", "hola " + link + ", mira"},
		{"sin distinguir mayúsculas", "@BEA ya", link + " ya"},
		{"al principio de una línea", "uno\n@bea dos", "uno\n" + link + " dos"},
		{"el punto final es de la frase", "avisa a @bea.", "avisa a " + link + "."},
		{"sin nombre, el usuario", "@Diegotsx", "[@diegotsx](" + MentionRef("44444444-4444-4444-8444-444444444444") + ")"},
		{"quien no es miembro se queda en texto", "hola @nadie", "hola @nadie"},
		{"un correo no se toca", "escribe a bea@cac.dev o a x.bea@cac.dev", "escribe a bea@cac.dev o a x.bea@cac.dev"},
		{"código en línea no se toca", "corre `ssh @bea` ya", "corre `ssh @bea` ya"},
		{"código en línea de dos acentos", "``a ` @bea`` y @bea", "``a ` @bea`` y " + link},
		{"un bloque de código no se toca", "antes\n```\n@bea\n```\ndespués @bea", "antes\n```\n@bea\n```\ndespués " + link},
		{"un bloque con ~~~ tampoco", "~~~sh\necho @bea\n~~~\n", "~~~sh\necho @bea\n~~~\n"},
		{"una mención que ya es enlace no cambia", "[@bea](" + MentionRef(bea) + ")", "[@bea](" + MentionRef(bea) + ")"},
		{"ni el texto de otro enlace", "[habla con @bea](https://x.io)", "[habla con @bea](https://x.io)"},
		{"ni una URL", "https://x.io/@bea y <https://y.io/@bea>", "https://x.io/@bea y <https://y.io/@bea>"},
		{"ni un escape", `\@bea`, `\@bea`},
		{"varias en una línea", "@bea y @diegotsx", link + " y [@diegotsx](" + MentionRef("44444444-4444-4444-8444-444444444444") + ")"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := LinkHandMentions(c.in, members); got != c.want {
				t.Errorf("\n in: %q\ngot: %q\nwant %q", c.in, got, c.want)
			}
		})
	}
}

// Lo que se le pregunta a la base: sólo lo escrito en prosa, una vez cada uno
// y en minúsculas.
func TestHandMentionsAreReadFromProseOnly(t *testing.T) {
	body := "@Bea y @bea, ana@x.io, `@caro`, [@dani](cac:user/x)\n```\n@eli\n```\n@Fer."
	if got, want := HandMentions(body), []string{"bea", "fer"}; !slices.Equal(got, want) {
		t.Errorf("got %v, want %v", got, want)
	}
}
