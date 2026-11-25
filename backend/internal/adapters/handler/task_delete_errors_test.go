package handler

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/go-chi/chi/v5"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
	"github.com/guz-studio/cac/backend/internal/core/service"
)

func taskDeleteDB(t *testing.T) (*gorm.DB, func()) {
	t.Helper()
	if repository.GetEnv("DB_HOST", "") == "" {
		t.Skip("no database configured")
	}
	dsn := func(name string) string {
		return fmt.Sprintf("host=%s port=%s user=%s password=%s dbname=%s sslmode=%s",
			repository.GetEnv("DB_HOST", "localhost"), repository.GetEnv("DB_PORT", "5432"),
			repository.GetEnv("DB_USER", "postgres"), repository.GetEnv("DB_PASSWORD", ""),
			name, repository.GetEnv("DB_SSLMODE", "disable"))
	}
	admin, err := gorm.Open(postgres.Open(dsn(repository.GetEnv("DB_NAME", "cac"))), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatalf("DB_HOST is set but the database does not answer (a skipped test is not a green one): %v", err)
	}
	const name = "cac_test_task_delete_errors"
	admin.Exec("DROP DATABASE IF EXISTS " + name)
	if err := admin.Exec("CREATE DATABASE " + name).Error; err != nil {
		t.Fatalf("DB_HOST is set but a throwaway database cannot be created: %v", err)
	}
	adminSQL, _ := admin.DB()
	db, err := gorm.Open(postgres.Open(dsn(name)), &gorm.Config{Logger: logger.Discard})
	if err != nil {
		t.Fatal(err)
	}
	if err := db.AutoMigrate(
		&domain.Organization{}, &domain.ReportProject{}, &domain.TaskSpace{}, &domain.TaskFolder{},
		&domain.TaskList{}, &domain.Item{}, &domain.ItemAssignee{}, &domain.TaskTagLink{},
		&domain.ItemComment{}, &domain.ItemAttachment{},
	); err != nil {
		t.Fatal(err)
	}
	return db, func() {
		if inner, _ := db.DB(); inner != nil {
			inner.Close()
		}
		admin.Exec("DROP DATABASE IF EXISTS " + name)
		adminSQL.Close()
	}
}

// Borrar una lista a la que entrega un proyecto de reportes contesta 409 con su
// código y el motivo, no un 500 sin código: quien borra lee «Hay un proyecto
// entregando aquí» y sabe qué hacer (command-and-control-385).
//
// Roturas que atrapa: que el manejador de borrar lista vuelva a contestar 500
// sin pasar por `mapTaskError`; y que el 409 no lleve el código
// `list-in-use-by-channel`.
func TestDeletingAListAReportProjectDeliversIntoSaysWhy(t *testing.T) {
	db, cleanup := taskDeleteDB(t)
	defer cleanup()
	for _, v := range []any{
		&domain.Organization{BaseModel: domain.BaseModel{ID: "org-1"}, Name: "Org", Slug: "org"},
		&domain.TaskSpace{BaseModel: domain.BaseModel{ID: "sp-1"}, OrgID: "org-1", Name: "Space"},
		&domain.TaskList{BaseModel: domain.BaseModel{ID: "l-1"}, SpaceID: "sp-1", Name: "inbox"},
		&domain.ReportProject{BaseModel: domain.BaseModel{ID: "proj-1"}, OrgID: "org-1", Name: "Acme", Slug: "acme",
			IngestKeyHash: []byte("h"), IsActive: true, ListID: func() *string { s := "l-1"; return &s }()},
	} {
		if err := db.Create(v).Error; err != nil {
			t.Fatal(err)
		}
	}
	h := &taskHandler{svc: service.NewTaskService(repository.NewTaskRepository(db), nil, nil, nil)}

	r := httptest.NewRequest(http.MethodDelete, "/api/v1/task-lists/l-1", nil)
	rctx := chi.NewRouteContext()
	rctx.URLParams.Add("id", "l-1")
	ctx := context.WithValue(r.Context(), chi.RouteCtxKey, rctx)
	ctx = context.WithValue(ctx, repository.UserContextKey, &domain.ClaimsJWT{UserID: "u-root", Superadmin: true})
	rec := httptest.NewRecorder()
	h.DeleteList(rec, r.WithContext(ctx))

	var body domain.APIResponse[any]
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if rec.Code != http.StatusConflict || body.Error != "list-in-use-by-channel" {
		t.Fatalf("%d %q (%s); quería 409 list-in-use-by-channel", rec.Code, body.Error, body.Message)
	}
	var still int64
	db.Model(&domain.TaskList{}).Where("id = ?", "l-1").Count(&still)
	if still != 1 {
		t.Fatal("un borrado rechazado no puede borrar")
	}
}
