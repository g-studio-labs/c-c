package service

import (
	"strings"
	"testing"
	"time"

	"github.com/guz-studio/cac/backend/internal/core/domain"
)

func orgShareIDs(rows []domain.OrgDocShareResponse) string {
	var out []string
	for _, r := range rows {
		out = append(out, r.PageTitle)
	}
	return strings.Join(out, ",")
}

// La lista de la organización enseña sólo lo que todavía abre algo y sólo de
// esa organización: ni el revocado, ni el caducado, ni el de una página en la
// papelera, ni el de otra org. Los que caducan antes, primero.
func TestTheOrgListShowsOnlyLiveLinksOfThatOrg(t *testing.T) {
	f := newShareFixture(t)
	docOf := func(p *domain.DocPage) *domain.Doc {
		d, err := f.docs.FindByID(p.DocID)
		if err != nil {
			t.Fatal(err)
		}
		return d
	}
	open := func(p *domain.DocPage, hours int) *domain.DocShareResponse {
		sh, err := f.svc.Create(docOf(p), p.ID, "u-host", hours)
		if err != nil {
			t.Fatal(err)
		}
		return sh
	}

	month := f.page(t, f.list, "Month", "", nil)
	open(month, 720)
	week := f.page(t, f.list, "Week", "", nil)
	open(week, 168)

	revoked := f.page(t, f.list, "Revoked", "", nil)
	if err := f.svc.Revoke(open(revoked, 720).ID); err != nil {
		t.Fatal(err)
	}
	expired := f.page(t, f.list, "Expired", "", nil)
	open(expired, 24)
	trashed := f.page(t, f.list, "Trashed", "", nil)
	open(trashed, 720)
	if _, err := f.docs.TrashPage(trashed.DocID, trashed.ID); err != nil {
		t.Fatal(err)
	}

	sp := &domain.TaskSpace{OrgID: "org-share-2", Name: "Other"}
	f.db.Create(sp)
	ol := &domain.TaskList{SpaceID: sp.ID, Name: "Theirs"}
	f.db.Create(ol)
	foreign, err := f.docs.CreatePage("org-share-2", domain.DocOwnerList, ol.ID,
		domain.CreateDocPageRequest{Title: "Foreign"}, "u-other")
	if err != nil {
		t.Fatal(err)
	}
	open(foreign, 720)

	// Un día y un segundo después: el de 24 h ya no abre.
	f.clock = f.clock.Add(24*time.Hour + time.Second)

	got, err := f.svc.ListForOrg(shareOrg, "")
	if err != nil {
		t.Fatal(err)
	}
	if orgShareIDs(got) != "Week,Month" {
		t.Fatalf("the org list is %q, want Week,Month", orgShareIDs(got))
	}
	w := got[0]
	if w.Link == "" || w.PageID != week.ID || w.DocKind != domain.DocOwnerList || w.OwnerID != f.list.ID ||
		w.OwnerName != "Apps" || w.SpaceName != "Proteus" || w.CreatedBy != "u-host" ||
		w.Mode != domain.DocShareModeRead || w.ResponseCount != 0 {
		t.Errorf("the row carries %+v", w)
	}
}

// El espacio sale del nodo del documento, sea el espacio mismo, una carpeta
// (también dentro de otra carpeta) o una lista dentro de una carpeta; y el
// filtro por espacio deja fuera los de otro espacio de la misma org.
func TestTheOrgListFiltersBySpaceThroughFoldersAndLists(t *testing.T) {
	f := newShareFixture(t)
	spaceID := f.list.SpaceID
	outer := &domain.TaskFolder{SpaceID: spaceID, Name: "Clients"}
	f.db.Create(outer)
	inner := &domain.TaskFolder{SpaceID: spaceID, Name: "Acme", ParentFolderID: &outer.ID}
	f.db.Create(inner)
	nested := &domain.TaskList{SpaceID: spaceID, Name: "Acme web", FolderID: &inner.ID}
	f.db.Create(nested)
	second := &domain.TaskSpace{OrgID: shareOrg, Name: "Galena"}
	f.db.Create(second)

	mk := func(kind domain.DocOwnerKind, owner, title string) {
		p, err := f.docs.CreatePage(shareOrg, kind, owner, domain.CreateDocPageRequest{Title: title}, "u-author")
		if err != nil {
			t.Fatal(err)
		}
		f.share(t, p)
	}
	mk(domain.DocOwnerSpace, spaceID, "OnSpace")
	mk(domain.DocOwnerFolder, inner.ID, "OnFolder")
	mk(domain.DocOwnerList, nested.ID, "OnList")
	mk(domain.DocOwnerSpace, second.ID, "OnGalena")

	got, err := f.svc.ListForOrg(shareOrg, spaceID)
	if err != nil {
		t.Fatal(err)
	}
	names := map[string]domain.OrgDocShareResponse{}
	for _, r := range got {
		names[r.PageTitle] = r
	}
	for title, owner := range map[string]string{"OnSpace": "Proteus", "OnFolder": "Acme", "OnList": "Acme web"} {
		r, ok := names[title]
		if !ok {
			t.Errorf("%s is missing from its space: %q", title, orgShareIDs(got))
			continue
		}
		if r.SpaceID != spaceID || r.SpaceName != "Proteus" || r.OwnerName != owner {
			t.Errorf("%s: space %q %q, owner %q", title, r.SpaceID, r.SpaceName, r.OwnerName)
		}
	}
	if _, ok := names["OnGalena"]; ok || len(got) != 3 {
		t.Errorf("the space filter lets in %q", orgShareIDs(got))
	}

	all, err := f.svc.ListForOrg(shareOrg, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(all) != 4 {
		t.Errorf("without a filter: %q", orgShareIDs(all))
	}
}
