package service

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/google/uuid"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"github.com/guz-studio/cac/backend/internal/core/domain"
	"github.com/guz-studio/cac/backend/internal/core/repository"
)

const shareOrg = "org-share-1"

type shareFixture struct {
	db    *gorm.DB
	docs  *repository.DocRepository
	svc   *DocShareService
	list  *domain.TaskList
	other *domain.TaskList
	clock time.Time
}

func newShareFixture(t *testing.T) *shareFixture {
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
	name := fmt.Sprintf("cac_test_doc_share_%d", time.Now().UnixNano()%1_000_000)
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
	if err := db.AutoMigrate(
		&domain.TaskSpace{}, &domain.TaskFolder{}, &domain.TaskList{}, &domain.User{},
		&domain.Doc{}, &domain.DocTab{}, &domain.DocVersion{}, &domain.DocAttachment{},
		&domain.DocPage{}, &domain.DocPageVersion{}, &domain.DocSection{}, &domain.Item{},
		&domain.DocShare{}, &domain.GraphLink{},
	); err != nil {
		t.Fatal(err)
	}
	if err := repository.EnsureSearchIndexes(db); err != nil {
		t.Fatal(err)
	}
	sp := &domain.TaskSpace{OrgID: shareOrg, Name: "Proteus"}
	if err := db.Create(sp).Error; err != nil {
		t.Fatal(err)
	}
	l := &domain.TaskList{SpaceID: sp.ID, Name: "Apps"}
	o := &domain.TaskList{SpaceID: sp.ID, Name: "Infra"}
	if err := db.Create(l).Error; err != nil {
		t.Fatal(err)
	}
	if err := db.Create(o).Error; err != nil {
		t.Fatal(err)
	}
	docs := repository.NewDocRepository(db)
	f := &shareFixture{db: db, docs: docs, list: l, other: o,
		clock: time.Date(2026, 11, 24, 12, 0, 0, 0, time.UTC)}
	f.svc = NewDocShareService(repository.NewDocShareRepository(db), docs)
	f.svc.now = func() time.Time { return f.clock }
	return f
}

func (f *shareFixture) page(t *testing.T, list *domain.TaskList, title, body string, parent *string) *domain.DocPage {
	t.Helper()
	p, err := f.docs.CreatePage(shareOrg, domain.DocOwnerList, list.ID,
		domain.CreateDocPageRequest{Title: title, Body: body, ParentID: parent}, "u-author")
	if err != nil {
		t.Fatal(err)
	}
	return p
}

func (f *shareFixture) share(t *testing.T, p *domain.DocPage) *domain.DocShareResponse {
	t.Helper()
	doc, err := f.docs.FindByID(p.DocID)
	if err != nil {
		t.Fatal(err)
	}
	sh, err := f.svc.Create(doc, p.ID, "u-host", 168)
	if err != nil {
		t.Fatal(err)
	}
	return sh
}

func (f *shareFixture) attachment(t *testing.T, docID string) *domain.DocAttachment {
	t.Helper()
	a := &domain.DocAttachment{DocID: docID, Path: "docs/" + docID + "/x.png", FileName: "x.png", ContentType: "image/png"}
	a.ID = uuid.NewString()
	a.URL = domain.DocAttachmentRef(docID, a.ID)
	if err := f.db.Create(a).Error; err != nil {
		t.Fatal(err)
	}
	return a
}

func sharedTitles(d *domain.PublicSharedDoc) string {
	var out []string
	for _, p := range d.Pages {
		out = append(out, p.Title)
	}
	return strings.Join(out, ",")
}

// El enlace abre la página y lo que cuelga de ella **tal y como está ahora**:
// lo colgado después de compartir sale, lo tirado a la papelera (y lo que cuelga
// de ello) no, y nada de fuera del subárbol. Las hermanas van por rango, no por
// fecha de creación, y cada hija detrás de su madre.
func TestASharedPageShowsItsLiveSubtreeInRankOrder(t *testing.T) {
	f := newShareFixture(t)
	parent := f.page(t, f.list, "Parent", "", nil)
	root := f.page(t, f.list, "Root", "", &parent.ID)
	f.page(t, f.list, "Outside", "", nil)
	a := f.page(t, f.list, "A", "", &root.ID)
	f.page(t, f.list, "A1", "", &a.ID)
	b := f.page(t, f.list, "B", "", &root.ID)
	f.page(t, f.list, "B1", "", &b.ID)

	sh := f.share(t, root)

	// Después de compartir: una hija nueva, que además se pone delante de A.
	c := f.page(t, f.list, "C", "", &root.ID)
	if err := f.docs.MovePage(root.DocID, c.ID, domain.MoveDocPageRequest{ParentID: &root.ID, BeforeID: &a.ID}); err != nil {
		t.Fatal(err)
	}
	if _, err := f.docs.TrashPage(root.DocID, b.ID); err != nil {
		t.Fatal(err)
	}

	got, err := f.svc.Public(sh.Link)
	if err != nil {
		t.Fatal(err)
	}
	if sharedTitles(got) != "Root,C,A,A1" {
		t.Fatalf("the shared tree is %q, want Root,C,A,A1", sharedTitles(got))
	}
	if got.Title != "Root" {
		t.Errorf("title %q", got.Title)
	}
	if got.Pages[0].ParentID != nil {
		t.Errorf("the root goes with parentId %v, want null", *got.Pages[0].ParentID)
	}
	if got.Pages[2].ParentID == nil || *got.Pages[2].ParentID != root.ID {
		t.Errorf("A does not hang from the root: %v", got.Pages[2].ParentID)
	}
	if !got.ExpiresAt.Equal(f.clock.Add(168 * time.Hour)) {
		t.Errorf("expiresAt %v", got.ExpiresAt)
	}
}

// Caducado, revocado, firma mala o página raíz en la papelera: el mismo error,
// sin decir cuál.
func TestASharedLinkStopsOpeningWhenExpiredRevokedOrDeleted(t *testing.T) {
	f := newShareFixture(t)
	root := f.page(t, f.list, "Root", "", nil)
	sh := f.share(t, root)

	if _, err := f.svc.Public(sh.Link); err != nil {
		t.Fatalf("a fresh link does not open: %v", err)
	}
	if _, err := f.svc.Public(sh.ID + ".deadbeef"); !errors.Is(err, ErrDocShareInvalid) {
		t.Errorf("a forged signature: %v", err)
	}

	f.clock = f.clock.Add(168*time.Hour + time.Second)
	if _, err := f.svc.Public(sh.Link); !errors.Is(err, ErrDocShareInvalid) {
		t.Errorf("an expired link: %v", err)
	}
	f.clock = f.clock.Add(-2 * time.Hour)
	if _, err := f.svc.Public(sh.Link); err != nil {
		t.Fatalf("still within its time and does not open: %v", err)
	}

	if err := f.svc.Revoke(sh.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := f.svc.Public(sh.Link); !errors.Is(err, ErrDocShareInvalid) {
		t.Errorf("a revoked link: %v", err)
	}
	doc, _ := f.docs.FindByID(root.DocID)
	if live, _ := f.svc.List(doc, root.ID); len(live) != 0 {
		t.Errorf("a revoked link is still listed: %+v", live)
	}

	other := f.share(t, root)
	if _, err := f.docs.TrashPage(root.DocID, root.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := f.svc.Public(other.Link); !errors.Is(err, ErrDocShareInvalid) {
		t.Errorf("a link to a deleted page: %v", err)
	}
}

// Un adjunto se abre sólo si es del documento y lo cita una página viva del
// árbol compartido.
func TestASharedAttachmentMustBeCitedInsideTheTree(t *testing.T) {
	f := newShareFixture(t)
	root := f.page(t, f.list, "Root", "", nil)
	outside := f.page(t, f.list, "Outside", "", nil)
	foreignPage := f.page(t, f.other, "Foreign", "", nil)

	cited := f.attachment(t, root.DocID)
	onlyOutside := f.attachment(t, root.DocID)
	inTrash := f.attachment(t, root.DocID)
	foreign := f.attachment(t, foreignPage.DocID)

	child := f.page(t, f.list, "Child", "![shot]("+cited.URL+")\n\n![other]("+foreign.URL+")", &root.ID)
	_ = child
	gone := f.page(t, f.list, "Gone", "![x]("+inTrash.URL+")", &root.ID)
	body := "![y](" + onlyOutside.URL + ")"
	if err := f.docs.SavePage(outside, nil, &body, "u-author"); err != nil {
		t.Fatal(err)
	}
	if _, err := f.docs.TrashPage(root.DocID, gone.ID); err != nil {
		t.Fatal(err)
	}
	sh := f.share(t, root)

	if att, err := f.svc.PublicAttachment(sh.Link, cited.ID); err != nil || att.ID != cited.ID {
		t.Fatalf("a cited attachment does not open: %v", err)
	}
	for name, id := range map[string]string{
		"cited only outside the tree":     onlyOutside.ID,
		"cited only in a trashed page":    inTrash.ID,
		"from another doc, cited in tree": foreign.ID,
		"that does not exist":             uuid.NewString(),
	} {
		if _, err := f.svc.PublicAttachment(sh.Link, id); !errors.Is(err, ErrDocShareInvalid) {
			t.Errorf("an attachment %s: %v", name, err)
		}
	}
	if _, err := f.svc.PublicAttachment(sh.ID+".00", cited.ID); !errors.Is(err, ErrDocShareInvalid) {
		t.Errorf("a cited attachment with a forged link: %v", err)
	}
}

// Lo que ve alguien de fuera no lleva ni la organización, ni el documento, ni
// quién escribió o compartió.
func TestThePublicSharedDocCarriesNoOrgDocOrUserIds(t *testing.T) {
	f := newShareFixture(t)
	root := f.page(t, f.list, "Root", "hello", nil)
	f.page(t, f.list, "Child", "world", &root.ID)
	sh := f.share(t, root)

	got, err := f.svc.Public(sh.Link)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(got)
	for name, id := range map[string]string{
		"org": shareOrg, "doc": root.DocID, "author": "u-author", "sharer": "u-host",
		"list": f.list.ID,
	} {
		if strings.Contains(string(raw), id) {
			t.Errorf("the public response carries the %s id: %s", name, raw)
		}
	}
	var keys []map[string]any
	var shape struct {
		Pages json.RawMessage `json:"pages"`
	}
	_ = json.Unmarshal(raw, &shape)
	_ = json.Unmarshal(shape.Pages, &keys)
	for _, k := range keys {
		if len(k) != 4 {
			t.Errorf("a public page has fields beyond id, parentId, title and body: %v", k)
		}
	}
}

// 1, 7 o 30 días, y nada más.
func TestASharedLinkLastsOneSevenOrThirtyDays(t *testing.T) {
	f := newShareFixture(t)
	root := f.page(t, f.list, "Root", "", nil)
	doc, _ := f.docs.FindByID(root.DocID)
	for _, h := range []int{0, 1, 48, 721, -24} {
		if _, err := f.svc.Create(doc, root.ID, "u-host", h); !errors.Is(err, ErrDocShareTTL) {
			t.Errorf("%d hours: %v", h, err)
		}
	}
	for _, h := range []int{24, 168, 720} {
		sh, err := f.svc.Create(doc, root.ID, "u-host", h)
		if err != nil {
			t.Fatalf("%d hours: %v", h, err)
		}
		if !sh.ExpiresAt.Equal(f.clock.Add(time.Duration(h) * time.Hour)) {
			t.Errorf("%d hours expires at %v", h, sh.ExpiresAt)
		}
	}
}
