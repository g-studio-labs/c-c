package repository

import (
	"testing"
	"time"

	"gorm.io/gorm"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

func deleteFixture(t *testing.T, db *gorm.DB) {
	t.Helper()
	for _, v := range []any{
		&domain.Organization{BaseModel: domain.BaseModel{ID: "org-1"}, Name: "Org", Slug: "org"},
		&domain.TaskSpace{BaseModel: domain.BaseModel{ID: "sp-1"}, OrgID: "org-1", Name: "Space"},
		// root > middle > leaf, and a list in each of middle and leaf.
		&domain.TaskFolder{BaseModel: domain.BaseModel{ID: "root"}, SpaceID: "sp-1", Name: "root"},
		&domain.TaskFolder{BaseModel: domain.BaseModel{ID: "middle"}, SpaceID: "sp-1", Name: "middle", ParentFolderID: strp("root")},
		&domain.TaskFolder{BaseModel: domain.BaseModel{ID: "leaf"}, SpaceID: "sp-1", Name: "leaf", ParentFolderID: strp("middle")},
		&domain.TaskList{BaseModel: domain.BaseModel{ID: "l-middle"}, SpaceID: "sp-1", FolderID: strp("middle"), Name: "l-middle"},
		&domain.TaskList{BaseModel: domain.BaseModel{ID: "l-leaf"}, SpaceID: "sp-1", FolderID: strp("leaf"), Name: "l-leaf"},
	} {
		if err := db.Create(v).Error; err != nil {
			t.Fatal(err)
		}
	}
}

func strp(s string) *string { return &s }

// Borrar una carpeta con subcarpetas las sube un nivel, con sus listas: ni
// desaparecen del árbol colgando de una madre que ya no existe ni se pierden
// (command-and-control-384).
//
// Roturas que atrapa: dejar las subcarpetas apuntando a la borrada; subirlas
// al espacio en vez de a la madre de la borrada; y subir las listas a la raíz
// del espacio cuando la carpeta borrada tenía madre.
func TestDeletingAFolderMovesItsSubfoldersAndListsUpOneLevel(t *testing.T) {
	db, cleanup := itemMigrationDB(t)
	defer cleanup()
	deleteFixture(t, db)
	repo := NewTaskRepository(db)

	if err := repo.DeleteFolder("middle"); err != nil {
		t.Fatal(err)
	}

	var leaf domain.TaskFolder
	if err := db.First(&leaf, "id = ?", "leaf").Error; err != nil {
		t.Fatal(err)
	}
	if leaf.ParentFolderID == nil || *leaf.ParentFolderID != "root" {
		t.Fatalf("la subcarpeta debía pasar a colgar de la madre de la borrada, quedó en %v", leaf.ParentFolderID)
	}
	var list domain.TaskList
	if err := db.First(&list, "id = ?", "l-middle").Error; err != nil {
		t.Fatal(err)
	}
	if list.FolderID == nil || *list.FolderID != "root" {
		t.Fatalf("la lista debía subir a la madre de la borrada, quedó en %v", list.FolderID)
	}

	// Una carpeta de primer nivel sube al espacio.
	if err := repo.DeleteFolder("root"); err != nil {
		t.Fatal(err)
	}
	if err := db.First(&leaf, "id = ?", "leaf").Error; err != nil {
		t.Fatal(err)
	}
	if err := db.First(&list, "id = ?", "l-middle").Error; err != nil {
		t.Fatal(err)
	}
	// l-middle ya había subido a root; al borrar root sube al espacio.
	if leaf.ParentFolderID != nil || list.FolderID != nil {
		t.Fatalf("al borrar una de primer nivel todo debía quedar en el espacio: %v %v", leaf.ParentFolderID, list.FolderID)
	}
}

// Borrar una tarea se lleva sus subtareas, también las de las subtareas, y no
// toca las de otra (command-and-control-386).
//
// Roturas que atrapa: dejar las subtareas vivas apuntando a una madre borrada;
// llevarse sólo el primer nivel; borrar de más (una tarea hermana o sus
// subtareas); y dejar vivos los asignados, etiquetas, comentarios o adjuntos
// de las subtareas (borrarlos sólo por el id de la madre).
func TestDeletingATaskTakesItsSubtasksWithIt(t *testing.T) {
	db, cleanup := itemMigrationDB(t)
	defer cleanup()
	deleteFixture(t, db)
	for _, it := range []domain.Item{
		{BaseModel: domain.BaseModel{ID: "parent"}, OrgID: "org-1", ListID: "l-middle", Title: "parent", Status: domain.ReportPending},
		{BaseModel: domain.BaseModel{ID: "sub"}, OrgID: "org-1", ListID: "l-middle", Title: "sub", Status: domain.ReportPending, ParentID: strp("parent")},
		{BaseModel: domain.BaseModel{ID: "subsub"}, OrgID: "org-1", ListID: "l-middle", Title: "subsub", Status: domain.ReportPending, ParentID: strp("sub")},
		{BaseModel: domain.BaseModel{ID: "other"}, OrgID: "org-1", ListID: "l-middle", Title: "other", Status: domain.ReportPending},
		{BaseModel: domain.BaseModel{ID: "other-sub"}, OrgID: "org-1", ListID: "l-middle", Title: "other-sub", Status: domain.ReportPending, ParentID: strp("other")},
	} {
		it := it
		if err := db.Create(&it).Error; err != nil {
			t.Fatal(err)
		}
	}
	// Cada subtarea con todo lo que puede colgarle, y más de un asignado: un
	// borrado que sólo mirara el id de la madre dejaría esto vivo.
	tag := domain.TaskTag{BaseModel: domain.BaseModel{ID: "tag-1"}, OrgID: "org-1", Name: "t", Color: "red"}
	if err := db.Create(&tag).Error; err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"sub", "subsub"} {
		for _, row := range []any{
			&domain.ItemAssignee{ItemID: id, UserID: "u-1"},
			&domain.ItemAssignee{ItemID: id, UserID: "u-2"},
			&domain.TaskTagLink{TaskID: id, TagID: "tag-1"},
			&domain.ItemComment{BaseModel: domain.BaseModel{ID: "c-" + id}, ItemID: id, Body: "hola"},
			&domain.ItemAttachment{BaseModel: domain.BaseModel{ID: "a-" + id}, ItemID: id, FileName: "f.txt"},
		} {
			if err := db.Create(row).Error; err != nil {
				t.Fatal(err)
			}
		}
	}

	gone, err := NewTaskRepository(db).DeleteTask("parent")
	if err != nil {
		t.Fatal(err)
	}
	if len(gone) != 3 || gone[0] != "parent" {
		t.Fatalf("debía devolver parent y sus dos subtareas, devolvió %v", gone)
	}

	var alive []string
	db.Model(&domain.Item{}).Order("id").Pluck("id", &alive)
	if len(alive) != 2 || alive[0] != "other" || alive[1] != "other-sub" {
		t.Fatalf("debían quedar sólo other y other-sub, quedan %v", alive)
	}
	for name, model := range map[string]any{
		"asignados":   &domain.ItemAssignee{},
		"etiquetas":   &domain.TaskTagLink{},
		"comentarios": &domain.ItemComment{},
		"adjuntos":    &domain.ItemAttachment{},
	} {
		var left int64
		col := "item_id"
		if name == "etiquetas" {
			col = "task_id"
		}
		db.Model(model).Where(col+" IN ?", []string{"sub", "subsub"}).Count(&left)
		if left != 0 {
			t.Errorf("los %s de una subtarea borrada se quedan colgando (%d)", name, left)
		}
	}
}

// Un ciclo en `parent_id` (dos tareas que se tienen por madre) no cuelga el
// borrado: se llevan a las dos y termina.
//
// Roturas que atrapa: quitar la memoria de lo ya visto, con lo que el recorrido
// de subtareas no acaba nunca.
func TestDeletingATaskInAParentCycleTerminates(t *testing.T) {
	db, cleanup := itemMigrationDB(t)
	defer cleanup()
	deleteFixture(t, db)
	for _, it := range []domain.Item{
		{BaseModel: domain.BaseModel{ID: "a"}, OrgID: "org-1", ListID: "l-middle", Title: "a", Status: domain.ReportPending, ParentID: strp("b")},
		{BaseModel: domain.BaseModel{ID: "b"}, OrgID: "org-1", ListID: "l-middle", Title: "b", Status: domain.ReportPending, ParentID: strp("a")},
	} {
		it := it
		if err := db.Create(&it).Error; err != nil {
			t.Fatal(err)
		}
	}
	done := make(chan error, 1)
	go func() { _, err := NewTaskRepository(db).DeleteTask("a"); done <- err }()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(20 * time.Second):
		t.Fatal("el borrado no termina con un ciclo en parent_id")
	}
	var left int64
	db.Model(&domain.Item{}).Count(&left)
	if left != 0 {
		t.Fatalf("quedan %d filas del ciclo", left)
	}
}
