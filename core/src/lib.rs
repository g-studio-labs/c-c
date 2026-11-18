//! La lógica de cac que comparten todas sus apps.
//!
//! Una sola base para el escritorio, el móvil y el MCP: lo que decide algo
//! —la sesión, qué se pide al servidor, en qué día cae un vencimiento— vive
//! aquí, y cada plataforma sólo pinta y aporta lo suyo del sistema. El diseño
//! entero está en cac, en la página «Diseño · Core en Rust compartido».

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub mod account;
pub mod chat;
pub mod comms;
pub mod comms_text;
pub mod comms_wire;
pub mod counters;
pub mod dates;
pub mod error;
pub mod doc_edit;
pub mod doc_header;
pub mod doc_links;
pub mod doc_pages;
pub mod doc_reading;
pub mod docs;
pub mod events;
pub mod mywork;
pub mod nav;
pub mod orgs;
pub mod prefs;
pub mod prose;
pub mod session;
pub mod storage;
pub mod stream;
pub mod task;
pub mod texts;
pub mod transport;
pub mod tree;
pub mod hue;
pub mod tree_rows;
pub mod workspace;

use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use account::{check_new_password, check_sign_in, gate, FormCheck, Gate, MIN_PASSWORD};
pub use chat::{
    around_label, enter_sends, file_token, mention_token, message_body, message_to_send, picture_step, thread_lines, thread_pictures, thread_splice, BodyFile, Chat, ComposerToken, MessageBody,
    ThreadLine, ThreadLink, ThreadMessage, ThreadPicture, ThreadQuote, ThreadSplice, ThreadState, MESSAGE_MAX_CHARS, THREAD_PAGE,
};
pub use comms::{
    comms_panel, conversation_title, people_matching, resolve_conversation, Channel, Comms, CommsPanel, CommsRow, CommsRows, CommsSection, CommsState,
    CommsViewer, Conversation, ConversationTitle,
};
pub use comms_text::{list_time, mentions_a_call, preview_line, quote_text, seen_recently, was_read, LastLine};
pub use comms_wire::{
    ChatFile, ChatLatest, ChatMessage, ChatMessageEvent, ChatUnread, DmMessage, DmMessageEvent, DmReadEvent, DmSummary, MessageQuote, Person, ReactionSummary,
    REACTION_EMOJIS, SYSTEM_KIND,
};
pub use counters::{count_badge, counted_label, rings_bell, Counters, CountersState, BELL_CAP, RAIL_CAP};
pub use dates::{date_and_time, due_day, due_state, short_date, today_in, zone_hint, DueState};
pub use doc_edit::{close_window_question, doc_editor_side_by_side, DocAsk, DocConflict, DocEdit, DocEditError, DocEditWhat, DocLeave, SIDE_BY_SIDE_FROM};
pub use doc_header::{DocHeader, DocInfo};
pub use doc_links::{doc_link, heading_slug, DocJump, DocLink, DocLinkFailure};
pub use doc_pages::{DocBacklink, DocBacklinkRow, DocPage, DocPageCrumb, DocPageItem, DocPageLink, DocPageRow, DocPageView};
pub use doc_reading::{decision_record_link, doc_active_entry, doc_headings, doc_pieces, DocEntryPlace, DocHeading, DocPiece, DocPieceEnd};
pub use docs::{doc_blocks, DocBlock, DocDecision, DocSection, DocTab, DocTarget, Docs, DocsState, DOC_TABS};
pub use error::ApiError;
pub use events::{Listener, NoListener};
pub use mywork::{
    can_drop, work_board, BoardAssignee, BoardCard, BoardColumn, BoardStatus, DropVerdict, BAD_TRANSITION, COLUMN_MISSING,
    day_and_month, due_label, empty_reason, filter_options, in_scope, work_groups, DueLabel, FilterOptions, MyWork, MyWorkState, OpenTask,
    ScopeKind, TaskFilter, WorkGroup, WorkLens, WorkRow, WorkScope, NOBODY, PRIORITIES,
};
pub use mywork::{
    calendar_chips, calendar_tone, day_detail_opens_up, day_title, lane_open_by_default, month_start, month_title, month_view, tone_is_high, unscheduled,
    week_start, week_title, week_view, weekday_name, CalendarChip, CalendarCursor, CalendarDay, CalendarView, CalendarWeek, MonthView,
    ToneCount, WeekSummary, WeekView, CALENDAR_TONES,
};
pub use nav::{docs_reachable_from_scope, tree_click, Looking, NodeKind, TreeClick};
pub use hue::{hue_index, HUES};
pub use orgs::{initials, org_hint, Organization, Orgs, OrgsState};
pub use prose::prose_for_painting;
pub use prefs::{first_spoken, folds_panel, is_dark, shortcut_text, KeysHeld, ThemePreference, WorkLayout};
pub use session::Session;
pub use tree::{
    can_edit_tree, clean_node_name, delete_notice, find_list, node_menu, tree_edit_phrase, DeleteNotice, NodeAction, TreeEditError, MAX_NODE_NAME,
};
pub use tree::{doc_key, find_folder, folder_list_ids, node_name, DocMark, FolderTree, ListSummary, SpacePerson, SpaceTree, Tree, TreeState};
pub use tree_rows::{filter_tree, tree_rows, tree_rows_matching, TreeRow};
pub use storage::{FileStorage, Storage};
pub use stream::{backoff, parse_frame, Connection, ConnectionState, FrameBuffer, ServerEvent, Stream};
pub use task::{
    can_transition, canonical_priority, canonical_status, due_instant, flow_for, new_task_body, open_subtasks_block, patch_body, status_refusal,
    status_refusal_phrase, SUBTASKS_OPEN,
    CommentAuthor, DecisionRef, DueChange, Flow, GitSummary, NewTask, ReleaseRef, StatusOption, Task,
    TaskAttachment, TaskBlocker, TaskCard, TaskComment, TaskDetail, TaskGitLink, TaskGitLinks, TaskPatch,
    TaskState, TaskStatus, TaskTag, Tasks, UserSummary,
};
pub use task::{attachment_fits, attachment_path, citation, file_citation, image_type, is_image, multipart, named_image, safe_file_name, size_label, thumbnail_size,
    with_citation, Cited, Picture,
    MAX_ATTACHMENT_BYTES, THUMBNAIL_MAX_HEIGHT, THUMBNAIL_MAX_WIDTH,
};
pub use task::{destinations, drafted, Destination, Destinations, TaskDraft};
pub use task::{
    comment_placeholder, mention_link, mention_matches, mention_query, mentioned_user, mentions_allowed, offered_mentions, with_mention,
    mention_closed, mention_highlighted, mention_mark_kept, mention_menu, mention_moved, MentionEdit, MentionMark, MentionMenu, MentionPerson, MentionQuery,
};
pub use task::{compose, continue_list, ComposeAction, ComposeEdit};
pub use task::{assignee_choices, retitled, shown_name, tag_choices, tag_color_for, title_line, Choice, MAX_TAG_NAME, TAG_PALETTE};
pub use task::{can_change_comment, is_agent_note, says_something, task_sheet, SheetActivity, SheetBlocker, SheetComment, SheetFile, SheetSubtask, SheetTag, StatusChoice, TaskSheet};
pub use texts::{error_phrase, pick_locale, text, text_count, text_count_unit, text_with, Locale};
pub use transport::Reply;

/// Con qué servidor habla el core y cómo se presenta.
#[derive(Debug, Clone)]
pub struct Config {
    /// Sin barra final: `https://cac.guz-studio.dev`.
    pub base_url: String,
    /// Qué cliente es, tal como lo espera el login (`desktop`, `web`). El
    /// servidor decide con eso qué rutas le abre.
    pub client: Option<String>,
    /// El plazo de cada petición.
    pub request_timeout: Duration,
    /// Cuánto vale por reciente un documento que se acaba de leer: al
    /// volver a la ventana no se vuelve a pedir si se leyó hace menos.
    pub doc_fresh_for: Duration,
    /// Cuánto silencio aguanta el stream de eventos antes de darlo por
    /// muerto. El servidor manda un ping cada 25 s: dos sin llegar.
    pub stream_silence: Duration,
    /// La primera espera antes de reintentar el stream; crece hasta treinta
    /// veces esto.
    pub stream_backoff: Duration,
    /// Cuánto puede faltar el stream antes de avisar a quien mira. Menos que
    /// esto es una reconexión de las normales.
    pub stream_grace: Duration,
}

impl Config {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: None,
            request_timeout: Duration::from_secs(12),
            doc_fresh_for: Duration::from_secs(5),
            stream_silence: Duration::from_secs(60),
            stream_backoff: Duration::from_secs(1),
            stream_grace: Duration::from_secs(45),
        }
    }
}

/// El core. Uno por proceso; clonarlo es barato y todos los clones son el
/// mismo.
#[derive(Clone)]
pub struct Core {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) config: Config,
    pub(crate) http: reqwest::Client,
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) listener: Arc<dyn Listener>,
    pub(crate) held: Mutex<Option<session::Held>>,
    /// El turno para renovar la sesión: ver `Core::refresh`.
    pub(crate) refreshing: tokio::sync::Mutex<()>,
    pub(crate) my_work: Mutex<MyWorkState>,
    pub(crate) orgs: Mutex<OrgsState>,
    pub(crate) tree: Mutex<TreeState>,
    pub(crate) docs: Mutex<DocsState>,
    /// Cuándo se leyó del servidor el documento abierto: ver
    /// `Docs::came_back`.
    pub(crate) doc_read_at: Mutex<Option<std::time::Instant>>,
    /// Cuántos guardados de documentación han salido bien: una lectura que
    /// se pidió antes de uno y llega después trae lo de antes, y no cuenta.
    pub(crate) doc_saves: std::sync::atomic::AtomicU64,
    pub(crate) task: Mutex<TaskState>,
    pub(crate) counters: Mutex<counters::CountersData>,
    /// Cuántas veces se ha pedido volver a contar cada cosa: ver
    /// `Counters::reload_soon`.
    pub(crate) counter_reloads: [std::sync::atomic::AtomicU64; counters::SOURCES],
    pub(crate) comms: Mutex<comms::CommsData>,
    /// Cuántas veces se ha pedido volver a traer las últimas líneas de los
    /// canales: ver `Comms::reload_soon`.
    pub(crate) comms_reloads: std::sync::atomic::AtomicU64,
    pub(crate) chat: Mutex<chat::ChatData>,
    /// Cuál de las aperturas de una conversación es la vigente: ver
    /// `Chat::open`.
    pub(crate) chat_turn: std::sync::atomic::AtomicU64,
    /// Cuántas veces se ha pedido releer la conversación: ver
    /// `Chat::refresh_soon`.
    pub(crate) chat_reloads: std::sync::atomic::AtomicU64,
    /// Cuántas lecturas de la conversación se han pedido: ver
    /// `Chat::refresh`.
    pub(crate) chat_reads: std::sync::atomic::AtomicU64,
    pub(crate) connection: Mutex<ConnectionState>,
    /// Qué stream de eventos es el vigente: ver `Stream::current`.
    pub(crate) stream_turn: std::sync::atomic::AtomicU64,
    /// Cuántas recargas de «Mi trabajo» se han pedido: la que espera su
    /// respiro sólo carga si sigue siendo la última.
    pub(crate) my_work_reloads: std::sync::atomic::AtomicU64,
}

impl Core {
    /// Arranca con la sesión que hubiera guardada.
    pub fn new(
        config: Config,
        storage: Arc<dyn Storage>,
        listener: Arc<dyn Listener>,
    ) -> Result<Self, ApiError> {
        let core = Self {
            inner: Arc::new(Inner {
                config,
                http: transport::new_client()?,
                storage,
                listener,
                held: Mutex::new(None),
                refreshing: tokio::sync::Mutex::new(()),
                my_work: Mutex::new(MyWorkState::default()),
                orgs: Mutex::new(OrgsState::default()),
                tree: Mutex::new(TreeState::default()),
                docs: Mutex::new(DocsState::default()),
                doc_read_at: Mutex::new(None),
                doc_saves: std::sync::atomic::AtomicU64::new(0),
                task: Mutex::new(TaskState::default()),
                counters: Mutex::new(counters::CountersData::default()),
                counter_reloads: Default::default(),
                comms: Mutex::new(comms::CommsData::default()),
                comms_reloads: std::sync::atomic::AtomicU64::new(0),
                chat: Mutex::new(chat::ChatData::default()),
                chat_turn: std::sync::atomic::AtomicU64::new(0),
                chat_reloads: std::sync::atomic::AtomicU64::new(0),
                chat_reads: std::sync::atomic::AtomicU64::new(0),
                connection: Mutex::new(ConnectionState::default()),
                stream_turn: std::sync::atomic::AtomicU64::new(0),
                my_work_reloads: std::sync::atomic::AtomicU64::new(0),
            }),
        };
        let held = core.load_held();
        *core.inner.held.lock().expect("held lock") = held;
        core.my_work().restore_preferences();
        core.orgs().restore();
        core.comms().restore();
        Ok(core)
    }
}

#[cfg(test)]
mod tests {
    /// Las dependencias que declara el manifiesto, por su nombre.
    fn dependency_names(manifest: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut in_dependencies = false;
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                // También `[target.'cfg(…)'.dependencies]` y `[dev-dependencies]`:
                // una interfaz que sólo se enlaza en una plataforma o en las
                // pruebas sigue siendo una interfaz dentro del core.
                in_dependencies = line.trim_end_matches(']').ends_with("dependencies");
                continue;
            }
            if !in_dependencies || line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((name, _)) = line.split_once('=') {
                names.push(name.trim().trim_matches('"').to_string());
            }
        }
        names
    }

    /// El core no enlaza ninguna interfaz.
    ///
    /// Es lo que lo hace compartible: con `tauri` o `gtk` dentro dejaría de
    /// compilar para Android, y el día que el escritorio cambie de interfaz se
    /// llevaría la lógica detrás. Nada más lo avisaría hasta ese día.
    #[test]
    fn the_core_depends_on_no_ui() {
        const UI: [&str; 7] = ["tauri", "wry", "tao", "gtk", "webkit", "gpui", "objc2"];
        let names = dependency_names(include_str!("../Cargo.toml"));
        let offending: Vec<&String> = names
            .iter()
            .filter(|name| UI.iter().any(|ui| name.starts_with(ui)))
            .collect();
        assert!(offending.is_empty(), "el core no puede depender de {offending:?}");
    }

    /// El lector de arriba ve de verdad las dependencias: sin esto, uno que no
    /// encontrara ninguna daría la prueba anterior por buena para siempre.
    #[test]
    fn the_manifest_reader_finds_dependencies() {
        let manifest = "[package]\nname = \"x\"\n\n[dependencies]\n# nota\nserde = \"1\"\n\
                        tauri = { version = \"2\" }\n\n[target.'cfg(unix)'.dependencies]\ngtk = \"0.18\"\n";
        assert_eq!(dependency_names(manifest), ["serde", "tauri", "gtk"]);
    }
}
