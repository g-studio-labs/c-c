//! Lo que el servidor manda de la comunicación: canales, directos y sus
//! mensajes, tal como viajan.
//!
//! Vino de `app/src/store/chat.store.ts`, `dm.store.ts` y `lib/reactions.ts`.
//! Cada tipo es espejo de un struct de Go y lo ata el contrato
//! (`tests/contract.rs`, con los ficheros que escribe
//! `core_contract_test.go`), **menos dos**: los eventos `chat:message` y
//! `dm:message`, que el servidor escribe como mapas sueltos y no tienen
//! struct que fijar. Van al final, y lo dicen.

use serde::{Deserialize, Serialize};

/// El juego cerrado de emojis con los que se reacciona, en el orden del
/// selector. Espejo de `domain.ReactionEmojis`.
///
/// Tiene que ser **el mismo que acepta el servidor**: uno de más aquí es un
/// botón que siempre falla con un 400; uno de menos, un emoji que llega de
/// otra pantalla y no se puede poner desde ésta.
/// Guardián: `tests::the_emoji_set_is_the_servers`, que lee el fichero de Go.
pub const REACTION_EMOJIS: [&str; 8] = ["👍", "❤️", "😂", "🎉", "👀", "✅", "🙏", "🔥"];

/// La clase de una línea que puso cac y no una persona (`domain.ChatKindSystem`).
pub const SYSTEM_KIND: &str = "system";

/// Un emoji de un mensaje, ya contado. Espejo de `domain.ReactionSummary`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ReactionSummary {
    pub emoji: String,
    pub count: i64,
    /// Si quien pregunta está entre ellos: se pinta marcado y pulsarlo lo quita.
    pub mine: bool,
    /// Quiénes, en el orden en que reaccionaron.
    #[serde(default)]
    pub names: Vec<String>,
}

/// Lo que una respuesta enseña de lo que cita. Espejo de `domain.MessageQuote`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct MessageQuote {
    pub id: String,
    #[serde(default)]
    pub author_name: String,
    /// Recortado en el servidor. Vacío si `gone`.
    #[serde(default)]
    pub body: String,
    /// Lo citado se retiró: la respuesta sigue y dice que ya no está.
    #[serde(default)]
    pub gone: bool,
}

/// Una línea del canal de un espacio. Espejo de `domain.ChatMessageResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub space_id: String,
    /// Siempre una persona, también en una línea del sistema: quien la provocó.
    pub author_user_id: String,
    #[serde(default)]
    pub author_name: String,
    /// `user` o `system`: quién habla, no quién puede escribir.
    #[serde(default)]
    pub kind: String,
    /// Por dónde entró: `mcp` es un agente con el token de esa persona.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub reactions: Vec<ReactionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageQuote>,
}

impl ChatMessage {
    pub fn is_system(&self) -> bool {
        self.kind == SYSTEM_KIND
    }
}

/// Una línea de una conversación privada. Espejo de
/// `domain.DMMessageResponse`. No lleva `kind`: en un directo no escribe cac.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DmMessage {
    pub id: String,
    pub conversation_id: String,
    pub author_user_id: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub reactions: Vec<ReactionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageQuote>,
}

/// La última línea de un canal, para su fila en la lista. Espejo de
/// `domain.ChatLatest`. El texto llega recortado: es un adelanto.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ChatLatest {
    pub space_id: String,
    #[serde(default)]
    pub author_user_id: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub body: String,
    pub created_at: String,
}

impl ChatLatest {
    pub fn is_system(&self) -> bool {
        self.kind == SYSTEM_KIND
    }
}

/// Lo sin leer de un canal. Espejo de `domain.ChatUnread`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct ChatUnread {
    pub space_id: String,
    pub count: i64,
}

/// Una conversación privada en la lista: con quién, y si ha dicho algo que
/// no se ha leído. Espejo de `domain.DMSummary`.
///
/// **Es de una organización** (`org_id`): dos personas que trabajan juntas en
/// dos tienen dos conversaciones, y el servidor las manda todas.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DmSummary {
    pub conversation_id: String,
    pub org_id: String,
    /// El otro.
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub unread: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    /// Del otro: si anda por aquí.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    /// El último mensaje, recortado, y de quién es.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_author_id: Option<String>,
    /// Hasta dónde leyó el otro: lo propio escrito antes está leído.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_read_at: Option<String>,
}

/// Alguien de la organización a quien se le puede escribir. Espejo de
/// `domain.UserSummary`, con lo que el de las tareas (`task::UserSummary`)
/// no lee: cuándo dio señales por última vez.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    /// Para buscar e identificar.
    pub username: String,
    /// Para enseñar. Puede faltar: se cae al usuario.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
}

impl Person {
    /// Cómo se le llama: su nombre, y si no lo ha puesto, su usuario
    /// (`nombreDe` en `app/src/lib/nombres.ts`).
    pub fn shown_name(&self) -> &str {
        self.name.as_deref().map(str::trim).filter(|name| !name.is_empty()).unwrap_or(&self.username)
    }
}

/// Lo que contesta el servidor al subir un fichero a un canal
/// (`domain.ChatAttachment`) o a un directo (`domain.DMAttachment`). La
/// `url` es relativa: se cita tal cual en el mensaje, que es lo único que
/// une el fichero al mensaje.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ChatFile {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub space_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub conversation_id: String,
    pub url: String,
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded_by: Option<String>,
}

/// Para nombrar a alguien de comunicación con las reglas de las menciones,
/// que se escribieron con la gente de una tarea.
impl From<Person> for crate::task::UserSummary {
    fn from(person: Person) -> Self {
        Self { id: person.id, username: person.username, name: person.name }
    }
}

/// El evento `dm:read`: alguien leyó una conversación hasta `at`. Llega sólo
/// al otro. Espejo de `domain.DMReadEvent`, el único de los tres eventos de
/// aquí que el servidor escribe con un struct.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct DmReadEvent {
    pub conversation_id: String,
    /// Quién leyó.
    #[serde(default)]
    pub user_id: String,
    pub at: String,
}

// ── Sin fijar por el contrato ───────────────────────────────────────────
//
// Los dos de abajo **no los ata `tests/contract.rs`**. El servidor los
// escribe como mapas sueltos —`map[string]any` en `publicarAlStream`
// (`backend/internal/core/service/chat.go`) y `map[string]string` en
// `publish` (`service/dm.go`)—, así que no hay struct de Go del que sacar un
// fichero: están escritos leyendo ese código el 18-nov-2026. Una clave que
// allí se renombre no rompe ninguna prueba; por eso todo campo tiene valor
// por omisión y quien los usa aguanta que falte cualquiera. Fijarlos pide
// darles un struct en el servidor, y eso está pendiente de decidir.

/// El evento `chat:message`: hay una línea nueva en un canal, o una que ya
/// estaba cambió (`edited`). **No fijado por el contrato** (ver arriba).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct ChatMessageEvent {
    pub space_id: String,
    pub message_id: String,
    /// Quien provocó la línea: en una del sistema, quien grabó, no quien firma.
    pub actor_id: String,
    pub kind: String,
    pub space_name: String,
    /// Vacío en una línea del sistema: ahí no firma nadie.
    pub author_name: String,
    /// El adelanto del texto, ya recortado y en limpio.
    pub preview: String,
    /// A quién se nombró. `null` si a nadie.
    pub mentions: Option<Vec<String>>,
    /// A quién se contesta. Vacío si a nadie.
    pub replied_to: String,
    pub edited: bool,
}

impl ChatMessageEvent {
    pub fn is_system(&self) -> bool {
        self.kind == SYSTEM_KIND
    }
}

/// El evento `dm:message`: alguien escribió en una conversación privada.
/// Llega sólo al otro, y **sin el texto**: lo privado no viaja en un aviso.
/// **No fijado por el contrato** (ver arriba).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase", default)]
pub struct DmMessageEvent {
    pub conversation_id: String,
    pub message_id: String,
    pub actor_id: String,
    pub author_name: String,
    /// Una corrección o un retiro, no un mensaje nuevo. Desde el 19-nov-2026
    /// (command-and-control-418); antes no venía y se tomaba por nuevo.
    pub edited: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El juego de emojis es el del servidor, y en su orden: se lee el
    /// fichero de Go, como hace `app/src/lib/reactions.test.ts`.
    ///
    /// Roturas que atrapa: un emoji de más aquí (un botón que siempre da
    /// 400); uno de menos (llega de otra pantalla y no se puede poner); el
    /// selector en otro orden que el de las demás apps.
    #[test]
    fn the_emoji_set_is_the_servers() {
        let path = format!("{}/../backend/internal/core/domain/reaction.go", env!("CARGO_MANIFEST_DIR"));
        let go = std::fs::read_to_string(path).expect("el fichero de Go con el juego de emojis");
        let line = go.lines().find(|line| line.starts_with("var ReactionEmojis")).expect("la línea de ReactionEmojis");
        let servers: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        assert!(servers.len() > 3, "no se leyó el juego del servidor: {line}");
        assert_eq!(REACTION_EMOJIS.to_vec(), servers);
    }

    /// Los dos eventos sin struct en el servidor se leen de lo que escribe
    /// hoy su código, y aguantan lo que falte.
    ///
    /// Roturas que atrapa: `mentions: null` (a nadie se nombró, y Go escribe
    /// así una lista vacía) tumbando la lectura del evento entero, con lo que
    /// la fila del canal no se movería; un evento sin `edited` o sin
    /// `preview` que no se puede leer.
    #[test]
    fn the_loose_events_are_read_as_the_server_writes_them_today() {
        let event: ChatMessageEvent = serde_json::from_str(
            r#"{"spaceId":"s1","messageId":"m1","actorId":"u2","kind":"system","spaceName":"Producto","authorName":"",
                "preview":"Grabación lista","mentions":null,"repliedTo":"","edited":true}"#,
        )
        .unwrap();
        assert_eq!((event.space_id.as_str(), event.preview.as_str(), event.edited), ("s1", "Grabación lista", true));
        assert!(event.is_system());
        assert_eq!(event.mentions, None);

        let bare: ChatMessageEvent = serde_json::from_str(r#"{"spaceId":"s1","mentions":["u1"]}"#).unwrap();
        assert_eq!((bare.space_id.as_str(), bare.edited, bare.is_system()), ("s1", false, false));
        assert_eq!(bare.mentions.as_deref(), Some(&["u1".to_string()][..]));

        let dm: DmMessageEvent =
            serde_json::from_str(r#"{"conversationId":"c1","messageId":"d1","actorId":"u2","authorName":"Luis"}"#).unwrap();
        assert_eq!((dm.conversation_id.as_str(), dm.actor_id.as_str()), ("c1", "u2"));
    }
}
