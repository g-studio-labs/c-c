//! Lo hablado, a trabajo: una tarea o una nota en la documentación de la
//! lista, desde un mensaje (`MessageToTask` en `ChannelView.tsx` y
//! `MessageToDoc.tsx`).
//!
//! Sólo caben listas **del espacio del canal**: una tarjeta hecha desde el
//! canal de un cliente en el tablero de otro es un error con radio de
//! explosión, así que se rechaza aquí y no se deja a quien pinta.

use crate::chat::Chat;
use crate::comms::Conversation;
use crate::error::ApiError;
use crate::mywork::{ScopeKind, WorkScope};
use crate::task::{destinations, Destination, NewTask};
use crate::texts::{text, text_with, Locale};

/// Las pestañas de la documentación que admiten un párrafo: la de decisiones
/// es un registro con su formulario, no markdown.
pub const DOC_TABS_FOR_MESSAGES: [&str; 3] = ["overview", "runbook", "links"];

/// Pasado de 120 letras se corta a 117 y un «…» (`firstLine` en `ChannelView.tsx`).
const TITLE_LIMIT: usize = 120;
const TITLE_KEPT: usize = 117;

fn refused(code: &str, message: &str) -> ApiError {
    ApiError::Server { status: 0, code: code.to_string(), message: message.to_string() }
}

/// El título de la tarea: la primera línea con letra, recortada. Sin ninguna
/// (un mensaje de sólo una imagen) dice «Mensaje».
pub fn task_title(body: &str, locale: Locale) -> String {
    let line = body.lines().map(str::trim).find(|line| !line.is_empty());
    let Some(line) = line else { return text(locale, "common:misc.message") };
    if line.chars().count() <= TITLE_LIMIT {
        return line.to_string();
    }
    let cut: String = line.chars().take(TITLE_KEPT).collect();
    format!("{cut}…")
}

/// El mensaje entero va de cuerpo, no sólo el título: el detalle suele estar
/// en las líneas de después, y una tarjeta que lo perdiera mandaría a releer
/// el canal.
pub fn task_description(body: &str, space_name: &str, locale: Locale) -> String {
    format!("{body}\n\n— {}", text_with(locale, "chat:taskFromChannel", &[("where", space_name)]))
}

/// El párrafo que entra en la documentación: citado y con de dónde salió,
/// porque uno suelto en un runbook no se distingue de uno escrito a propósito.
pub fn doc_paragraph(body: &str, space_name: &str, locale: Locale) -> String {
    let quoted = body.lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n");
    format!("{quoted}\n\n— {}", text_with(locale, "work:docs.fromChannel", &[("where", space_name)]))
}

impl Chat {
    /// Las listas donde puede nacer algo de esta conversación: las de su
    /// espacio. Un directo y la sala general no tienen.
    pub fn work_targets(&self, conversation: &Conversation) -> Vec<Destination> {
        let Conversation::Channel { space_id } = conversation else { return Vec::new() };
        let tree = self.core.tree().state();
        let org_id = self.core.orgs().state().current_org_id;
        let scope = WorkScope { kind: ScopeKind::Space, id: space_id.clone(), name: String::new(), org_id: None };
        destinations(&tree.spaces, org_id.as_deref(), Some(&scope)).options
    }

    fn space_name(&self, conversation: &Conversation) -> String {
        let Conversation::Channel { space_id } = conversation else { return String::new() };
        self.core.tree().state().spaces.iter().find(|space| &space.id == space_id).map(|space| space.name.clone()).unwrap_or_default()
    }

    fn target(&self, conversation: &Conversation, list_id: &str) -> Result<Destination, ApiError> {
        self.work_targets(conversation)
            .into_iter()
            .find(|target| target.list_id == list_id)
            .ok_or_else(|| refused("list-not-in-channel", "That list does not belong to this channel's space"))
    }

    /// Crea una tarea con el mensaje. `internal` sólo cuenta en la lista de
    /// un cliente, y ahí se manda siempre dicho (`public` o `internal`): el
    /// que lo pide ya fue preguntado.
    pub async fn task_from_message(&self, conversation: Conversation, message_id: String, list_id: String, internal: bool, locale: Locale) -> Result<(), ApiError> {
        let target = self.target(&conversation, &list_id)?;
        let body = self.message_text(&conversation, &message_id).ok_or_else(|| refused("message-gone", "That message is not on screen any more"))?;
        let new = NewTask {
            title: task_title(&body, locale),
            description: Some(task_description(&body, &self.space_name(&conversation), locale)),
            visibility: target.client.then(|| if internal { "internal" } else { "public" }.to_string()),
            ..NewTask::default()
        };
        self.core.tasks().create(&list_id, new).await?;
        if let Some(org_id) = self.core.orgs().state().current_org_id {
            self.core.tree().load(&org_id).await;
        }
        Ok(())
    }

    /// Añade el mensaje, citado, a esa pestaña de la documentación de la lista.
    pub async fn save_message_to_docs(&self, conversation: Conversation, message_id: String, list_id: String, tab: String, locale: Locale) -> Result<(), ApiError> {
        if !DOC_TABS_FOR_MESSAGES.contains(&tab.as_str()) {
            return Err(refused("bad-doc-tab", "Only the overview, runbook and links take a paragraph"));
        }
        self.target(&conversation, &list_id)?;
        let body = self.message_text(&conversation, &message_id).ok_or_else(|| refused("message-gone", "That message is not on screen any more"))?;
        let paragraph = doc_paragraph(&body, &self.space_name(&conversation), locale);
        let path = format!("/api/v1/docs/list/{list_id}/tabs/{tab}/append");
        self.core.call_ok("POST", &path, Some(serde_json::json!({ "body": paragraph }))).await
    }

    /// La pregunta de visibilidad, la misma que hace el tablero.
    pub fn client_sees_question(&self, locale: Locale) -> crate::task::Question {
        crate::task::Question {
            title: text(locale, "chat:clientSeesTitle"),
            body: text(locale, "chat:clientSeesBody"),
            warning: None,
            confirm: text(locale, "chat:visibleToThem"),
            cancel: text(locale, "chat:internal"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_is_the_first_line_with_letters_cut_to_the_limit() {
        assert_eq!(task_title("\n  \nrevisar el contrato\ncon Luis", Locale::Es), "revisar el contrato");
        let long = "x".repeat(130);
        let title = task_title(&long, Locale::Es);
        assert_eq!(title.chars().count(), TITLE_KEPT + 1);
        assert!(title.ends_with('…'));
        assert_eq!(task_title("   \n", Locale::Es), "Mensaje");
    }

    #[test]
    fn the_description_keeps_every_line_and_says_where_it_came_from() {
        let said = task_description("uno\ndos", "Producto", Locale::Es);
        assert!(said.starts_with("uno\ndos\n\n— "), "{said}");
        assert!(said.contains("Producto"), "{said}");
    }

    #[test]
    fn the_doc_paragraph_quotes_every_line() {
        let said = doc_paragraph("uno\n\ndos", "Producto", Locale::Es);
        assert!(said.starts_with("> uno\n> \n> dos\n\n— "), "{said}");
        assert!(said.contains("Producto"), "{said}");
    }
}
