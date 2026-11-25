package repository

import (
	"fmt"
	"testing"
	"time"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

/*
Un usuario se encuentra aunque se escriba con otras mayúsculas.

El 25-nov Itzel pidió tres veces otra contraseña como «itzramn» y su cuenta es
«Itzramn»: la búsqueda era exacta, no la encontraba y, como la pantalla contesta
lo mismo exista o no, no supo nunca por qué no le llegaba nada. Entrar fallaba
igual.

Roturas que atrapa: volver a la comparación exacta al entrar o al pedir otra
contraseña; tomar la cuenta de otro cuando dos sólo cambian en mayúsculas;
dejar crear «itzramn» con «Itzramn» ya dada.
*/
func TestAUsernameIsFoundWhateverItsCase(t *testing.T) {
	db, cleanup := usernameCaseDB(t)
	defer cleanup()
	auth := NewAuthRepository(db)
	accounts := NewAccountRepository(db)

	for _, typed := range []string{"Itzramn", "itzramn", "ITZRAMN"} {
		u, err := auth.FindByUsername(typed)
		if err != nil || u.ID != "u-itzel" {
			t.Errorf("entrar como %q: %v %v", typed, u, err)
		}
		u, err = accounts.FindByLogin(typed)
		if err != nil || u.ID != "u-itzel" {
			t.Errorf("pedir otra contraseña como %q: %v %v", typed, u, err)
		}
	}

	// «Bob» y «bob» existen los dos: la forma exacta gana, y otra forma no
	// elige ninguna.
	if u, err := auth.FindByUsername("bob"); err != nil || u.ID != "u-bob-lower" {
		t.Errorf("la forma exacta tiene que ganar: %v %v", u, err)
	}
	if u, err := auth.FindByUsername("BOB"); err == nil {
		t.Errorf("con dos que sólo cambian en mayúsculas no se elige ninguna, y salió %s", u.ID)
	}
	if u, err := accounts.FindByLogin("BOB"); err == nil {
		t.Errorf("pedir otra contraseña tampoco elige una de las dos, y salió %s", u.ID)
	}

	copy := &domain.User{Username: "itzramn", Email: "c@x.io", Password: "x"}
	copy.ID = "u-copy"
	if err := auth.CreateUser(copy); err != ErrUsernameTaken {
		t.Errorf("«itzramn» con «Itzramn» ya dada: %v", err)
	}
}

func usernameCaseDB(t *testing.T) (*gorm.DB, func()) {
	t.Helper()
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
	const name = "cac_test_username_case"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	adminSQL, _ := admin.DB()

	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	if err := db.AutoMigrate(&domain.User{}); err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	for _, u := range [][3]string{{"u-itzel", "Itzramn", "itzel@x.io"}, {"u-bob-upper", "Bob", "bob1@x.io"}, {"u-bob-lower", "bob", "bob2@x.io"}} {
		if err := db.Exec(`INSERT INTO users (id, username, email, password, created_at, updated_at) VALUES (?,?,?,'x',?,?)`,
			u[0], u[1], u[2], now, now).Error; err != nil {
			t.Fatal(err)
		}
	}

	return db, func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		adminSQL.Close()
	}
}
