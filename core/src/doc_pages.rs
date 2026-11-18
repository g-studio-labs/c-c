//! Las páginas de un documento: el árbol que cuelga de su portada.
//!
//! Un documento es una portada con cuatro pestañas fijas más las páginas que
//! haga falta, anidadas sin límite. Aquí se leen: el árbol, cuál está
//! abierta, de dónde cuelga y qué pasa cuando deja de existir. Crearlas y
//! escribirlas es de `doc_edit`; moverlas o tirarlas sigue en la app de
//! siempre.
//!
//! El orden del árbol es **el de la lista que manda el servidor**, que ya la
//! ordena por rango: aquí no se ordena nada (ver `doc-pages.store.ts`).

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::doc_edit::DocLeave;
use crate::docs::{kind_segment, DocSection, Docs, DocsState, DOC_TABS};
use crate::error::ApiError;
use crate::texts::{text, Locale};

/// Go escribe `null` donde una lista está sin crear. Aquí es una lista vacía.
pub(crate) fn or_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

fn is_zero(count: &u32) -> bool {
    *count == 0
}

/// Una fila del árbol, sin cuerpo. Espejo de `domain.DocPageTreeItem`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocPageItem {
    pub id: String,
    /// Sin madre cuelga de la portada.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub rank: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub has_body: bool,
    #[serde(default)]
    pub updated_at: String,
    /// Sólo en la papelera, que aquí todavía no se enseña.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub subpages: u32,
}

/// Un paso de las migas de pan. Espejo de `domain.DocPageCrumb`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocPageCrumb {
    pub id: String,
    #[serde(default)]
    pub title: String,
}

/// Algo que enlaza a una página. Espejo de `domain.DocBacklink`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocBacklink {
    /// `page`, `tab` o `task`.
    pub kind: String,
    #[serde(default)]
    pub title: String,
    /// El nodo de la página, la clave de la pestaña o la lista de la tarea.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default)]
    pub link: String,
}

/// La página en sí. Espejo de `domain.DocPage`, con lo que se lee.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocPage {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub title: String,
    /// Markdown.
    #[serde(default)]
    pub body: String,
    /// De qué versión es lo que se lee (ver `DocTab::body_hash`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by_name: Option<String>,
    /// Sus títulos, con el id que les da el servidor.
    #[serde(default, deserialize_with = "or_empty", skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<DocSection>,
}

/// Una página abierta: ella, de dónde cuelga, qué cuelga de ella y qué la
/// enlaza. Espejo de `domain.DocPageView`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocPageView {
    pub page: DocPage,
    /// De la raíz a la madre, sin la propia página.
    #[serde(default, deserialize_with = "or_empty")]
    pub breadcrumb: Vec<DocPageCrumb>,
    #[serde(default, deserialize_with = "or_empty")]
    pub children: Vec<DocPageItem>,
    #[serde(default, deserialize_with = "or_empty")]
    pub referenced_from: Vec<DocBacklink>,
}

/// Una fila del árbol de páginas, lista para pintar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocPageRow {
    pub id: String,
    /// Ya con «Sin título» si no tiene.
    pub title: String,
    pub depth: u32,
    pub has_children: bool,
    /// Si enseña lo que cuelga de ella.
    pub open: bool,
    /// Si es la que se lee.
    pub active: bool,
}

/// Una página a la que se puede ir: una miga o una hija.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocPageLink {
    pub id: String,
    pub title: String,
}

/// Algo que enlaza a la página abierta, dicho.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocBacklinkRow {
    /// Una tarea lleva otro icono que una página o una pestaña.
    pub from_task: bool,
    pub title: String,
    /// De dónde es, al lado y apagado. Una pestaña ya lo dice en su título.
    pub place: Option<String>,
    /// Si es otra página **de este mismo documento**, cuál.
    pub page_id: Option<String>,
    /// El enlace a lo que la nombra, para seguirlo (`Docs::follow_link`):
    /// una tarea, o la pestaña o la página de otro documento.
    pub link: String,
}

/// El título de una página, o «Sin título».
fn shown_title(locale: Locale, title: &str) -> String {
    if title.trim().is_empty() {
        text(locale, "work:docs.untitled")
    } else {
        title.to_string()
    }
}

impl DocsState {
    /// Si se lee una página y no la portada.
    pub fn on_page(&self) -> bool {
        self.page_id.is_some()
    }

    /// Las hijas de una página (o de la portada, con `None`), en el orden
    /// del servidor.
    fn children_of<'a>(&'a self, parent: Option<&'a str>) -> impl Iterator<Item = &'a DocPageItem> + 'a {
        self.pages.iter().filter(move |page| page.parent_id.as_deref() == parent)
    }

    /// Las madres de una página según el árbol, de la más cercana a la raíz.
    fn ancestors_of(&self, id: &str) -> Vec<&DocPageItem> {
        let mut chain: Vec<&DocPageItem> = Vec::new();
        let mut at = self.pages.iter().find(|page| page.id == id).and_then(|page| page.parent_id.as_deref());
        while let Some(parent) = at {
            // Un árbol que se muerde la cola no puede colgar a la app.
            if parent == id || chain.iter().any(|seen| seen.id == parent) {
                break;
            }
            let Some(page) = self.pages.iter().find(|page| page.id == parent) else { break };
            chain.push(page);
            at = page.parent_id.as_deref();
        }
        chain
    }

    /// El árbol de páginas tal como se pinta: cada fila con su hondura, y
    /// las hijas sólo de las ramas desplegadas. Va desplegado lo que lleva a
    /// la página que se lee, salvo lo que se haya plegado o desplegado a mano.
    pub fn page_rows(&self, locale: Locale) -> Vec<DocPageRow> {
        let open_by_default: HashSet<&str> =
            self.page_id.as_deref().map(|id| self.ancestors_of(id).into_iter().map(|page| page.id.as_str()).collect()).unwrap_or_default();
        let mut rows = Vec::new();
        // Una pila en vez de recursión: el árbol no tiene fondo.
        let mut pending: Vec<(&DocPageItem, u32)> = self.children_of(None).map(|page| (page, 0)).collect();
        pending.reverse();
        while let Some((page, depth)) = pending.pop() {
            let children: Vec<&DocPageItem> = self.children_of(Some(page.id.as_str())).collect();
            let open = !children.is_empty()
                && self.page_folds.get(&page.id).copied().unwrap_or_else(|| open_by_default.contains(page.id.as_str()));
            rows.push(DocPageRow {
                id: page.id.clone(),
                title: shown_title(locale, &page.title),
                depth,
                has_children: !children.is_empty(),
                open,
                active: self.page_id.as_deref() == Some(page.id.as_str()),
            });
            if open {
                pending.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
            }
        }
        rows
    }

    /// El título de la página que se lee. Mientras llega se dice el del
    /// árbol: la página ya tiene nombre antes de tener texto. De una que ni
    /// llegó ni está en el árbol no se sabe, y no se inventa un «Sin título».
    pub fn page_title(&self, locale: Locale) -> Option<String> {
        let id = self.page_id.as_deref()?;
        let title = match &self.page {
            Some(view) => view.page.title.as_str(),
            None => self.pages.iter().find(|page| page.id == id)?.title.as_str(),
        };
        Some(shown_title(locale, title))
    }

    /// Las migas de pan: de la raíz a la madre, sin la propia página. Las
    /// que manda el servidor con la página; mientras llega, las del árbol.
    pub fn page_trail(&self, locale: Locale) -> Vec<DocPageLink> {
        let Some(id) = self.page_id.as_deref() else { return Vec::new() };
        match &self.page {
            Some(view) => view.breadcrumb.iter().map(|crumb| DocPageLink { id: crumb.id.clone(), title: shown_title(locale, &crumb.title) }).collect(),
            None => self.ancestors_of(id).into_iter().rev().map(|page| DocPageLink { id: page.id.clone(), title: shown_title(locale, &page.title) }).collect(),
        }
    }

    /// Las páginas que cuelgan de la que se lee, para su pie.
    pub fn page_children(&self, locale: Locale) -> Vec<DocPageLink> {
        let Some(view) = &self.page else { return Vec::new() };
        view.children.iter().map(|child| DocPageLink { id: child.id.clone(), title: shown_title(locale, &child.title) }).collect()
    }

    /// Lo que enlaza a la página que se lee (`DocPageView.tsx`): una pestaña
    /// se dice «Nodo · Runbook»; una página o una tarea, con su título y de
    /// dónde es al lado.
    pub fn page_backlinks(&self, locale: Locale) -> Vec<DocBacklinkRow> {
        let Some(view) = &self.page else { return Vec::new() };
        view.referenced_from
            .iter()
            .map(|link| {
                let place = link.place.clone().filter(|place| !place.is_empty());
                if link.kind == "tab" {
                    let tab = place.filter(|key| DOC_TABS.contains(&key.as_str())).map(|key| text(locale, &format!("work:docs.{key}")));
                    let title = match tab {
                        Some(tab) => format!("{} · {tab}", link.title),
                        None => link.title.clone(),
                    };
                    return DocBacklinkRow { from_task: false, title, place: None, page_id: None, link: link.link.clone() };
                }
                DocBacklinkRow { from_task: link.kind == "task", title: shown_title(locale, &link.title), place, page_id: self.page_for_link(&link.link), link: link.link.clone() }
            })
            .collect()
    }
}

impl Docs {
    fn pages_path(&self) -> Option<(String, String)> {
        let target = self.state().target?;
        Some((format!("/api/v1/docs/{}/{}/pages", kind_segment(target.kind), target.id), target.id))
    }

    fn still_on(&self, node: &str) -> bool {
        self.state().target.as_ref().map(|target| target.id.as_str()) == Some(node)
    }

    /// Trae el árbol de páginas del documento abierto.
    ///
    /// Si la página que se leía ya no está en él —alguien la tiró—, se
    /// vuelve a la portada: seguir enseñándola sería leer algo que ya no
    /// existe. Un fallo deja el árbol que había y lo dice.
    pub async fn reload_pages(&self) {
        let Some((path, node)) = self.pages_path() else { return };
        let answer = self.core.get_list::<DocPageItem>(&path).await;
        // Si mientras tanto se abrió otro nodo, este árbol ya no es de nadie.
        if !self.still_on(&node) {
            return;
        }
        match answer {
            Ok(pages) => self.update(|state| {
                // Una que no se pudo abrir sigue diciendo que no se pudo.
                let gone = state.page_error.is_none() && state.page_id.as_ref().is_some_and(|open| !pages.iter().any(|page| &page.id == open));
                if gone {
                    state.page_id = None;
                    state.page = None;
                    state.page_loading = false;
                }
                state.pages = pages;
                state.pages_error = None;
            }),
            Err(error) => self.update(|state| state.pages_error = Some(error.to_string())),
        }
    }

    /// Abre la hoja «Páginas» y vuelve a pedir el árbol: una página que un
    /// agente acaba de crear tiene que estar al mirar.
    pub async fn show_pages(&self) {
        if self.state().target.is_none() {
            return;
        }
        self.update(|state| state.pages_shown = true);
        self.reload_pages().await;
    }

    pub fn hide_pages(&self) {
        if self.state().pages_shown {
            self.update(|state| state.pages_shown = false);
        }
    }

    /// Abre una página del documento abierto y la trae. Elegirla cierra la
    /// hoja. Sin documento abierto no hace nada. Con texto sin guardar,
    /// pregunta antes.
    pub async fn open_page(&self, page_id: &str) {
        if self.state().target.is_none() || page_id.is_empty() {
            return;
        }
        if self.held_back(DocLeave::Page { id: page_id.to_string() }) {
            return;
        }
        self.show_page(page_id).await;
    }

    /// Lo mismo, sin preguntar: para la página que se acaba de crear.
    pub(crate) async fn show_page(&self, page_id: &str) {
        self.update(|state| {
            // La de antes no se enseña mientras llega ésta: se leería bajo
            // un título que no es el suyo.
            if state.page_id.as_deref() != Some(page_id) {
                state.page = None;
            }
            state.page_id = Some(page_id.to_string());
            state.page_loading = true;
            state.page_error = None;
            state.pages_shown = false;
            state.jump = None;
        });
        self.load_page(page_id).await;
    }

    /// Pide la página y la deja en el estado, si sigue siendo la que se mira.
    pub(crate) async fn load_page(&self, page_id: &str) {
        let Some((path, _)) = self.pages_path() else { return };
        let answer = self.core.get::<DocPageView>(&format!("{path}/{page_id}")).await;
        // Se cambió de página a medio camino. O de nodo: abrir otro deja
        // sin página abierta, así que esto lo cubre también.
        if self.state().page_id.as_deref() != Some(page_id) {
            return;
        }
        match answer {
            Ok(view) => self.update(|state| {
                state.page = Some(view);
                state.page_loading = false;
                state.page_error = None;
            }),
            Err(error) => {
                let missing = matches!(error, ApiError::Server { status: 404, .. });
                self.update(|state| {
                    state.page_loading = false;
                    // Si ya se estaba leyendo, esto era ponerla al día: lo
                    // leído vale más que un «no se pudo» encima.
                    if state.page.is_none() {
                        state.page_error = Some(error.to_string());
                    }
                });
                // Ya no existe: el árbol que la ofrecía está viejo.
                if missing {
                    self.reload_pages().await;
                }
            }
        }
    }

    /// De vuelta a la portada. También cierra la hoja: es elegir algo en
    /// ella. Con texto sin guardar en una página, pregunta antes.
    pub fn close_page(&self) {
        let state = self.state();
        if state.page_id.is_some() && self.held_back(DocLeave::Home) {
            return;
        }
        if state.page_id.is_some() || state.pages_shown {
            self.update(|state| {
                state.page_id = None;
                state.page = None;
                state.page_loading = false;
                state.page_error = None;
                state.pages_shown = false;
                state.jump = None;
            });
        }
    }

    /// Pliega o despliega una página del árbol. Lo que se hace a mano manda
    /// sobre lo que va abierto por llevar a la página que se lee.
    pub fn toggle_page(&self, page_id: &str) {
        let Some(row) = self.state().page_rows(Locale::En).into_iter().find(|row| row.id == page_id) else { return };
        self.update(|state| {
            state.page_folds.insert(page_id.to_string(), !row.open);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, parent: Option<&str>) -> DocPageItem {
        DocPageItem { id: id.into(), parent_id: parent.map(str::to_string), title: id.into(), ..DocPageItem::default() }
    }

    /// Un árbol que se muerde la cola (dos páginas, cada una madre de la
    /// otra) no cuelga a quien lo pinta.
    ///
    /// Rotura que atrapa: buscar las madres de la página abierta sin fin, y
    /// con ello la app entera parada al abrir la hoja.
    #[test]
    fn a_tree_that_bites_its_tail_does_not_hang() {
        let state = DocsState {
            pages: vec![item("root", None), item("a", Some("b")), item("b", Some("a"))],
            page_id: Some("a".into()),
            ..DocsState::default()
        };
        let rows: Vec<String> = state.page_rows(Locale::Es).into_iter().map(|row| row.id).collect();
        assert_eq!(rows, ["root"]);
        let trail: Vec<String> = state.page_trail(Locale::Es).into_iter().map(|crumb| crumb.id).collect();
        assert_eq!(trail, ["b"]);
    }
}
