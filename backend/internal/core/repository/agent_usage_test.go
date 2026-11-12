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

func agentUsageDB(t *testing.T) *gorm.DB {
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
	const name = "cac_test_agent_usage"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	adminSQL, _ := admin.DB()
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		adminSQL.Close()
	})
	if err := db.AutoMigrate(&domain.AgentCall{}); err != nil {
		t.Fatal(err)
	}
	return db
}

// Lo que se lee para decidir si un hook ayuda o es ruido
// (command-and-control-347): cuántas veces se llamó cada herramienta, y de lo
// que enseñó cada hook, cuánto abrió después la misma persona.
//
// Roturas que atrapa: contar dos veces una llamada que hace varias peticiones
// (mismo callID); mezclar los hooks con las herramientas; contar como abierto
// lo que abrió otra persona, lo abierto antes de enseñarlo o mucho después;
// contar como abierto que otro hook lo enseñara otra vez; no acotar a quien
// pregunta.
func TestUsageCountsCallsOnceAndWhatWasShownThenOpened(t *testing.T) {
	db := agentUsageDB(t)
	r := NewAgentUsageRepository(db)
	at := func(callID string, d time.Duration) {
		if err := db.Model(&domain.AgentCall{}).Where("call_id = ?", callID).Update("created_at", time.Now().Add(d)).Error; err != nil {
			t.Fatal(err)
		}
	}
	for _, c := range [][4]string{
		{"u1", "get_task", "c1", "cc-7"},
		{"u1", "get_task", "c1", "cc-7"}, // la segunda petición de la misma llamada
		{"u1", "get_incident", "c2", "inc-1"},
		{"u2", "get_incident", "c3", "inc-2"}, // otra persona abre lo que se le enseñó a u1
		{"u1", "get_incident", "c4", "inc-3"}, // antes de enseñarlo
		{"u1", "get_incident", "c5", "inc-4"}, // tres horas después
	} {
		if err := r.RecordCall(c[0], c[1], c[2], c[3]); err != nil {
			t.Fatal(err)
		}
	}
	if err := r.RecordShown("u1", domain.AgentShownRequest{Hook: "cac-prompt",
		Targets: []string{"inc-1", "inc-2", "inc-3", "inc-4", "inc-5"}}); err != nil {
		t.Fatal(err)
	}
	if err := r.RecordShown("u1", domain.AgentShownRequest{Hook: "cac-edit", Targets: []string{"inc-5"}}); err != nil {
		t.Fatal(err)
	}
	// Lo enseñado, hace una hora; lo abierto, alrededor.
	db.Model(&domain.AgentCall{}).Where("tool LIKE 'hook:%'").Update("created_at", time.Now().Add(-time.Hour))
	at("c2", -30*time.Minute)
	at("c3", -30*time.Minute)
	at("c4", -2*time.Hour)
	at("c5", 2*time.Hour)

	all, err := r.Usage("", time.Now().Add(-24*time.Hour))
	if err != nil {
		t.Fatal(err)
	}
	calls := map[string]int{}
	for _, x := range all.Tools {
		calls[x.Tool] = x.Calls
	}
	if calls["get_task"] != 1 || calls["get_incident"] != 4 || len(calls) != 2 {
		t.Errorf("herramientas: %+v", all.Tools)
	}
	shown := map[string][2]int{}
	for _, x := range all.Shown {
		shown[x.Hook] = [2]int{x.Shown, x.Opened}
	}
	if shown["cac-prompt"] != [2]int{5, 1} {
		t.Errorf("cac-prompt enseñó 5 y u1 abrió 1 a tiempo: %+v", shown["cac-prompt"])
	}
	if shown["cac-edit"] != [2]int{1, 0} {
		t.Errorf("que otro hook lo enseñe no es abrirlo: %+v", shown["cac-edit"])
	}

	mine, _ := r.Usage("u2", time.Now().Add(-24*time.Hour))
	if len(mine.Tools) != 1 || mine.Tools[0].Calls != 1 || len(mine.Shown) != 0 {
		t.Errorf("u2 ve lo de otros: %+v", mine)
	}
}
