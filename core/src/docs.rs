//! La documentación de un nodo, para leerla.
//!
//! Cuelga de un nodo del árbol (espacio, carpeta o lista) y tiene cuatro
//! pestañas fijas. Aquí sólo se lee: escribirla sigue en la app de siempre.
//!
//! Qué documento hay abierto es también lo que contesta «¿qué está mirando?»
//! a la regla del clic (`nav::tree_click`): con uno abierto se mira
//! documentación, y un clic en el árbol abre la de ese nodo.

use serde::{Deserialize, Serialize};

use crate::mywork::day_and_month;
use crate::nav::{docs_reachable_from_scope, Looking, NodeKind, TreeClick};
use crate::texts::Locale;
use crate::tree_rows::find_node;
use crate::Core;

/// Las cuatro pestañas, en el orden en que se pintan. Ninguna se esconde por
/// estar vacía: que un proyecto no tenga runbook es un dato sobre el proyecto.
pub const DOC_TABS: [&str; 4] = ["overview", "runbook", "decisions", "links"];

/// Una pestaña del documento. Espejo de `domain.DocTab`, con lo que se lee.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocTab {
    pub key: String,
    /// Markdown.
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by_name: Option<String>,
}

/// Una entrada del registro de decisiones. Espejo de `domain.Decision`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DocDecision {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    pub decided_at: String,
    /// El título de la tarea de la que salió.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_title: Option<String>,
    /// La decisión que dejó a ésta sin efecto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_id: Option<String>,
}

impl DocDecision {
    /// Cuándo se decidió, dicho: «30 sept 2026». Es un instante, así que el
    /// día es el de quien lo lee.
    pub fn decided_on(&self, locale: Locale) -> String {
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&self.decided_at) else { return String::new() };
        let day = at.with_timezone(&chrono::Local).date_naive();
        format!("{} {}", day_and_month(locale, day), chrono::Datelike::year(&day))
    }
}

/// El sobre con que el servidor contesta `GET /api/v1/docs/{kind}/{id}`. Un
/// nodo sin documento contesta igual, con las listas vacías.
#[derive(Debug, Default, Deserialize)]
struct DocAnswer {
    #[serde(default)]
    tabs: Vec<DocTab>,
    #[serde(default)]
    decisions: Vec<DocDecision>,
}

/// De qué nodo es el documento abierto.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocTarget {
    pub kind: NodeKind,
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DocsState {
    /// `None` es «no se mira documentación».
    pub target: Option<DocTarget>,
    /// La pestaña que se lee, por su clave (`DOC_TABS`).
    pub tab: String,
    pub loading: bool,
    pub error: Option<String>,
    pub tabs: Vec<DocTab>,
    pub decisions: Vec<DocDecision>,
}

impl Default for DocsState {
    fn default() -> Self {
        Self { target: None, tab: DOC_TABS[0].to_string(), loading: false, error: None, tabs: Vec::new(), decisions: Vec::new() }
    }
}

impl DocsState {
    /// El texto de la pestaña que se lee. Vacío si no tiene nada escrito, o
    /// si el nodo todavía no tiene documento.
    pub fn body(&self) -> &str {
        self.tabs.iter().find(|tab| tab.key == self.tab).map(|tab| tab.body.trim()).unwrap_or("")
    }

    /// Las decisiones que siguen vigentes, y sólo en su pestaña. Una que otra
    /// dejó sin efecto no se enseña: leerla como vigente es peor que no verla.
    pub fn standing_decisions(&self) -> Vec<DocDecision> {
        if self.tab != "decisions" {
            return Vec::new();
        }
        self.decisions.iter().filter(|decision| decision.superseded_by_id.is_none()).cloned().collect()
    }

    /// Por qué no hay nada que leer, como clave del catálogo; `None` si lo
    /// hay. No es lo mismo «no ha llegado», «se rompió» y «no hay nada
    /// escrito», y una página en blanco no distingue ninguna.
    pub fn empty_reason(&self) -> Option<&'static str> {
        if self.loading && self.tabs.is_empty() {
            Some("work:tree.loading")
        } else if self.error.is_some() {
            Some("common:last.requestsFailing")
        } else if self.body().is_empty() && self.standing_decisions().is_empty() {
            Some(if self.tab == "decisions" { "work:decisions.empty" } else { "work:docs.emptyTab" })
        } else {
            None
        }
    }
}

fn kind_segment(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Space => "space",
        NodeKind::Folder => "folder",
        NodeKind::List => "list",
    }
}

/// La documentación de este core.
pub struct Docs {
    core: Core,
}

impl Core {
    pub fn docs(&self) -> Docs {
        Docs { core: self.clone() }
    }

    /// Qué se está mirando: documentación si hay un documento abierto.
    pub fn looking(&self) -> Looking {
        if self.docs().state().target.is_some() {
            Looking::Docs
        } else {
            Looking::Tasks
        }
    }

    /// Un clic en un nodo del árbol, entero: aplica la regla de `tree_click`
    /// con lo que se está mirando y, si toca, abre la documentación.
    pub async fn click_node(&self, id: &str) -> Option<TreeClick> {
        let click = self.tree().click(id, self.looking())?;
        if click == TreeClick::OpenDocs {
            self.docs().open(id).await;
        }
        Some(click)
    }

    /// El botón de documentación de «Mi trabajo»: abre la del nodo al que
    /// está acotado, sea lista o espacio (command-and-control-372). Sin acotar
    /// no hay nodo del que colgar, y no hace nada.
    pub async fn open_scope_docs(&self) {
        let Some(scope) = self.my_work().state().scope else { return };
        if docs_reachable_from_scope(Some(scope.kind)) {
            self.docs().open(&scope.id).await;
        }
    }

    /// De la documentación a las tareas **del mismo nodo**: cambiar de vista
    /// no cambia de qué nodo se habla. Una carpeta no acota (lo dice
    /// `tree_click`), así que desde la suya se vuelve a lo que hubiera.
    pub fn back_to_tasks(&self) {
        let Some(target) = self.docs().state().target else { return };
        self.docs().close();
        self.tree().click(&target.id, Looking::Tasks);
    }

    /// «Mi trabajo» entero: sin ámbito y sin documento abierto.
    pub fn show_all_work(&self) {
        self.docs().close();
        self.my_work().set_scope(None);
    }
}

impl Docs {
    pub fn state(&self) -> DocsState {
        self.core.inner.docs.lock().expect("docs lock").clone()
    }

    fn update(&self, change: impl FnOnce(&mut DocsState)) {
        let state = {
            let mut state = self.core.inner.docs.lock().expect("docs lock");
            change(&mut state);
            state.clone()
        };
        self.core.inner.listener.docs_changed(state);
    }

    /// Abre la documentación de un nodo y la trae. Un id que no está en el
    /// árbol no abre nada.
    pub async fn open(&self, id: &str) {
        let Some((kind, name)) = find_node(&self.core.tree().state().spaces, id) else { return };
        let target = DocTarget { kind, id: id.to_string(), name };
        self.update(|state| {
            // El documento de otro nodo no se enseña mientras llega éste:
            // se leería bajo un nombre que no es el suyo.
            if state.target.as_ref() != Some(&target) {
                *state = DocsState { target: Some(target), ..DocsState::default() };
            }
            state.loading = true;
            state.error = None;
        });
        let path = format!("/api/v1/docs/{}/{id}", kind_segment(kind));
        let answer = self.core.get::<DocAnswer>(&path).await;
        // Si mientras tanto se abrió otro, o se cerró, esta respuesta ya no
        // es de lo que se mira.
        if self.state().target.as_ref().map(|target| target.id.as_str()) != Some(id) {
            return;
        }
        match answer {
            Ok(answer) => self.update(|state| {
                state.tabs = answer.tabs;
                state.decisions = answer.decisions;
                state.loading = false;
            }),
            Err(error) => self.update(|state| {
                state.error = Some(error.to_string());
                state.loading = false;
            }),
        }
    }

    pub fn close(&self) {
        if self.state().target.is_some() {
            self.update(|state| *state = DocsState::default());
        }
    }

    /// Cambia de pestaña. Una clave que no es de las cuatro no hace nada.
    pub fn show_tab(&self, key: &str) {
        if DOC_TABS.contains(&key) {
            self.update(|state| state.tab = key.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mientras llega no se dice «vacío», y al recargar lo que ya se leía no
    /// se cambia por un «cargando».
    ///
    /// Roturas que atrapa: «todavía no hay nada aquí» durante la carga de un
    /// documento que sí tiene; un parpadeo a «cargando» en cada recarga.
    #[test]
    fn loading_is_said_only_while_there_is_nothing_to_read() {
        let arriving = DocsState { loading: true, ..DocsState::default() };
        assert_eq!(arriving.empty_reason(), Some("work:tree.loading"));

        let reloading = DocsState {
            loading: true,
            tabs: vec![DocTab { key: "overview".into(), body: "qué es".into(), ..DocTab::default() }],
            ..DocsState::default()
        };
        assert_eq!(reloading.empty_reason(), None);
    }

    /// Rotura que atrapa: enseñar el instante crudo, o una fecha sin año en
    /// un registro que se lee años después.
    #[test]
    fn a_decision_says_the_day_it_was_made() {
        // A mediodía de Greenwich es el mismo día en casi cualquier zona.
        let decision = DocDecision { decided_at: "2026-09-30T12:00:00Z".into(), ..DocDecision::default() };
        assert_eq!(decision.decided_on(Locale::Es), "30 sept 2026");
        assert_eq!(decision.decided_on(Locale::En), "Sep 30 2026");
        assert_eq!(DocDecision { decided_at: "ayer".into(), ..DocDecision::default() }.decided_on(Locale::Es), "");
    }
}
