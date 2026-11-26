package repository

import (
	"fmt"
	"testing"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"
)

// `created_via` se rellena con el `via` de cada fila **una sola vez**, y luego
// lo mantiene el modelo (command-and-control-547).
//
// Roturas que detecta: no rellenar las filas que ya estaban (un mensaje que
// dejó un agente dejaría de poder retirarse con su token); rellenar en cada
// arranque (un mensaje tecleado a mano y corregido por un agente quedaría
// retirable por el token); que corregir con `Rewrite` toque `created_via`; y
// que un mensaje nuevo no la guarde al crearse.
func TestCreatedViaIsFilledOnceAndNeverFollowsAnEdit(t *testing.T) {
	if GetEnv("DB_HOST", "") == "" {
		t.Skip("no database configured")
	}
	dsn := func(name string) string {
		return fmt.Sprintf("host=%s port=%s user=%s password=%s dbname=%s sslmode=%s",
			GetEnv("DB_HOST", "localhost"), GetEnv("DB_PORT", "5432"),
			GetEnv("DB_USER", "postgres"), GetEnv("DB_PASSWORD", ""),
			name, GetEnv("DB_SSLMODE", "disable"))
	}
	admin, err := gorm.Open(postgres.Open(dsn(GetEnv("DB_NAME", "cac"))), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatalf("DB_HOST is set but the database does not answer (a skipped test is not a green one): %v", err)
	}
	const name = "cac_test_created_via_migration"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if sqlDB, e := db.DB(); e == nil {
			sqlDB.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		if s, e := admin.DB(); e == nil {
			s.Close()
		}
	}()

	// El esquema de antes, sin la columna: `AutoMigrate` la crearía ya.
	if err := db.AutoMigrate(&domain.ChatMessage{}, &domain.DMMessage{}); err != nil {
		t.Fatal(err)
	}
	for _, table := range createdViaTables {
		db.Exec(fmt.Sprintf(`ALTER TABLE %s DROP COLUMN created_via`, table))
	}
	db.Exec(`INSERT INTO chat_messages (id, space_id, org_id, author_user_id, body, kind, via)
	         VALUES ('c-agent', 's', 'o', 'u', 'x', 'user', 'mcp'), ('c-typed', 's', 'o', 'u', 'x', 'user', '')`)
	db.Exec(`INSERT INTO dm_messages (id, conversation_id, org_id, author_user_id, body, via)
	         VALUES ('d-agent', 'c', 'o', 'u', 'x', 'mcp'), ('d-typed', 'c', 'o', 'u', 'x', '')`)

	addCreatedVia(db)
	createdVia := func(table, id string) string {
		var v string
		db.Raw(fmt.Sprintf(`SELECT created_via FROM %s WHERE id = ?`, table), id).Scan(&v)
		return v
	}
	for _, c := range []struct{ table, id, want string }{
		{"chat_messages", "c-agent", "mcp"}, {"chat_messages", "c-typed", ""},
		{"dm_messages", "d-agent", "mcp"}, {"dm_messages", "d-typed", ""},
	} {
		if got := createdVia(c.table, c.id); got != c.want {
			t.Errorf("%s %s: created_via %q, quería %q", c.table, c.id, got, c.want)
		}
	}

	// Lo tecleado a mano, corregido por un agente: `via` pasa a mcp, la de
	// creación no, ni al corregir ni en el arranque siguiente.
	if err := NewChatRepository(db).Rewrite("c-typed", "y", domain.ViaMCP); err != nil {
		t.Fatal(err)
	}
	if err := NewDMRepository(db).Rewrite("d-typed", "y", domain.ViaMCP); err != nil {
		t.Fatal(err)
	}
	addCreatedVia(db)
	if createdVia("chat_messages", "c-typed") != "" || createdVia("dm_messages", "d-typed") != "" {
		t.Error("corregido por un agente sigue creado a mano, también tras otro arranque")
	}

	// Uno nuevo la guarda al crearse.
	m := &domain.ChatMessage{SpaceID: "s", OrgID: "o", AuthorUserID: "u", Body: "z", Kind: domain.ChatKindUser, Via: domain.ViaMCP}
	if err := db.Create(m).Error; err != nil {
		t.Fatal(err)
	}
	d := &domain.DMMessage{ConversationID: "c", OrgID: "o", AuthorUserID: "u", Body: "z", Via: domain.ViaMCP}
	if err := db.Create(d).Error; err != nil {
		t.Fatal(err)
	}
	if createdVia("chat_messages", m.ID) != "mcp" || createdVia("dm_messages", d.ID) != "mcp" {
		t.Error("un mensaje que deja un agente nace creado por un agente")
	}
}
