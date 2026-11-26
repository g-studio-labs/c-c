//! La lógica de cac que comparten todas sus apps.
//!
//! Una sola base para el escritorio, el móvil y el MCP: lo que decide algo
//! —la sesión, qué se pide al servidor, en qué día cae un vencimiento— vive
//! aquí, y cada plataforma sólo pinta y aporta lo suyo del sistema. El diseño
//! entero está en cac, en la página «Diseño · Core en Rust compartido».

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub mod account;
pub mod ansi;
pub mod call_card;
pub mod channel_tabs;
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
pub mod doc_share;
pub mod docs;
pub mod docs_panel;
pub mod events;
pub mod message_to;
pub mod mywork;
pub mod nav;
pub mod notes;
pub mod notice;
pub mod notice_sound;
pub mod org_admin;
pub mod org_github;
pub mod org_integrations;
pub mod org_meetings;
pub mod orgs;
pub mod overview;
pub mod platform;
pub mod prefs;
pub mod prose;
pub mod request_links;
pub mod requests;
pub mod session;
pub mod storage;
pub mod stream;
pub mod task;
pub mod texts;
pub mod transport;
pub mod tree;
pub mod hue;
pub mod inbox;
pub mod inbox_peek;
pub mod tree_rows;
pub mod ring_tone;
pub mod voice;
pub mod voice_ring;
pub mod wall;
pub mod workspace;

use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use account::{check_new_password, check_sign_in, gate, FormCheck, Gate, PasswordChanged, MIN_PASSWORD};
pub use chat::{
    apply_reaction, around_label, enter_sends, file_token, mention_token, message_body, message_to_send, picture_step, thread_lines, thread_pictures, thread_splice, BodyFile, Chat, ComposerToken, MessageBody,
    ThreadLine, ThreadLink, ThreadMessage, ThreadPicture, ThreadQuote, ThreadSplice, ThreadState, MESSAGE_MAX_CHARS, THREAD_PAGE,
};
pub use call_card::{call_card, call_length, CallCard, CallFace, CallParticipant, CallSession};
pub use channel_tabs::{
    call_title, channel_tabs, guest_link, local_moment, recording_duration, row_meta, recording_file_name, search_page, search_path, tab_path, CallInvite, CallOccupant, ChannelTab, ChannelTabs,
    ChatLinkItem, ChatMediaItem, PagedTab, Recording, RecordingPlace, RecordingPolicy, TabItem, TabPage, TabShows, CALL_TITLE_MAX, GUEST_TTLS, SEARCH_PAGE,
};
pub use comms::{
    comms_panel, conversation_title, people_matching, resolve_conversation, Channel, Comms, CommsPanel, CommsRow, CommsRows, CommsSection, CommsState,
    CommsViewer, Conversation, ConversationTitle,
};
pub use comms_text::{call_id_in, list_time, mentions_a_call, preview_line, quote_text, seen_recently, was_read, LastLine};
pub use comms_wire::{
    ChatFile, ChatLatest, ChatMessage, ChatMessageEvent, ChatUnread, DmMessage, DmMessageEvent, DmReadEvent, DmSummary, MessageQuote, Person, ReactionSummary,
    REACTION_EMOJIS, SYSTEM_KIND,
};
pub use counters::{count_badge, counted_label, rings_bell, Counters, CountersState, BELL_CAP, RAIL_CAP};
pub use dates::{date_and_time, due_day, due_state, short_date, today_in, zone_hint, DueState};
pub use doc_edit::{close_window_question, doc_editor_side_by_side, DocAsk, DocConflict, DocEdit, DocEditError, DocEditWhat, DocLeave, SIDE_BY_SIDE_FROM};
pub use doc_header::{DocHeader, DocInfo};
pub use notes::{note_link_target, note_rows, Note, NoteAsk, NoteAttachment, NoteConflict, NoteDetail, NoteEdit, NoteLeave, NoteLink, NoteProblem, NoteRow, NoteSearch, NoteTrash, NoteTrashItem, NoteTreeItem, Notes, NotesState};
pub use doc_share::{shared_doc_link, DocShare, DocShares, DOC_SHARE_TTLS};
pub use doc_links::{doc_link, heading_slug, DocJump, DocLink, DocLinkFailure};
pub use doc_pages::{DocBacklink, DocBacklinkRow, DocPage, DocPageCrumb, DocPageItem, DocPageLink, DocPageRow, DocPageView};
pub use doc_reading::{decision_record_link, doc_active_entry, doc_headings, doc_pieces, DocEntryPlace, DocHeading, DocPiece, DocPieceEnd};
pub use docs_panel::{doc_index, DocIndex, DocIndexRow, DocTreeItem, DocTreeRow};
pub use docs::{doc_blocks, DocBlock, DocDecision, DocSection, DocTab, DocTarget, Docs, DocsState, DOC_TABS};
pub use error::ApiError;
pub use events::{Listener, NoListener};
pub use mywork::{
    can_drop, work_board, BoardAssignee, BoardCard, BoardColumn, BoardStatus, DropVerdict, BAD_TRANSITION, COLUMN_MISSING,
    day_and_month, due_label, empty_reason, filter_options, in_scope, AssigneeOption, work_groups, DueLabel, FilterOptions, MyWork, MyWorkState, OpenTask,
    ScopeKind, TaskFilter, WorkGroup, WorkLens, WorkRow, WorkScope, NOBODY, PRIORITIES,
};
pub use mywork::{
    calendar_chips, calendar_tone, day_detail_opens_up, day_title, lane_open_by_default, month_start, month_title, month_view, tone_is_high, unscheduled,
    week_start, week_title, week_view, weekday_name, CalendarChip, CalendarCursor, CalendarDay, CalendarView, CalendarWeek, MonthView,
    ToneCount, WeekSummary, WeekView, CALENDAR_TONES,
};
pub use notice::{channel_notice, direct_notice, Announcement, NoticeKind, NoticeTarget, Notices, Viewer};
pub use inbox::{group_key_of, inbox_panel, link_target, time_ago, GroupTally, Inbox, InboxFeed, InboxGroup, InboxIcon, InboxItem, InboxPanel, InboxRow, InboxState, InboxTab, InboxTarget, InboxTone, INBOX_TABS};
pub use inbox_peek::{peek_path, previews_wanted, quick_lines, row_preview, with_previews, PreviewWanted, QuickLine, PEEK_LIMIT};
pub use notice_sound::{render, tone_for, Chime, Tone, TONE_GAP};
pub use nav::{docs_reachable_from_scope, tree_click, Looking, NodeKind, TreeClick};
pub use hue::{hue_index, HUES};
pub use org_admin::{
    account_notice, can_manage_org, date_with_year, delete_org_confirmed, delete_org_phrase, invitation_deadline, member_face, members_matching, org_summary, org_tabs, space_access_text, space_cards,
    username_initials, valid_email, valid_team_zone, AccountLink, AccountNotice, AddOutcome, Invitation, OrgMember, OrgPatch, OrgTab, PersonHit, ReportProject, SpaceAccess, SpaceCard, SpaceFace, ORG_ROLES,
};
pub use org_github::{
    bare_refs_enabled, docs_line, linkable_spaces, shown_repos, GitHubInstallation, GitHubRepo, GitHubStatus, GitHubUnbound, RepoChange,
};
pub use org_integrations::{inbox_lists, list_path, lists_by_space, ListPath, ProjectForm, ProjectPatch};
pub use org_meetings::{
    default_meeting_zone, dual_time, hour_in, is_zone, local_zone_name, meeting_preview, meeting_when, next_ring, normalized_wall_time, readable_rule, readable_weekdays, reached, toggled_exclusion, wall_time_today,
    zone_city, zone_label, zone_offset, Freq, Meeting, MeetingDraft, MeetingForm, MeetingRoom, MeetingWhen, ALL_DAYS, WEEK_ORDER, WORKWEEK,
};
pub use orgs::{initials, org_hint, Organization, Orgs, OrgsState};
pub use overview::{
    is_open_report, long_date, overview_view, Figure, InfraCard, Overview, OverviewList, OverviewReport, OverviewRow, OverviewState, OverviewView, UnreadCard, UnreadRow,
};
pub use wall::{
    audience_label, audience_text, comment_meta, comments_label, edit_payload, post_detail_meta, post_meta, post_payload, NewPost, PostAudience, PostKind, Wall, WallComment, WallPermissions,
    WallPost, WallState, POST_BODY_MAX, POST_COMMENT_MAX, POST_TITLE_MAX,
};
pub use platform::{
    agent_base, agent_update_outcome, platform_sections, server_edit_phrase, server_permissions, server_tabs, servers_of, Agent, AgentAction, AgentIdentity, AgentInstallOutcome, AgentInstallPhase, AgentJob, AgentNote, AgentWait, AGENT_VERSION_LATEST, DeployableSummary, IncidentServerRef, IncidentSummary, ProvisioningRun, ServerOverview, ServerTab, AGENT_VERSION_DEPLOYS, AgentStream, Platform, PlatformSection, PlatformState, Server, ServerEditError, ServerField, ServerForm, ServerHealth, ServerKind, ServerPermissions,
    AGENT_TIMEOUT, PASS_MARGIN, PROBE_TIMEOUT, filter_services, LogLine, LogStatus, ServiceHealth, ServiceLogs, ServiceReplicas, SwarmService, LOG_LINES_KEPT, cpu_load, format_bytes, mem_load, stats_view, cert_warns, cluster_status, days_until, route_links, workloads_failing_first, ClusterBadge, ClusterStatus, Integration, K8sCert, K8sDatastore, K8sGateway, K8sHealth, K8sHub, K8sNode, K8sRoute, K8sRoutes, K8sWorkload, CERT_WARN_DAYS, ContainerStats, Load, StatsColumn, StatsSort, StatsView, SwarmNode,
    append_rows, label_to_name, merge_build_args, ref_problems, valid_secret_name, valid_secret_ref, RefProblem, SecretRefRow, SecretRotation, SecretUsage, SecretsState,
    commit_pick, infer_owner_repo, repo_of_deployable, CommitPick, CommitRow, ci_notice_step, guess_environment, is_sha, repo_from_image, runs_commit, short_ref, short_service_name, uses_short_sha, CiKey, CiNotify, DeployStatus, Deployable, DeployableSettings, Deployment, DeploysState, ImageBuild, NewDeployable, RepoCommit, AGENT_VERSION_BUILDS, AGENT_VERSION_MIGRATES, LIST_PREVIEW, SECRETS_ENTRYPOINT_SNIPPET,
};
pub use platform::activity::*;
pub use platform::users::*;
pub use platform::diagnostics::*;
pub use platform::provision::*;
pub use platform::provision_run::*;
pub use platform::{detect_databases, is_age_public_key, Backup, BackupEngine, BackupKind, BackupRecipient, BackupStatus, BackupTarget, BackupTargetForm, BackupsState, BackupsStatus, DetectedDatabase, DEFAULT_BACKUP_TIMEZONE};
pub use ansi::{ansi_spans, AnsiColor, AnsiSpan};
pub use prose::prose_for_painting;
pub use request_links::{link_segments, LinkSegment};
pub use requests::{
    build_answer, elsewhere_label, request_outcome, waiting_label, AgentRequest, AnswerDraft, RequestAnswer, RequestBody, RequestKind, RequestOption, RequestPerson, RequestScope, RequestStatus,
    RequestTask, Requests, RequestsElsewhere, RequestsState,
};
pub use prefs::{first_spoken, folds_panel, is_dark, rail_slot, shortcut_text, KeysHeld, ThemePreference, WorkLayout};
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
    AgentSignature, CommentAuthor, DecisionRef, DueChange, Flow, GitSummary, NewTask, ReleaseRef, StatusOption, Task,
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
pub use ring_tone::{render_ring, ring_cycle, RingTone};
pub use voice_ring::{ring_clock, ring_tone_now};
pub use voice::{
    disconnect_reason, join_names, DeviceChoices, person_initials, voice_error_phrase, voice_layout, voice_room_name, voice_status, voice_tiles, Publishing, ScreenSource, ScreenSources, StageItem, VideoSource, VoiceFocus, VoiceLayout, VoiceTile, IncomingRing, MeetToken, MovedCall, OccupancyWatch, OutgoingRing, Voice, VoiceCaller, VoiceEngine, VoiceError, VoiceEvent, VoiceFuture,
    VoicePeer, VoicePhase, VoiceRecording, VoiceState, VoiceToken,
};

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
    /// Cada cuánto se pregunta quién anda por la voz de los canales, mientras
    /// alguien lo mira.
    pub voice_presence_every: Duration,
    /// Cuánto suena un timbre que sale antes de rendirse. **Tiene que
    /// coincidir con `service.TimbreTTL` del backend**, que pone el
    /// `expiresAt` del que entra.
    pub ring_for: Duration,
    /// Cada cuánto se leen los recursos de un servidor mientras se mira su
    /// pestaña.
    pub stats_every: Duration,
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
            voice_presence_every: Duration::from_secs(15),
            ring_for: Duration::from_secs(20),
            stats_every: Duration::from_secs(5),
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
    pub(crate) notes: Mutex<notes::NotesData>,
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
    pub(crate) notices: Mutex<notice::NoticeData>,
    pub(crate) inbox: Mutex<inbox::InboxData>,
    /// Cuántas veces se ha pedido volver a traer la bandeja: ver
    /// `Inbox::reload_soon`.
    pub(crate) inbox_reloads: std::sync::atomic::AtomicU64,
    pub(crate) requests: Mutex<requests::RequestsData>,
    pub(crate) wall: Mutex<wall::WallData>,
    pub(crate) overview: Mutex<overview::OverviewData>,
    /// Cuántas lecturas del resumen se han pedido: la que llega después de
    /// otra más nueva no pisa a ésta.
    pub(crate) overview_reads: std::sync::atomic::AtomicU64,
    /// Cuántas veces se ha pedido releer el resumen: ver
    /// `Overview::server_event`.
    pub(crate) overview_reloads: std::sync::atomic::AtomicU64,
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
    pub(crate) voice: Mutex<voice::VoiceData>,
    /// Qué entrada a una sala es la vigente: ver `Voice::join_channel`.
    pub(crate) voice_turn: std::sync::atomic::AtomicU64,
    /// Qué ronda de preguntas por la ocupación es la vigente.
    pub(crate) occupancy_turn: std::sync::atomic::AtomicU64,
    /// Qué timbre que sale es el vigente: su reloj sólo lo rinde si lo es.
    pub(crate) ring_turn: std::sync::atomic::AtomicU64,
    pub(crate) platform: Mutex<platform::PlatformData>,
    /// Pone en fila los cambios de Plataforma con su aviso: ver
    /// `Platform::change`.
    pub(crate) platform_order: Mutex<()>,
    /// Los pases para hablar con los agentes, por servidor: ver
    /// `platform::agent`.
    pub(crate) agent_passes: Mutex<platform::agent::AgentPasses>,
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
                notes: Mutex::new(notes::NotesData::default()),
                doc_read_at: Mutex::new(None),
                doc_saves: std::sync::atomic::AtomicU64::new(0),
                task: Mutex::new(TaskState::default()),
                counters: Mutex::new(counters::CountersData::default()),
                counter_reloads: Default::default(),
                comms: Mutex::new(comms::CommsData::default()),
                comms_reloads: std::sync::atomic::AtomicU64::new(0),
                chat: Mutex::new(chat::ChatData::default()),
                notices: Mutex::new(notice::NoticeData::default()),
                inbox: Mutex::new(inbox::InboxData::default()),
                inbox_reloads: std::sync::atomic::AtomicU64::new(0),
                requests: Mutex::new(requests::RequestsData::default()),
                wall: Mutex::new(wall::WallData::default()),
                overview: Mutex::new(overview::OverviewData::default()),
                overview_reads: std::sync::atomic::AtomicU64::new(0),
                overview_reloads: std::sync::atomic::AtomicU64::new(0),
                chat_turn: std::sync::atomic::AtomicU64::new(0),
                chat_reloads: std::sync::atomic::AtomicU64::new(0),
                chat_reads: std::sync::atomic::AtomicU64::new(0),
                connection: Mutex::new(ConnectionState::default()),
                stream_turn: std::sync::atomic::AtomicU64::new(0),
                my_work_reloads: std::sync::atomic::AtomicU64::new(0),
                voice: Mutex::new(voice::VoiceData::default()),
                voice_turn: std::sync::atomic::AtomicU64::new(0),
                occupancy_turn: std::sync::atomic::AtomicU64::new(0),
                ring_turn: std::sync::atomic::AtomicU64::new(0),
                platform: Mutex::new(platform::PlatformData::default()),
                platform_order: Mutex::new(()),
                agent_passes: Mutex::new(platform::agent::AgentPasses::default()),
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
