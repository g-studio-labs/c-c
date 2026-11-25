package repository

import (
	"testing"

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

