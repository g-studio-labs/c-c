//! La app por dentro: el riel de módulos, el panel con la organización y el
//! árbol de espacios, y el sitio de la vista.
//!
//! Qué filas tiene el árbol, cuáles van abiertas, qué hace un clic y qué se
//! trae al entrar lo dice el core. Aquí se pinta y se le pasa cada gesto.

use cac_core::{count_badge, counted_label, Locale, NodeAction, NodeKind, ThemePreference, TreeEditError, TreeRow, BELL_CAP, RAIL_CAP};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Side, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::chat::ChatView;
use crate::comms::CommsView;
use crate::connect::{ConnectClosed, ConnectView};
use crate::theme::Leading as _;
use crate::docs::DocsView;
use crate::lang::{locale, tr};
use crate::mywork::MyWorkView;
use crate::task::TaskView;

/// Las medidas de la app de siempre: el riel y el panel.
const RAIL_WIDTH: f32 = 48.;
/// El puesto de cada módulo en el riel, para ⌘1–⌘9 (`AppRail.tsx`): el
/// 2 es Docs, que aún no está en esta app.
const RAIL_WORK: usize = 1;
const RAIL_COMMS: usize = 3;
const RAIL_PLATFORM: usize = 4;
const PANEL_WIDTH: f32 = 256.;
const ROW_HEIGHT: f32 = 28.;
/// Una fila del panel (`PanelRow.tsx`: `text-sm` con `py-1.5`).
pub(crate) const PANEL_ROW_HEIGHT: f32 = 32.;
/// El lado de un botón del riel.
const RAIL_BUTTON: f32 = 36.;
/// El ancho del menú de la cuenta: el de la app de siempre (`w-64`).
const ACCOUNT_MENU_WIDTH: f32 = 256.;
/// Lo que entra una fila por cada nivel que cuelga: el `ml-4` de
/// `SpacesNavigator.tsx`.
pub(crate) const INDENT: f32 = 16.;

/// `capitalize` de CSS: cada palabra con su primera letra en mayúscula.
fn capitalized(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for letter in text.chars() {
        if start && letter.is_alphabetic() {
            out.extend(letter.to_uppercase());
        } else {
            out.push(letter);
        }
        start = letter.is_whitespace();
    }
    out
}

/// El número que lleva un botón del riel: cuánto, y si es un «míralo cuando
/// puedas» (ámbar) en vez de algo sin leer (el color de acento).
struct RailCount {
    id: &'static str,
    count: Option<i64>,
    warning: bool,
}

/// A qué se le está poniendo nombre en el árbol.
#[derive(Clone, PartialEq)]
enum Naming {
    /// Un espacio nuevo, al final del árbol.
    NewSpace,
    /// Una carpeta o una lista nueva, dentro de un espacio o de una carpeta.
    NewChild { parent: NodeKind, parent_id: String, child: NodeKind },
    /// Un nodo que ya existe: el campo va en su fila, con su nombre.
    Rename { kind: NodeKind, id: String },
}

/// El campo en el que se escribe un nombre, en el árbol, donde la cosa va a
/// estar: el `InlineName.tsx` de la app de siempre, que sustituyó a un
/// diálogo (nombrar una lista son tres palabras, y cinco listas seguidas
/// eran cinco diálogos).
struct NameField {
    naming: Naming,
    input: Entity<InputState>,
    /// Esperando al servidor: otro Enter no manda otra vez.
    busy: bool,
    /// Por qué no se guardó, dicho bajo el campo. Lo escrito se queda.
    problem: Option<String>,
    _on_enter: Subscription,
}

/// Casi sin estado propio: lo que pinta lo lee del core al pintarse, y GPUI
/// repinta una vista cuando cambia algo que leyó. No hace falta suscribirse.
pub struct Workspace {
    pub(crate) my_work: Entity<MyWorkView>,
    docs: Entity<DocsView>,
    pub(crate) task: Entity<TaskView>,
    /// La lista de comunicación: lo que el panel enseña mientras se está en
    /// ese módulo.
    pub(crate) comms: Entity<CommsView>,
    servers: Entity<crate::platform::servers::ServersView>,
    _on_comms: Subscription,
    /// La conversación que se tiene delante.
    pub(crate) chat: Entity<ChatView>,
    /// Lo escrito para filtrar el árbol por título. Es de esta pantalla: qué
    /// casa con ello lo dice el core.
    tree_filter: Entity<InputState>,
    /// El panel fijo al lado del riel. Suelto, no ocupa sitio: se asoma por
    /// encima de la vista al pasar el ratón por el riel, y se va al salir.
    pub(crate) panel_pinned: bool,
    /// Dónde está el ratón mientras el panel está suelto: en el riel o en
    /// el panel asomado. Con cualquiera de los dos, se ve.
    over_rail: bool,
    over_panel: bool,
    /// Dónde está el foco cuando no lo tiene nada: sin nadie enfocado, una
    /// tecla no llega a ningún elemento de la app y Ctrl+B no haría nada
    /// hasta pulsar algo con el ratón.
    focus_anchor: FocusHandle,
    _on_focus_lost: Subscription,
    /// El nombre que se está escribiendo en el árbol, si alguno.
    naming: Option<NameField>,
    /// Por qué no se pudo borrar lo último que se pidió borrar.
    tree_problem: Option<String>,
    /// «Conectar Claude Code»: instalar `cac-mcp` y mantenerlo al día. Vive
    /// con la app y no sólo mientras se ve: es quien sabe si hay una versión
    /// nueva, y eso enciende el punto del menú de la cuenta.
    pub(crate) connect: Entity<ConnectView>,
    connect_open: bool,
    /// El menú de la cuenta, abierto. Lo lleva esta vista y no el menú para
    /// poder cerrarlo al abrir desde él algo que va encima.
    account_menu_open: bool,
    _on_connect: [Subscription; 2],
    /// El panel de la campana.
    pub(crate) inbox: crate::inbox::InboxPanelState,
    /// Mientras se está en comunicación se pregunta quién anda por la voz
    /// de los canales; al irse, se deja de preguntar.
    occupancy: Option<cac_core::OccupancyWatch>,
    /// La sala estaba a la vista en el último pintado: abrirla pliega el
    /// panel y cerrarla lo devuelve (`useEncogerEnLlamada.ts`).
    stage_was_shown: bool,
    /// Los vídeos de la sala: uno por cámara o pantalla encendida mientras la
    /// sala está a la vista, ninguno si no.
    pub(crate) video: Entity<crate::voice_video::VideoWall>,
    /// El panel se plegó por la sala, y nadie lo ha tocado desde entonces:
    /// al cerrarla vuelve. Si se abre a mano durante la llamada, esto deja
    /// de mandar.
    folded_for_call: bool,
    /// Los ajustes de audio de la sala.
    pub(crate) voice_settings: crate::voice_settings::SettingsState,
    /// El timbre: a quién llamar, el aviso de uno nuevo, el reloj.
    pub(crate) ring: crate::voice_ring::RingUi,
    /// El foco del selector de pantalla, que se le da al abrirse.
    picker_focus: FocusHandle,
    was_picking: bool,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Quien entra, entra a trabajar: organizaciones, árbol, tareas y el
        // stream de eventos.
        Backend::run(cx, |core| async move { core.open_workspace().await });
        let placeholder = tr(cx, "nav:panel.filter");
        // Al entrar nada tiene el foco, y lo pierde cualquier campo que deja
        // de pintarse con él puesto (el filtro del árbol al plegar el panel,
        // el título de una tarea al cerrarla): se vuelve aquí.
        let focus_anchor = cx.focus_handle();
        window.focus(&focus_anchor, cx);
        let on_focus_lost = cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus_anchor, cx));
        let connect = cx.new(ConnectView::new);
        let comms = cx.new(|cx| CommsView::new(window, cx));
        // «Nueva conversación» se pinta aquí, encima de todo: al abrirse o
        // cerrarse hay que repintar.
        let on_comms = cx.observe(&comms, |_, _, cx| cx.notify());
        let chat = cx.new(|cx| ChatView::new(window, cx));
        let on_connect = [
            // Lo que sabe cambia desde otro hilo (mirar la máquina, instalar):
            // el punto del menú se repinta con ello.
            cx.observe(&connect, |_, _, cx| cx.notify()),
            cx.subscribe_in(&connect, window, |this, _, _: &ConnectClosed, window, cx| {
                this.connect_open = false;
                window.focus(&this.focus_anchor, cx);
                cx.notify();
            }),
        ];
        Self {
            my_work: cx.new(|cx| MyWorkView::new(window, cx)),
            docs: cx.new(DocsView::new),
            task: cx.new(|cx| TaskView::new(window, cx)),
            comms,
            servers: cx.new(|_| crate::platform::servers::ServersView::new()),
            _on_comms: on_comms,
            chat,
            tree_filter: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            // Como se dejó en esta máquina la última vez.
            panel_pinned: Backend::core(cx).panel_pinned(),
            over_rail: false,
            over_panel: false,
            focus_anchor,
            _on_focus_lost: on_focus_lost,
            naming: None,
            tree_problem: None,
            inbox: Default::default(),
            occupancy: None,
            stage_was_shown: false,
            video: cx.new(crate::voice_video::VideoWall::new),
            folded_for_call: false,
            voice_settings: Default::default(),
            ring: Default::default(),
            picker_focus: cx.focus_handle(),
            was_picking: false,
            connect,
            connect_open: false,
            account_menu_open: false,
            _on_connect: on_connect,
        }
    }

    /// Abre «Conectar Claude Code» encima de todo, y cierra el menú desde el
    /// que se pidió.
    fn open_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.account_menu_open = false;
        self.connect_open = true;
        let focus = self.connect.update(cx, |connect, cx| {
            connect.opened(cx);
            connect.focus_handle(cx)
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Abre el campo para nombrar algo en el árbol, con el foco puesto. Al
    /// renombrar lleva el nombre de ahora, seleccionado: lo normal es
    /// cambiarlo entero. Al crear dentro de un nodo, el nodo se abre para
    /// que se vea dónde nace.
    fn start_naming(&mut self, naming: Naming, window: &mut Window, cx: &mut Context<Self>) {
        let tree = Backend::core(cx).tree();
        let (placeholder, current) = match &naming {
            Naming::NewSpace => ("work:tree.newSpace", None),
            Naming::NewChild { parent_id, child, .. } => {
                tree.open(parent_id);
                (if *child == NodeKind::Folder { "work:tree.newFolder" } else { "work:tree.newList" }, None)
            }
            Naming::Rename { kind, id } => {
                let key = match kind {
                    NodeKind::Space => "work:tree.spaceName",
                    NodeKind::Folder => "work:tree.folderName",
                    NodeKind::List => "work:tree.listName",
                };
                (key, cac_core::node_name(&tree.state().spaces, *kind, id))
            }
        };
        let placeholder = tr(cx, placeholder);
        let renaming = current.is_some();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(current.unwrap_or_default()));
        let on_enter = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_name(window, cx);
            }
        });
        // El foco, ya: el menú desde el que se llega aquí se lo devuelve a
        // quien lo tenía salvo que alguien lo haya movido antes de cerrarse.
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            if renaming {
                input.select_all(window, cx);
            }
        });
        self.naming = Some(NameField { naming, input, busy: false, problem: None, _on_enter: on_enter });
        self.tree_problem = None;
        cx.notify();
    }

    /// Escape: se deja de nombrar, y no se manda nada.
    fn cancel_naming(&mut self, cx: &mut Context<Self>) {
        self.naming = None;
        cx.notify();
    }

    /// Enter: manda el nombre. Qué es un nombre que vale, y a dónde va cada
    /// cosa, lo dice el core.
    ///
    /// Si se guardó: al renombrar el campo se cierra; al crear se queda
    /// abierto y vacío, que lo normal es crear más de una. Si no, lo escrito
    /// se queda y se dice por qué (un nombre en blanco no dice nada: el
    /// campo sigue esperándolo).
    fn commit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.naming.as_mut() else { return };
        if field.busy {
            return;
        }
        let name = field.input.read(cx).value().to_string();
        let naming = field.naming.clone();
        field.busy = true;
        field.problem = None;
        cx.notify();
        let sent = naming.clone();
        let answer = Backend::ask(cx, move |core| async move {
            let tree = core.tree();
            match sent {
                Naming::NewSpace => tree.create_space(&name).await,
                Naming::NewChild { parent, parent_id, child } => tree.create_in(parent, &parent_id, child, &name).await,
                Naming::Rename { kind, id } => tree.rename(kind, &id, &name).await,
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                // Sólo si sigue siendo el mismo campo: pudo cancelarse, o
                // abrirse otro, mientras el servidor contestaba.
                let Some(field) = this.naming.as_mut().filter(|field| field.naming == naming) else { return };
                field.busy = false;
                match result {
                    Ok(()) if matches!(naming, Naming::Rename { .. }) => this.naming = None,
                    Ok(()) => field.input.update(cx, |input, cx| input.set_value("", window, cx)),
                    Err(TreeEditError::EmptyName) => {}
                    Err(error) => {
                        let fallback = if matches!(naming, Naming::NewSpace) { "common:misc.errCreateSpace" } else { "common:misc.errSave" };
                        field.problem = Some(cac_core::tree_edit_phrase(locale(cx), &error, fallback));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Borra un nodo, preguntando antes: la pregunta nombra el nodo y dice
    /// qué se va con él (lo escribe el core). Lo que no se puede borrar se
    /// explica, y entonces no hay nada que confirmar.
    fn ask_delete(&mut self, kind: NodeKind, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(notice) = Backend::core(cx).tree().delete_notice(kind, &id, locale(cx)) else { return };
        self.tree_problem = None;
        cx.notify();
        if !notice.allowed {
            let close = tr(cx, "common:action.close");
            let _ = window.prompt(PromptLevel::Info, &notice.title, Some(&notice.body), &[close.as_str()], cx);
            return;
        }
        let cancel = tr(cx, "common:action.cancel");
        let answer = window.prompt(PromptLevel::Warning, &notice.title, Some(&notice.body), &[notice.confirm.as_str(), cancel.as_str()], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let Ok(deleted) = cx.update(|_, cx| Backend::ask(cx, move |core| async move { core.tree().delete(kind, &id).await })) else { return };
            let Ok(Err(error)) = deleted.await else { return };
            this.update(cx, |this, cx| {
                this.tree_problem = Some(cac_core::tree_edit_phrase(locale(cx), &error, "common:admin.errDelete"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Lo que hace una entrada del menú de una fila.
    fn act_on_node(&mut self, action: NodeAction, kind: NodeKind, id: String, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            NodeAction::OpenDocs => Self::open_docs(id, cx),
            NodeAction::NewFolder => self.start_naming(Naming::NewChild { parent: kind, parent_id: id, child: NodeKind::Folder }, window, cx),
            NodeAction::NewList => self.start_naming(Naming::NewChild { parent: kind, parent_id: id, child: NodeKind::List }, window, cx),
            NodeAction::Rename => self.start_naming(Naming::Rename { kind, id }, window, cx),
            NodeAction::Delete => self.ask_delete(kind, id, window, cx),
        }
    }

    /// El campo del nombre, con la sangría de la fila en la que va
    /// (`InlineName.tsx`: `px-2 py-1` y un campo `h-6`).
    fn render_name_field(&self, field: &NameField, depth: u32, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        div()
            .id("tree-name")
            .test_support()
            .flex()
            .flex_none()
            .flex_col()
            .mx_2()
            .pl(px(8. + INDENT * depth as f32))
            .pr_2()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel_naming(cx);
                    cx.stop_propagation();
                }
            }))
            .child(div().h(px(ROW_HEIGHT)).flex().items_center().child(Input::new(&field.input).id("tree-name-input").small().h_6().disabled(field.busy)))
            .children(field.problem.clone().map(|problem| {
                div().id("tree-name-problem").pb_1().leading_xs().text_color(danger).aria_label(problem.clone()).child(problem).test_support()
            }))
    }

    /// Fija el panel o lo suelta. Al cambiar, el ratón no cuenta como
    /// encima de nada: soltarlo desde su botón lo esconde aunque el puntero
    /// siga ahí, que es lo que se acaba de pedir.
    pub(crate) fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        self.panel_pinned = !self.panel_pinned;
        // Tocado a mano: la sala ya no lo devuelve al cerrarse.
        self.folded_for_call = false;
        // Se recuerda en esta máquina. Si no se pudo guardar, el panel hace
        // igual lo que se le pidió: sólo no se acordará al volver a abrir.
        let _ = Backend::core(cx).set_panel_pinned(self.panel_pinned);
        self.over_rail = false;
        self.over_panel = false;
        cx.notify();
    }

    fn switch_org(org_id: String, cx: &App) {
        Backend::run(cx, move |core| async move {
            core.switch_org(&org_id).await;
        });
    }

    /// Elige el tema: el core lo recuerda y la app se viste ya.
    fn choose_theme(preference: ThemePreference, cx: &mut App) {
        if Backend::core(cx).set_theme(preference).is_ok() {
            crate::theme::apply_preference(cx);
        }
    }

    /// Elige el idioma (`None` es el del sistema). El core lo cambia aquí al
    /// momento y lo guarda en la cuenta; si el servidor no pudo, lo elegido
    /// se queda en esta máquina.
    fn choose_language(preference: Option<Locale>, cx: &App) {
        Backend::run(cx, move |core| async move {
            let _ = core.set_locale(preference).await;
        });
    }

    pub(crate) fn log_out(cx: &App) {
        Backend::run(cx, |core| async move { core.logout().await });
    }

    /// Una fila de opciones del menú de la cuenta: su nombre y, debajo, las
    /// tres a la vez. Con un botón que rota no se distingue «auto, ahora en
    /// oscuro» de «oscuro», y son respuestas distintas.
    fn options_row(
        id: &'static str,
        icon: Lucide,
        label: String,
        options: Vec<(String, bool, Box<dyn Fn(&mut App)>)>,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, primary, on_primary, hover) = (theme.muted_foreground, theme.primary, theme.primary_foreground, theme.accent);
        div()
            .id(id)
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_1p5()
            .child(div().flex().items_center().gap_2().leading_xs().text_color(muted).child(Icon::new(icon).size_3p5()).child(label))
            .child(div().flex().gap_0p5().children(options.into_iter().enumerate().map(|(at, (label, active, choose))| {
                div()
                    .id(SharedString::from(format!("{id}-{at}")))
                    .test_support()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .px_1p5()
                    .py_1()
                    .text_size(px(11.))
                    .cursor_pointer()
                    .when(active, |this| this.bg(primary).text_color(on_primary))
                    .when(!active, |this| this.hover(move |style| style.bg(hover)))
                    .on_click(move |_, _, cx| choose(cx))
                    .child(label)
            })))
    }

    /// Una entrada del menú de la cuenta: icono y nombre, apagada hasta que
    /// el ratón pasa.
    fn account_row(id: &'static str, icon: impl Into<Icon>, label: String, danger: bool, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        // «Salir» va en rojo y se queda en rojo bajo el ratón.
        let (muted, hover, foreground) = if danger {
            (theme.danger, theme.accent, theme.danger)
        } else {
            (theme.muted_foreground, theme.accent, theme.foreground)
        };
        div()
            .id(id)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(4.))
            .px_2()
            .py_1p5()
            .leading_xs()
            .text_color(muted)
            .hover(move |style| style.bg(hover).text_color(foreground))
            .child(icon.into().size_3p5().flex_none())
            .child(label)
    }

    /// Una entrada que la app de siempre tiene y ésta todavía no: se pinta
    /// igual, y al pasar el ratón dice que aún no está en vez de callar.
    fn pending_row(id: &'static str, icon: impl Into<Icon>, label: String, cx: &App) -> impl IntoElement {
        let not_yet = tr(cx, "common:misc.notInThisApp");
        Self::account_row(id, icon, label, false, cx).tooltip(move |window, cx| Tooltip::new(not_yet.clone()).build(window, cx)).test_support()
    }

    /// El avatar de quien está dentro, al pie del riel, con el punto del
    /// flujo en vivo y su menú: quién es, tema, idioma, lo suyo y salir.
    /// Las medidas son las de `AccountMenu.tsx`.
    fn render_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (sidebar, success, danger, muted, border, popover, primary) =
            (theme.sidebar, theme.success, theme.danger, theme.muted_foreground, theme.border, theme.popover, theme.primary);
        let mono = theme.mono_font_family.clone();
        let snapshot = Backend::snapshot(cx).read(cx);
        // Rojo sólo cuando se cayó: mientras conecta al arrancar no es un
        // corte, y un punto que parpadea en rojo en cada entrada es ruido.
        let live = snapshot.connection.status != cac_core::Connection::Down;
        let session = snapshot.session.clone();
        let (name, email, initials, tint) = session
            .map(|session| {
                let initials = cac_core::initials(session.name.as_deref(), &session.username);
                // Su color sale de su usuario, como en todas partes: el cian
                // del acento se queda para lo que se pulsa.
                let tint = crate::theme::space_color("", cac_core::hue_index(&session.username), cx);
                let email = session.email.filter(|email| !email.trim().is_empty());
                (session.name.filter(|name| !name.trim().is_empty()).unwrap_or(session.username), email, initials, tint)
            })
            .unwrap_or_else(|| (String::new(), None, String::new(), primary));
        let face = move |size: f32| {
            div()
                .size(px(size))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(tint.opacity(0.15))
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tint)
                .child(initials.clone())
        };
        let in_menu = face.clone();
        let live_hint = tr(cx, if live { "common:misc.receivingLive" } else { "common:misc.notReceivingLive" });
        let trigger = Button::new("account-menu").ghost().p_0().size(px(RAIL_BUTTON)).rounded(px(10.)).tooltip(tr(cx, "nav:rail.account")).child(
            div().relative().size(px(RAIL_BUTTON)).flex().items_center().justify_center().child(face(24.)).child(
                // Pegado al avatar: es la señal de que la app sigue
                // recibiendo. El borde del color del riel lo despega de él.
                div()
                    .id("account-live")
                    .test_support()
                    .absolute()
                    .bottom(px(2.))
                    .right(px(2.))
                    .size(px(12.))
                    .rounded_full()
                    .border_2()
                    .border_color(sidebar)
                    .bg(if live { success } else { danger })
                    .tooltip(move |window, cx| Tooltip::new(live_hint.clone()).build(window, cx)),
            ),
        );
        // A la derecha del avatar, que es donde hay sitio; la ventana lo
        // sube lo que no cabe hacia abajo.
        let update_ready = self.connect.read(cx).update_available();
        let this = cx.entity().downgrade();
        let opener = this.clone();
        let menu = Popover::new("account-popover")
            .appearance(false)
            .anchor(Anchor::LeftCenter)
            .offset(px(8.))
            .open(self.account_menu_open)
            .on_open_change(move |open, _, cx| {
                this.update(cx, |this, cx| {
                    this.account_menu_open = *open;
                    cx.notify();
                })
                .ok();
            })
            .trigger(trigger);
        menu.content(move |_, _, cx| {
            let opener = opener.clone();
            let inbox_opener = opener.clone();
            let update_hint = tr(cx, "nav:mcp.updateAvailable");
            let here = locale(cx);
            let theme_chosen = Backend::core(cx).theme();
            let themes = [
                (ThemePreference::System, "nav:account.themeAuto"),
                (ThemePreference::Light, "nav:account.themeLight"),
                (ThemePreference::Dark, "nav:account.themeDark"),
            ]
            .into_iter()
            .map(|(preference, key)| {
                let choose: Box<dyn Fn(&mut App)> = Box::new(move |cx| Self::choose_theme(preference, cx));
                (cac_core::text(here, key), theme_chosen == preference, choose)
            })
            .collect();
            let language_chosen = Backend::core(cx).locale_preference();
            let languages = [
                (None, "nav:account.languageAuto"),
                (Some(Locale::En), "common:language.en"),
                (Some(Locale::Es), "common:language.es"),
            ]
            .into_iter()
            .map(|(preference, key)| {
                let choose: Box<dyn Fn(&mut App)> = Box::new(move |cx| Self::choose_language(preference, cx));
                (cac_core::text(here, key), language_chosen == preference, choose)
            })
            .collect();
            let rule = move || div().my_1().h(px(1.)).bg(border);
            let not_yet = tr(cx, "common:misc.notInThisApp");
            div()
                .id("account-items")
                .w(px(ACCOUNT_MENU_WIDTH))
                .flex()
                .flex_col()
                .p_1()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .bg(popover)
                .shadow_lg()
                .child(
                    div().flex().items_center().gap_2().px_2().py_1p5().child(in_menu(28.)).child(
                        div()
                            .min_w_0()
                            .child(div().truncate().leading_xs().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                            .children(email.clone().map(|email| div().truncate().text_size(px(11.)).text_color(muted).child(email))),
                    ),
                )
                .child(rule())
                .child(Self::options_row("account-theme", Lucide::Moon, cac_core::text(here, "nav:account.theme"), themes, cx))
                .child(Self::options_row("account-language", Lucide::Languages, cac_core::text(here, "nav:account.language"), languages, cx))
                .child(Self::pending_row("account-profile", Lucide::UserRound, tr(cx, "nav:profile.title"), cx))
                .child(Self::pending_row("account-password", Lucide::KeyRound, tr(cx, "nav:account.changePassword"), cx))
                .child(
                    Self::account_row("account-connect", Lucide::Bot, tr(cx, "nav:account.connectMcp"), false, cx)
                        .cursor_pointer()
                        // Hay un `cac-mcp` más nuevo que el instalado: el
                        // punto lo dice sin abrir nada.
                        .when(update_ready, |row| {
                            row.child(div().flex_1()).child(
                                div()
                                    .id("account-connect-update")
                                    .flex_none()
                                    .size(px(8.))
                                    .rounded_full()
                                    .bg(primary)
                                    .tooltip(move |window, cx| Tooltip::new(update_hint.clone()).build(window, cx))
                                    .test_support(),
                            )
                        })
                        .on_click(move |_, window, cx| {
                            opener.update(cx, |this, cx| this.open_connect(window, cx)).ok();
                        })
                        .test_support(),
                )
                .child({
                    // El mismo panel que la campana.
                    let opener = inbox_opener.clone();
                    Self::account_row("account-notifications", Lucide::Bell, tr(cx, "nav:account.notifications"), false, cx)
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            opener
                                .update(cx, |this, cx| {
                                    this.account_menu_open = false;
                                    if !this.inbox.open {
                                        this.toggle_inbox(cx);
                                    }
                                })
                                .ok();
                        })
                        .test_support()
                })
                .child(rule())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1p5()
                        .leading_xs()
                        .text_color(muted)
                        .child(Icon::new(Lucide::CircleCheck).size_3p5().flex_none().text_color(success))
                        .child(div().font_family(mono.clone()).child(concat!("v", env!("CARGO_PKG_VERSION"))))
                        .child(div().flex_1())
                        .child(
                            // Esta app todavía no se actualiza sola: el
                            // enlace está donde va a estar, y lo dice.
                            div()
                                .id("account-check")
                                .test_support()
                                .text_size(px(11.))
                                .text_color(primary)
                                .child(tr(cx, "common:misc.check"))
                                .tooltip(move |window, cx| Tooltip::new(not_yet.clone()).build(window, cx)),
                        ),
                )
                .child(
                    Self::account_row("account-log-out", Lucide::LogOut, tr(cx, "nav:account.logOut"), true, cx)
                        .cursor_pointer()
                        .on_click(|_, _, cx| Self::log_out(cx))
                        .test_support(),
                )
        })
    }

    /// Qué hace el clic (acotar las tareas o abrir la documentación) lo
    /// decide el core con lo que se esté mirando.
    fn click_row(row: &TreeRow, cx: &App) {
        let id = row.id.clone();
        Backend::run(cx, move |core| async move {
            core.click_node(&id).await;
        });
    }

    /// La documentación de un nodo, se esté mirando lo que se esté mirando.
    pub(crate) fn open_docs(id: String, cx: &App) {
        Backend::run(cx, move |core| async move { core.docs().open(&id).await });
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        div()
            .id("connection-banner")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(danger.opacity(0.3))
            .bg(danger.opacity(0.1))
            .leading_sm()
            .child(Icon::new(Lucide::WifiOff).text_color(danger))
            .child(div().font_weight(FontWeight::MEDIUM).text_color(danger).child(tr(cx, "common:last.liveUpdatesStopped")))
            .child(div().flex_1())
            .child(
                Button::new("reconnect")
                    .outline()
                    .small()
                    .icon(Icon::new(Lucide::RefreshCw))
                    .label(tr(cx, "common:last.reconnect"))
                    .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.open_workspace().await })),
            )
    }

    /// Un botón del riel, con las medidas de `RailButton` en `AppRail.tsx`:
    /// el activo lleva fondo, el color de acento y una raya a su izquierda.
    fn rail_button(id: &'static str, icon: Lucide, label: String, active: bool, cx: &App) -> Stateful<Div> {
        Self::rail_button_keyed(id, icon, label, None, active, cx)
    }

    /// Con su puesto en el riel, el tooltip dice el atajo que lo pulsa.
    fn rail_button_keyed(id: &'static str, icon: Lucide, label: String, slot: Option<usize>, active: bool, cx: &App) -> Stateful<Div> {
        let label = match slot {
            Some(slot) => cac_core::shortcut_text(&cac_core::text_with(locale(cx), "nav:rail.shortcut", &[("name", &label), ("keys", &format!("⌘{slot}"))]), cx.platform_info().is_mac()),
            None => label,
        };
        let theme = cx.theme();
        let (accent, primary, muted, foreground) = (theme.sidebar_accent, theme.sidebar_primary, theme.muted_foreground, theme.foreground);
        div()
            .id(id)
            .relative()
            .size(px(RAIL_BUTTON))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .text_color(if active { primary } else { muted })
            .when(active, |this| {
                this.bg(accent).child(div().absolute().left(px(-6.)).top_2().bottom_2().w(px(2.)).rounded_full().bg(primary))
            })
            .when(!active, |this| this.hover(move |style| style.bg(accent).text_color(foreground)))
            .child(Icon::new(icon).size_5())
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
    }

    /// El número de un botón del riel, en su esquina: el `span` de
    /// `AppRail.tsx` (`absolute -right-0.5 -top-0.5 min-w-4 rounded-full px-1
    /// text-[9px] font-medium leading-4 ring-2 ring-sidebar`). El aro es del
    /// color del riel: separa el número del icono que pisa. Qué se escribe
    /// —nada, el número o «99+»— lo dice el core.
    fn rail_badge(badge: RailCount, cx: &App) -> Option<impl IntoElement> {
        let text = count_badge(badge.count, RAIL_CAP)?;
        let theme = cx.theme();
        let (fill, ink) = if badge.warning { (theme.warning, black()) } else { (theme.primary, theme.primary_foreground) };
        let ring = BoxShadow { color: theme.sidebar, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false };
        Some(
            div()
                .id(badge.id)
                .absolute()
                .right(px(-2.))
                .top(px(-2.))
                .min_w_4()
                .flex()
                .justify_center()
                .rounded_full()
                .px_1()
                .text_size(px(9.))
                .font_weight(FontWeight::MEDIUM)
                .line_height(px(16.))
                .bg(fill)
                .text_color(ink)
                .shadow(vec![ring])
                .aria_label(text.clone())
                .child(text)
                .test_support(),
        )
    }

    /// Un módulo que la app de siempre tiene y ésta todavía no: está en su
    /// sitio y lo dice al pasar el ratón. Si tiene algo pendiente lo cuenta,
    /// en su esquina y en lo que dice: «Comunicación (3)».
    fn pending_module(id: &'static str, icon: Lucide, label_key: &str, count: Option<RailCount>, cx: &App) -> impl IntoElement {
        let pending = count.as_ref().and_then(|count| count.count);
        let label = format!("{} · {}", counted_label(&tr(cx, label_key), pending), tr(cx, "common:misc.notInThisApp"));
        Self::rail_button(id, icon, label.clone(), false, cx)
            .aria_label(label)
            .children(count.and_then(|count| Self::rail_badge(count, cx)))
            .test_support()
    }

    /// Una fila del panel que la app de siempre tiene y ésta todavía no.
    pub(crate) fn pending_panel_row(id: &'static str, icon: Lucide, label_key: &str, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, foreground) = (theme.muted_foreground, theme.sidebar_accent.opacity(0.5), theme.sidebar_foreground);
        let not_yet = tr(cx, "common:misc.notInThisApp");
        div()
            .id(id)
            .h(px(PANEL_ROW_HEIGHT))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_1()
            .mx_1()
            .rounded(px(8.))
            .leading_sm()
            .text_color(muted)
            .hover(move |style| style.bg(hover).text_color(foreground))
            .child(Icon::new(icon).size_4())
            .child(tr(cx, label_key))
            .tooltip(move |window, cx| Tooltip::new(not_yet.clone()).build(window, cx))
            .test_support()
    }

    /// Un botón pequeño de cabecera que todavía no hace nada aquí: dice qué
    /// es y que aún no está.
    fn pending_tool(id: &'static str, icon: Lucide, label_key: &str, cx: &App) -> impl IntoElement {
        let label = format!("{} · {}", tr(cx, label_key), tr(cx, "common:misc.notInThisApp"));
        Self::header_tool(id, icon, label, cx).test_support()
    }

    /// Un botón pequeño de cabecera: su icono y, al pasar el ratón, qué es.
    fn header_tool(id: &'static str, icon: Lucide, label: String, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let (muted, hover, foreground) = (theme.muted_foreground, theme.sidebar_accent, theme.foreground);
        div()
            .id(id)
            .size(px(24.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .text_color(muted)
            .hover(move |style| style.bg(hover).text_color(foreground))
            .child(Icon::new(icon).size_3p5())
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
    }

    /// La barra de arriba de la app de siempre: el buscador en el centro y
    /// la campana a la derecha (`AppLayout.tsx`). El buscador todavía no
    /// abre nada en esta app, y lo dice al pasar el ratón; la campana cuenta
    /// lo que hay sin leer y abre su panel (`inbox.rs`).
    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, surface, hover, foreground) = (theme.border, theme.muted_foreground, theme.muted, theme.accent, theme.foreground);
        let (primary, on_primary) = (theme.primary, theme.primary_foreground);
        let mono = theme.mono_font_family.clone();
        let not_yet = tr(cx, "common:misc.notInThisApp");
        let unread = Backend::snapshot(cx).read(cx).counters.notifications;
        let bell_hint = counted_label(&tr(cx, "common:last.notifications"), unread);
        let inbox_open = self.inbox.open;
        // El `span` de la campana en `AppLayout.tsx`: `absolute -right-0.5
        // -top-0.5 flex min-w-4 items-center justify-center rounded-full
        // bg-primary px-1 text-[10px] font-medium text-primary-foreground`.
        let bell_count = count_badge(unread, BELL_CAP).map(|text| {
            div()
                .id("bell-count")
                .absolute()
                .right(px(-2.))
                .top(px(-2.))
                .min_w_4()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(primary)
                .px_1()
                .text_size(px(10.))
                .line_height(px(15.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(on_primary)
                .aria_label(text.clone())
                .child(text)
                .test_support()
        });
        div()
            .id("top-bar")
            .h(px(48.))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_4()
            .border_b_1()
            .border_color(border)
            .child(div().flex_1())
            .child(
                div()
                    .id("global-search")
                    .w_full()
                    .max_w(px(448.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(surface.opacity(0.3))
                    .leading_xs()
                    .text_color(muted)
                    .hover(move |style| style.bg(surface.opacity(0.6)))
                    .child(Icon::new(IconName::Search).size_3p5().flex_none())
                    .child(div().flex_1().min_w_0().truncate().child(tr(cx, "common:palette.searchPlaceholder")))
                    .child(div().font_family(mono).text_size(px(10.)).opacity(0.7).child("⌘K"))
                    .tooltip(move |window, cx| Tooltip::new(not_yet.clone()).build(window, cx))
                    .test_support(),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("notifications-bell")
                    .relative()
                    .ml_2()
                    .p_1p5()
                    .flex_none()
                    .rounded(px(8.))
                    .text_color(muted)
                    .hover(move |style| style.bg(hover).text_color(foreground))
                    .cursor_pointer()
                    .child(Icon::new(Lucide::Bell).small())
                    .children(bell_count)
                    .aria_label(bell_hint.clone())
                    // Con el panel abierto el globo de ayuda sobra, y se
                    // quedaba encima de su cabecera tapando «Marcar todo leído».
                    .when(!inbox_open, |this| this.tooltip(move |window, cx| Tooltip::new(bell_hint.clone()).build(window, cx)))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_inbox(cx)))
                    .test_support(),
            )
            .test_support()
    }

    /// Pulsar el puesto `slot` del riel, con el ratón o con ⌘N: el módulo
    /// en el que se está fija el panel o lo suelta; otro lleva a él.
    fn press_rail(&mut self, slot: usize, cx: &mut Context<Self>) {
        let (on_comms, on_platform) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            (snapshot.comms.on_screen, snapshot.platform.on_screen)
        };
        let here = match slot {
            RAIL_WORK => !on_comms && !on_platform,
            RAIL_COMMS => on_comms,
            RAIL_PLATFORM => on_platform,
            _ => return,
        };
        if here {
            self.toggle_panel(cx);
            return;
        }
        let core = Backend::core(cx);
        match slot {
            RAIL_WORK => {
                core.comms().leave();
                core.platform().leave();
            }
            RAIL_COMMS => core.comms().enter(),
            _ => {
                core.platform().enter();
                // Salir paró el sondeo de recursos; si se vuelve a esa
                // pestaña, sigue.
                Backend::run(cx, |core| async move {
                    core.platform().start_stats();
                    core.platform().refresh().await
                });
            }
        }
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (sidebar, border) = (theme.sidebar, theme.sidebar_border);
        let counters = Backend::snapshot(cx).read(cx).counters.clone();
        let comms = RailCount { id: "rail-comms-count", count: counters.comms, warning: false };
        // En ámbar: un servidor que no está en línea es un «míralo cuando
        // puedas», no algo sin leer.
        let servers = RailCount { id: "rail-platform-count", count: counters.servers, warning: true };
        let invitations = RailCount { id: "rail-organization-count", count: counters.invitations, warning: false };
        // En cuál de los módulos con pantalla se está lo dice el core.
        let (on_comms, on_platform) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            (snapshot.comms.on_screen, snapshot.platform.on_screen)
        };
        let on_work = !on_comms && !on_platform;
        let platform_label = counted_label(&tr(cx, "nav:module.platform"), servers.count);
        let comms_label = counted_label(&tr(cx, "nav:module.comms"), comms.count);
        div()
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .items_center()
            .gap_1()
            .py_2()
            .bg(sidebar)
            .border_r_1()
            .border_color(border)
            .child(div().id("rail-brand").test_support().mb_2().child(crate::brand::brand_mark(20., sidebar, cx)))
            // Pulsar el módulo en el que se está fija el panel o lo suelta;
            // pulsar otro lleva a él, con su panel.
            .child(
                Self::rail_button_keyed("rail-work", Lucide::Briefcase, tr(cx, "nav:module.work"), Some(RAIL_WORK), on_work, cx)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.press_rail(RAIL_WORK, cx)))
                    .test_support(),
            )
            .child(Self::pending_module("rail-docs", Lucide::BookOpen, "nav:module.docs", None, cx))
            // Con lo que tiene sin leer, en su esquina y en lo que dice.
            .child(
                Self::rail_button_keyed("rail-comms", Lucide::MessagesSquare, comms_label.clone(), Some(RAIL_COMMS), on_comms, cx)
                    .aria_label(comms_label)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.press_rail(RAIL_COMMS, cx)))
                    .children(Self::rail_badge(comms, cx))
                    .test_support(),
            )
            // Con los servidores que no están en línea, en ámbar. Entrar los
            // vuelve a pedir: su estado no llega por evento.
            .child(
                Self::rail_button_keyed("rail-platform", Lucide::Server, platform_label.clone(), Some(RAIL_PLATFORM), on_platform, cx)
                    .aria_label(platform_label)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.press_rail(RAIL_PLATFORM, cx)))
                    .children(Self::rail_badge(servers, cx))
                    .test_support(),
            )
            .child(div().flex_1())
            // Con el panel plegado, la llamada en curso queda como un punto.
            .when(!self.panel_pinned, |this| this.children(crate::voice::mini_dot(cx)))
            .child(Self::pending_module("rail-organization", Lucide::Building2, "nav:rail.organization", Some(invitations), cx))
            .child(self.render_account(cx))
    }

    /// La organización que se mira: la marca, su nombre y, debajo, quién
    /// eres en ella y cómo es de grande. Pulsarla ofrece las demás.
    fn render_org(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (primary, on_primary, muted) = (theme.primary, theme.primary_foreground, theme.muted_foreground);
        let snapshot = Backend::snapshot(cx).read(cx);
        let orgs = snapshot.orgs.clone();
        let spaces = snapshot.tree.spaces.len();
        let name = orgs.current().map(|org| org.name.clone()).unwrap_or_else(|| tr(cx, "nav:rail.organization"));
        let hint = orgs.current().map(|org| cac_core::org_hint(org, spaces, locale(cx))).filter(|hint| !hint.is_empty());
        let current = orgs.current_org_id.clone();
        let face = div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .size(px(32.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(10.))
                    .bg(primary)
                    .child(crate::brand::brand_mark_in(16., on_primary, primary)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    // `OrgSwitcher.tsx`: `gap-0.5 leading-none`, el nombre a
                    // `text-[12.5px] font-semibold` y debajo, a
                    // `text-[10.5px] capitalize`, quién eres y cómo es de grande.
                    .gap_0p5()
                    .child(
                        div().id("org-name").test_support().w_full().truncate().text_size(px(12.5)).line_height(px(14.)).font_weight(FontWeight::SEMIBOLD).child(name.clone()),
                    )
                    .children(hint.map(|hint| {
                        div().id("org-hint").test_support().w_full().truncate().text_size(px(10.5)).line_height(px(12.)).text_color(muted).child(capitalized(&hint))
                    })),
            )
            // Siempre, también con una sola organización: desde aquí se crea otra.
            .child(Icon::new(Lucide::ChevronsUpDown).size_3p5().flex_none().opacity(0.6));
        let label = tr(cx, "common:misc.organizations");
        Button::new("org-switch").ghost().w_full().h(px(48.)).p_2().child(face).dropdown_menu(move |menu, _, _| {
            let mut menu = menu.min_w(px(224.)).check_side(Side::Right).label(label.clone());
            for org in &orgs.orgs {
                let id = org.id.clone();
                menu = menu.item(
                    PopupMenuItem::new(org.name.clone())
                        .icon(Icon::new(Lucide::Building2))
                        .checked(current.as_deref() == Some(org.id.as_str()))
                        .on_click(move |_, _, cx| Self::switch_org(id.clone(), cx)),
                );
            }
            // Está donde en la app de siempre; aquí todavía no crea nada, y
            // lo dice al pasar el ratón.
            menu.separator().item(PopupMenuItem::element(|_, cx| {
                let not_yet = tr(cx, "common:misc.notInThisApp");
                div()
                    .id("org-create")
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(IconName::Plus).small())
                    .child(tr(cx, "common:misc.createOrg"))
                    .tooltip(move |window, cx| Tooltip::new(not_yet.clone()).build(window, cx))
                    .test_support()
            }))
        })
    }

    /// Una fila del árbol, con las medidas de `SpacesNavigator.tsx`: dentro
    /// del `p-2` del grupo, `px-2 py-1` y `ml-4` por nivel; el chevrón
    /// (`size-3.5` en un espacio, `size-3` en una carpeta), la marca y el
    /// nombre a `gap-1` (`gap-1.5` en una lista), y al final lo que la fila
    /// tiene: el ojo, el documento, lo abierto y el «···», todo pequeño.
    fn render_row(&self, row: TreeRow, first: bool, can_edit: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let danger = theme.danger;
        let (hover, active_bg, muted, foreground, primary) =
            (theme.sidebar_accent.opacity(0.5), theme.sidebar_accent, theme.muted_foreground, theme.sidebar_foreground, theme.primary);
        let chevron = if row.open { IconName::ChevronDown } else { IconName::ChevronRight };
        let toggle_id = row.id.clone();
        let clicked = row.clone();
        let count = (row.kind == NodeKind::List && row.open_count > 0).then(|| row.open_count.to_string());
        let (space, list) = (row.kind == NodeKind::Space, row.kind == NodeKind::List);
        let folder = row.kind == NodeKind::Folder;
        // Qué es cada fila, a la vista: el color de su espacio, una carpeta
        // o una lista.
        let mark = match row.kind {
            NodeKind::Space => {
                div().size(px(10.)).flex_none().rounded_full().bg(crate::theme::space_color(&row.color, row.hue, cx)).into_any_element()
            }
            NodeKind::Folder => Icon::new(Lucide::Folder).size_3p5().flex_none().text_color(muted).into_any_element(),
            NodeKind::List => Icon::new(Lucide::ListChecks).size_3p5().flex_none().text_color(muted).into_any_element(),
        };
        let chevron_size = if space { 14. } else { 12. };
        let menu_label = tr(
            cx,
            match row.kind {
                NodeKind::Space => "work:tree.spaceMenu",
                NodeKind::Folder => "work:tree.folderMenu",
                NodeKind::List => "work:tree.listMenu",
            },
        );
        // Lo que ofrece el menú de este nodo a quien lo abre lo dice el core;
        // aquí cada cosa lleva su texto y su icono, los de los menús de
        // `SpacesNavigator.tsx`.
        let here = locale(cx);
        let delete_key = match row.kind {
            NodeKind::Space => "work:tree.deleteSpace",
            NodeKind::Folder => "work:tree.deleteFolder",
            NodeKind::List => "work:tree.deleteList",
        };
        let actions: Vec<(NodeAction, &'static str, String, Lucide)> = cac_core::node_menu(row.kind, can_edit)
            .into_iter()
            .map(|action| {
                let (slug, key, icon) = match action {
                    NodeAction::OpenDocs => ("docs", "work:tree.openDocs", Lucide::FileText),
                    NodeAction::NewFolder => ("new-folder", "work:tree.newFolder", Lucide::FolderPlus),
                    NodeAction::NewList => ("new-list", "work:tree.newList", Lucide::ListChecks),
                    NodeAction::Rename => ("rename", "work:tree.rename", Lucide::Pencil),
                    NodeAction::Delete => ("delete", delete_key, Lucide::Trash),
                };
                (action, slug, cac_core::text(here, key), icon)
            })
            .collect();
        let (menu_id, menu_kind, workspace) = (row.id.clone(), row.kind, cx.entity().downgrade());
        // Lo ve un cliente: es su canal. En un espacio va delante del nombre;
        // en una lista, detrás.
        let client = row.client.then(|| {
            div().id(SharedString::from(format!("tree-client-{}", row.id))).test_support().flex_none().child(Icon::new(IconName::Eye).size_3().text_color(primary))
        });
        let (client_before, client_after) = if space { (client, None) } else { (None, client) };

        div()
            .id(SharedString::from(format!("tree-{}", row.id)))
            .test_support()
            .h(px(ROW_HEIGHT))
            .flex()
            .flex_none()
            .items_center()
            .map(|this| if list { this.gap_1p5().rounded(px(4.)) } else { this.gap_1() })
            .mx_2()
            .pl(px(8. + INDENT * row.depth as f32))
            .pr_2()
            // Cada espacio deja 2 px tras lo suyo (`mb-0.5`).
            .when(space && !first, |this| this.mt(px(2.)))
            .leading_sm()
            .text_color(foreground)
            .when(space, |this| this.font_weight(FontWeight::MEDIUM))
            // El espacio al que se acota se dice con su nombre en el color de
            // la marca; la lista, con la fila entera, y la carpeta igual que
            // la lista (command-and-control-382): no tiene marca de color
            // propia con la que decirlo.
            .when(space && row.active, |this| this.text_color(primary))
            .when(list && row.active, |this| this.bg(active_bg))
            .when(folder && row.active, |this| this.rounded(px(4.)).bg(active_bg))
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .on_click(move |_, _, cx| Self::click_row(&clicked, cx))
            // Una lista no se abre: su icono va donde el chevrón de las demás.
            .when(!list, |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!("tree-toggle-{}", row.id)))
                        .test_support()
                        .size(px(chevron_size))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .when(row.expandable, |this| {
                            this.child(Icon::new(chevron).size(px(chevron_size)).text_color(muted)).on_click(move |_, _, cx| {
                                // El chevrón abre y cierra; no es un clic en la fila.
                                cx.stop_propagation();
                                Backend::core(cx).tree().toggle(&toggle_id);
                            })
                        }),
                )
            })
            .child(mark)
            .children(client_before)
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(row.name))
            .children(client_after)
            // Tiene documentación escrita.
            .when(row.has_doc, |this| {
                this.child(div().id(SharedString::from(format!("tree-doc-{}", row.id))).test_support().flex_none().child(Icon::new(Lucide::FileText).size_3().text_color(muted)))
            })
            .children(count.map(|count| div().flex_none().leading_xs().font_weight(FontWeight::NORMAL).text_color(muted).child(count)))
            // El menú de la fila. Pulsar un nodo —espacio, carpeta o lista—
            // acota sus tareas, así que su documentación se abre desde aquí
            // (command-and-control-372). Abrirlo no es un clic en la fila: el
            // desplegable del kit ya se queda con él (medido en
            // `popover.rs`); si deja de hacerlo, lo dice la prueba del menú.
            // El «···» es sólo su icono (`size-3.5`), sin caja de botón.
            .child(
                div().id(SharedString::from(format!("tree-menu-{}", row.id))).test_support().flex_none().child(
                    Button::new(SharedString::from(format!("tree-more-{}", row.id)))
                        .ghost()
                        .xsmall()
                        .size(px(14.))
                        .p_0()
                        .child(Icon::new(IconName::Ellipsis).size_3p5().text_color(muted))
                        .tooltip(menu_label)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (action, slug, label, icon) in actions.clone() {
                                let (id, workspace) = (menu_id.clone(), workspace.clone());
                                let item_id = SharedString::from(format!("tree-act-{slug}-{id}"));
                                let deletes = action == NodeAction::Delete;
                                menu = menu.item(
                                    PopupMenuItem::element(move |_, _| {
                                        // Borrar va en rojo, como en la app de siempre.
                                        div().id(item_id.clone()).when(deletes, |this| this.text_color(danger)).child(label.clone()).test_support()
                                    })
                                    .icon(Icon::new(icon))
                                    .on_click(move |_, window, cx| {
                                        workspace.update(cx, |this, cx| this.act_on_node(action, menu_kind, id.clone(), window, cx)).ok();
                                    }),
                                );
                            }
                            menu
                        }),
                ),
            )
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (sidebar, border, muted, hover, foreground) =
            (theme.sidebar, theme.sidebar_border, theme.muted_foreground, theme.sidebar_accent, theme.sidebar_foreground);
        let snapshot = Backend::snapshot(cx).read(cx);
        // «Mi trabajo» es la pantalla, esté acotada o no: a qué, lo dice el
        // árbol. Leyendo documentación ya no se está en ella.
        let on_my_work = snapshot.docs.target.is_none();
        // En comunicación el panel es su lista, y en Plataforma sus filas:
        // misma cabecera, otro cuerpo.
        let on_comms = snapshot.comms.on_screen;
        let on_platform = snapshot.platform.on_screen;
        let pinned = self.panel_pinned;
        // El catálogo anuncia el atajo como en un Mac («⌘B»): se dice con
        // la tecla de este sistema.
        let on_a_mac = cx.platform_info().is_mac();
        let toggle_hint = cac_core::shortcut_text(&tr(cx, "nav:rail.collapsePanel"), on_a_mac);
        let query = self.tree_filter.read(cx).value().to_string();
        let rows = Backend::core(cx).tree().rows_matching(&query);
        // A quien sólo puede mirar no se le ofrece cambiar el árbol.
        let can_edit = Backend::core(cx).tree().can_edit();
        let danger = theme.danger;
        let tree_problem = self.tree_problem.clone();
        let new_space = can_edit.then(|| {
            Self::header_tool("spaces-new", Lucide::Plus, tr(cx, "work:tree.newSpace"), cx)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| this.start_naming(Naming::NewSpace, window, cx)))
                .test_support()
        });
        // Dónde va el campo del nombre: en la fila que se renombra; tras lo
        // último que cuelga del nodo en el que se crea; o al final, para un
        // espacio nuevo (y para un nodo que el filtro ha dejado sin fila).
        let place = self.naming.as_ref().map(|field| match &field.naming {
            Naming::NewSpace => (rows.len(), 0, false),
            Naming::Rename { kind, id } => match rows.iter().position(|row| row.kind == *kind && &row.id == id) {
                Some(at) => (at, rows[at].depth, true),
                None => (rows.len(), 0, false),
            },
            Naming::NewChild { parent, parent_id, .. } => match rows.iter().position(|row| row.kind == *parent && &row.id == parent_id) {
                Some(at) => {
                    let depth = rows[at].depth;
                    let end = rows[at + 1..].iter().position(|row| row.depth <= depth).map_or(rows.len(), |after| at + 1 + after);
                    (end, depth + 1, false)
                }
                None => (rows.len(), 0, false),
            },
        });
        let mut tree: Vec<AnyElement> = Vec::with_capacity(rows.len() + 1);
        let count = rows.len();
        for (index, row) in rows.into_iter().enumerate() {
            match (place, self.naming.as_ref()) {
                (Some((at, depth, replaces)), Some(field)) if at == index => {
                    tree.push(self.render_name_field(field, depth, cx).into_any_element());
                    if replaces {
                        continue;
                    }
                }
                _ => {}
            }
            tree.push(self.render_row(row, index == 0, can_edit, cx).into_any_element());
        }
        if let (Some((at, depth, _)), Some(field)) = (place, self.naming.as_ref()) {
            if at >= count {
                tree.push(self.render_name_field(field, depth, cx).into_any_element());
            }
        }
        // Filtrando y sin nada que enseñar se dice: un árbol en blanco se lee
        // como que la organización no tiene espacios.
        let nothing = count == 0 && !query.trim().is_empty();

        let panel = div()
            .w(px(PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .bg(sidebar)
            .text_color(foreground)
            .border_r_1()
            .border_color(border)
            .child(div().p_2().child(self.render_org(cx)))
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl_3()
                    .pr_1()
                    .leading_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(div().flex_1().child(tr(cx, if on_comms { "nav:module.comms" } else if on_platform { "nav:module.platform" } else { "nav:module.work" }).to_uppercase()))
                    .child(
                        Button::new("panel-toggle")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if pinned { Lucide::PanelLeftClose } else { Lucide::PanelLeftOpen }).text_color(muted))
                            .tooltip(toggle_hint)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(cx))),
                    ),
            );
        if on_comms {
            return panel
                .child(div().flex_1().min_h_0().flex().flex_col().child(self.comms.clone()))
                .children(crate::voice::mini(cx))
                .into_any_element();
        }
        if on_platform {
            return panel
                .child(div().id("platform-panel").flex_1().min_h_0().overflow_y_scroll().child(crate::platform::panel::panel(cx)))
                .children(crate::voice::mini(cx))
                .into_any_element();
        }
        panel
            .child(
                div()
                    .id("tree-everything")
                    .test_support()
                    // `PanelRow.tsx`: activa, `bg-accent font-medium`; si
                    // no, apagada hasta que pasa el ratón.
                    .h(px(PANEL_ROW_HEIGHT))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .mx_1()
                    .rounded(px(8.))
                    .leading_sm()
                    .when(on_my_work, |this| this.bg(hover).font_weight(FontWeight::MEDIUM))
                    .when(!on_my_work, |this| this.text_color(muted).hover(move |style| style.bg(hover.opacity(0.5)).text_color(foreground)))
                    .cursor_pointer()
                    .on_click(|_, _, cx| Backend::core(cx).show_all_work())
                    .child(Icon::new(IconName::Inbox).size_4())
                    .child(tr(cx, "nav:item.myWork")),
            )
            .child(Self::pending_panel_row("panel-overview", Lucide::LayoutDashboard, "nav:item.overview", cx))
            .child(Self::pending_panel_row("panel-notes", Lucide::NotebookPen, "nav:item.notes", cx))
            // Filtra lo que ya está en el panel; no es el buscador.
            .child(
                div().mt_1().px_2().py_1().child(
                    Input::new(&self.tree_filter)
                        .id("tree-filter")
                        .small()
                        // `PanelFilter.tsx`: `h-7`, y la lupa a `size-3.5`.
                        .h_7()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size_3p5().text_color(muted)),
                ),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl_4()
                    .pr_4()
                    .mt_2()
                    .leading_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(foreground.opacity(0.7))
                    .child(div().flex_1().child(tr(cx, "nav:panel.spaces")))
                    // Reordenar está donde en la app de siempre, y todavía
                    // no hace nada aquí. El «+» abre el campo del nombre al
                    // final del árbol, que es donde nace un espacio.
                    .child(Self::pending_tool("spaces-rearrange", Lucide::Lock, "work:tree.rearrange", cx))
                    .children(new_space),
            )
            // Un borrado que el servidor rechazó: se dice, y un clic lo quita.
            .children(tree_problem.map(|problem| {
                div()
                    .id("tree-problem")
                    .mx_2()
                    .px_2()
                    .py_1()
                    .leading_xs()
                    .text_color(danger)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tree_problem = None;
                        cx.notify();
                    }))
                    .aria_label(problem.clone())
                    .child(problem)
                    .test_support()
            }))
            .when(nothing, |this| {
                this.child(div().id("tree-no-matches").test_support().px_5().py_3().leading_xs().text_color(muted).child(tr(cx, "nav:panel.noMatches")))
            })
            .child(
                div()
                    .id("tree")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(tree),
            )
            // La llamada en curso, al pie: se queda al navegar.
            .children(crate::voice::mini(cx))
            .into_any_element()
    }
}

impl Workspace {
    /// El selector de pantalla, con el foco al abrirse (Escape lo cierra) y
    /// devuelto a la ventana al cerrarse.
    fn render_screen_picker(&mut self, voice: &cac_core::VoiceState, stage_shown: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sources = voice.picking.as_ref().filter(|_| stage_shown);
        let picking = sources.is_some();
        if picking && !self.was_picking {
            self.picker_focus.focus(window, cx);
        } else if !picking && self.was_picking {
            self.focus_anchor.focus(window, cx);
        }
        self.was_picking = picking;
        sources.map(|sources| crate::voice_screen_picker::screen_picker(sources, &self.picker_focus, window, cx))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (on_comms, stage_shown) = {
            let snapshot = Backend::snapshot(cx).read(cx);
            let voice = &snapshot.voice;
            (snapshot.comms.on_screen, snapshot.comms.on_screen && voice.stage_open && voice.phase != cac_core::VoicePhase::Out)
        };
        // Quién anda por la voz, sólo mientras se mira comunicación.
        if on_comms && self.occupancy.is_none() {
            self.occupancy = Some(Backend::within(cx, |core| core.voice().watch_occupancy()));
        } else if !on_comms {
            self.occupancy = None;
        }
        // Abrir la sala pliega el panel, que roba el ancho que se viene a
        // mirar; cerrarla lo devuelve como estaba. Sin guardarlo: es de esta
        // llamada, no una preferencia.
        if stage_shown && !self.stage_was_shown && self.panel_pinned {
            self.panel_pinned = false;
            self.folded_for_call = true;
        }
        if !stage_shown && self.stage_was_shown && self.folded_for_call {
            self.panel_pinned = true;
            self.folded_for_call = false;
        }
        self.stage_was_shown = stage_shown;
        // Lo que cuelga de la sala se cierra con ella.
        let voice = Backend::snapshot(cx).read(cx).voice.clone();
        if !stage_shown {
            self.voice_settings.open = false;
            self.ring.invite_open = false;
            // Elegir qué compartir sin ver la sala no se queda pendiente
            // para cuando vuelvas.
            if voice.picking.is_some() {
                Backend::core(cx).voice().cancel_pick();
            }
        }
        let screen_picker = self.render_screen_picker(&voice, stage_shown, window, cx);
        self.sync_ring(&voice, cx);
        self.video.update(cx, |wall, cx| wall.sync(&voice, stage_shown, cx));
        let stage: Option<AnyElement> = if stage_shown {
            let settings_button = crate::voice_settings::available(cx).then(|| {
                // Sin el rojo de «activo»: abierto no es silenciado.
                crate::voice::round_button("voice-settings-toggle", Lucide::SlidersHorizontal, tr(cx, "common:voice.settings"), false, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_settings(cx)))
                    .test_support()
                    .into_any_element()
            });
            let extras = crate::voice::StageExtras {
                settings_button,
                settings_panel: self.render_voice_settings(cx),
                invite_button: (voice.space_id.is_some() || voice.meet_id.is_some()).then(|| self.invite_button(cx)),
                invite_panel: self.render_invite(cx),
                ring_row: self.render_ring_row(&voice, cx),
                screen_picker,
                video: Some(self.video.clone()),
            };
            Some(crate::voice::stage(extras, cx))
        } else {
            None
        };
        let incoming = self.render_incoming(&voice, cx);
        let snapshot = Backend::snapshot(cx).read(cx);
        let lasting = snapshot.connection.lasting;
        // Con un documento abierto se lee documentación; si no, las tareas.
        // En comunicación, la conversación —o la sala, si está abierta—;
        // volver a «Trabajo» deja lo que se estuviera mirando allí.
        let main: AnyElement = if let Some(stage) = stage {
            stage
        } else if snapshot.comms.on_screen {
            crate::comms::conversation(&self.chat, cx)
        } else if snapshot.platform.on_screen {
            self.servers.clone().into_any_element()
        } else if snapshot.docs.target.is_some() {
            self.docs.clone().into_any_element()
        } else {
            self.my_work.clone().into_any_element()
        };
        // «Nueva conversación», abierta desde la lista de comunicación.
        let picker = self.comms.read(cx).picker.clone();
        // La tarea abierta, sobre lo que se estuviera mirando.
        let task = snapshot.task.task_id.is_some().then(|| self.task.clone());
        let pinned = self.panel_pinned;
        let peeking = !pinned && (self.over_rail || self.over_panel);
        div()
            .id("open")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            // ⌘B o Ctrl+B pliega el panel y lo vuelve a fijar, como en
            // `app/src/components/ui/sidebar.tsx`. Al subir, no al bajar: lo
            // que tiene el foco lo oye antes, y el editor de markdown se
            // queda con esa tecla para la negrita (`editor.rs` corta ahí la
            // subida). Qué pulsación es la del panel lo dice el core.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                // Escape cierra el panel de la campana, que no tiene el foco.
                if this.inbox.open && event.keystroke.key == "escape" {
                    this.close_inbox(cx);
                    cx.stop_propagation();
                    return;
                }
                let held = event.keystroke.modifiers;
                let held = cac_core::KeysHeld { secondary: held.secondary(cx), shift: held.shift, alt: held.alt };
                if cac_core::folds_panel(&event.keystroke.key, held) {
                    this.toggle_panel(cx);
                    cx.stop_propagation();
                } else if let Some(slot) = cac_core::rail_slot(&event.keystroke.key, held) {
                    this.press_rail(slot, cx);
                    cx.stop_propagation();
                }
            }))
            // Sin tamaño: no se pinta ni se pulsa, sólo guarda el foco.
            .child(div().absolute().size_0().track_focus(&self.focus_anchor))
            .when(lasting, |this| this.child(self.render_banner(cx)))
            .child(
                div()
                    .flex_1()
                    .relative()
                    .flex()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("rail")
                            .test_support()
                            .h_full()
                            .flex_none()
                            .on_hover(cx.listener(|this, over: &bool, _, cx| {
                                this.over_rail = *over;
                                cx.notify();
                            }))
                            .child(self.render_rail(cx)),
                    )
                    .when(pinned, |this| this.child(self.render_panel(cx)))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(self.render_top_bar(cx))
                            .child(div().flex_1().min_h_0().overflow_hidden().child(main)),
                    )
                    // El panel de la campana, por encima de todo lo de abajo.
                    .children(self.render_inbox(cx))
                    // Suelto y con el ratón en el riel o en él: asomado por
                    // encima de la vista, sin empujarla.
                    .when(peeking, |this| {
                        this.child(
                            div()
                                .id("panel-peek")
                                .test_support()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(RAIL_WIDTH))
                                .occlude()
                                .shadow_lg()
                                .on_hover(cx.listener(|this, over: &bool, _, cx| {
                                    this.over_panel = *over;
                                    cx.notify();
                                }))
                                .child(self.render_panel(cx)),
                        )
                    })
                    // La tarea abierta es una página sobre todo lo demás, no
                    // un cajón al lado: lo que tapa no recibe el ratón. El
                    // aviso de conexión sigue a la vista, encima.
                    .children(task.map(|task| div().id("task-over").absolute().inset_0().occlude().child(task)))
                    // «Conectar Claude Code», encima de todo lo demás.
                    .children(self.connect_open.then(|| div().id("connect-over").absolute().inset_0().child(self.connect.clone())))
                    .children(picker.map(|picker| div().id("new-conversation-over").absolute().inset_0().child(picker)))
                    // Quien te llama, encima de todo, estés donde estés.
                    .children(incoming),
            )
    }
}

#[cfg(test)]
mod tests {
    // Sin `use super::*`: ver la nota en `shell.rs`.
    use super::Workspace;
    use crate::backend::Backend;
    use crate::testing::{ok, sign_in, wait_until, FakeServer};
    use cac_core::ScopeKind;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::Mock;

    /// La pestaña Provisioning, en su fichero para no engordar éste.
    mod provision {
        use super::*;
        include!("platform/provision_tests.rs");
    }

    fn org(id: &str, name: &str) -> serde_json::Value {
        json!({
            "id": id, "name": name, "slug": id, "role": "member", "memberCount": 2,
            "createdAt": "2026-06-09T12:00:00Z", "timezone": "",
        })
    }

    fn space(id: &str, name: &str) -> serde_json::Value {
        json!({
            "id": id, "orgId": "o1", "name": name, "color": "#20D9E8",
            "folders": [{
                "id": format!("f-{id}"), "name": "Carpeta", "folders": [],
                "lists": [{ "id": format!("in-{id}"), "name": "En carpeta", "openCount": 2, "taskCount": 2 }],
            }],
            "lists": [{ "id": format!("l-{id}"), "name": "Pendientes", "openCount": 3, "taskCount": 5 }],
            "people": [],
        })
    }

    /// Una tarea como la manda el servidor: la del contrato del core, en
    /// otra lista.
    fn task(id: &str, list: &str) -> serde_json::Value {
        let mut task: serde_json::Value =
            serde_json::from_str(include_str!("../../core/tests/fixtures/open_task.json")).unwrap();
        task["id"] = json!(id);
        task["listId"] = json!(list);
        task["spaceId"] = json!("a");
        task
    }

    /// Un servidor con dos organizaciones; la primera con un espacio y dos
    /// tareas, la segunda con otro.
    fn server() -> FakeServer {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio"), org("o2", "Cliente")]))));
        server.mount(
            get("/api/v1/task-spaces/").and(query_param("orgId", "o1")).respond_with(ok(json!([space("a", "Producto")]))),
        );
        server.mount(
            get("/api/v1/task-spaces/").and(query_param("orgId", "o2")).respond_with(ok(json!([space("z", "Soporte")]))),
        );
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(
            get("/api/v1/tasks/")
                .and(query_param("orgId", "o1"))
                .respond_with(ok(json!([task("1", "l-a"), task("2", "in-a")]))),
        );
        server.mount(get("/api/v1/tasks/").and(query_param("orgId", "o2")).respond_with(ok(json!([]))));
        server
    }

    fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
        open_sized(1100., cx)
    }

    fn open_sized(width: f32, cx: &mut TestAppContext) -> AnyWindowHandle {
        open_with_view(width, cx).0
    }

    fn open_with_view(width: f32, cx: &mut TestAppContext) -> (AnyWindowHandle, gpui_kit::Entity<Workspace>) {
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(width), px(760.)),
                })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window")
        })
    }

    fn on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn click(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    /// Baja la rueda sobre ese elemento, que tiene su propio scroll.
    fn wheel_in(window: AnyWindowHandle, id: &str, down: f32, cx: &mut TestAppContext) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.scroll(id, gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-down))), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Baja por ese scroll de poco en poco hasta que el elemento se ve.
    fn reveal(window: AnyWindowHandle, scroll: &str, id: &str, cx: &mut TestAppContext) {
        let target = gpui_kit::SharedString::from(id.to_string());
        for _ in 0..40 {
            let seen = cx
                .update_window(window, |_, window, cx| {
                    window.render_frame(cx);
                    window.try_find(target.clone()).is_some_and(|found| found.visible())
                })
                .unwrap();
            if seen {
                // Se ve el borde: un poco más, para que el centro (donde
                // cae el clic) también esté dentro.
                wheel_in(window, scroll, 120., cx);
                return;
            }
            wheel_in(window, scroll, 80., cx);
        }
        panic!("no se llega a ver {id}");
    }

    /// Deja correr lo que un clic haya puesto en marcha.
    fn settle_frames(window: AnyWindowHandle, cx: &mut TestAppContext) {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(150));
        cx.run_until_parked();
        on_screen(window, "open", cx);
    }

    fn scope(cx: &mut TestAppContext) -> Option<(ScopeKind, String)> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).my_work.scope.as_ref().map(|scope| (scope.kind, scope.id.clone())))
    }

    /// Entrar trae la organización y su árbol, y los pinta.
    ///
    /// Roturas que atrapa: una app que abre con el panel vacío hasta que se
    /// pulsa algo; pintar lo que hay dentro de una carpeta cerrada.
    #[gpui_kit::test]
    fn entering_paints_the_organization_and_its_tree(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);

        wait_until(cx, "el árbol llega a la pantalla", |cx| on_screen(window, "tree-l-a", cx));

        assert!(on_screen(window, "tree-a", cx) && on_screen(window, "tree-f-a", cx));
        assert!(!on_screen(window, "tree-in-a", cx), "se pinta lo de dentro de una carpeta cerrada");
        assert!(on_screen(window, "rail-brand", cx), "el riel no lleva la marca");
        for module in ["rail-work", "rail-docs", "rail-comms", "rail-platform", "rail-organization"] {
            assert!(on_screen(window, module, cx), "falta {module} en el riel");
        }
        for piece in ["panel-overview", "panel-notes", "spaces-rearrange", "spaces-new", "global-search", "notifications-bell"] {
            assert!(on_screen(window, piece, cx), "falta {piece}");
        }
        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).orgs.current().map(|org| org.name.clone())).as_deref(), Some("G Studio"));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Pulsar un nodo acota «Mi trabajo» a él; el chevrón sólo abre y cierra;
    /// y «Mi trabajo» quita el filtro.
    ///
    /// Roturas que atrapa: un árbol que no hace nada al pulsarlo; que abrir
    /// una rama cambie de qué se está hablando; no poder volver a verlo todo.
    #[gpui_kit::test]
    fn clicking_the_tree_scopes_and_the_chevron_only_opens(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));

        click(window, "tree-toggle-f-a", cx);
        assert!(on_screen(window, "tree-in-a", cx), "el chevrón no abrió la carpeta");
        assert_eq!(scope(cx), None, "abrir una rama acotó la vista");

        click(window, "tree-in-a", cx);
        wait_until(cx, "acota a la lista", |cx| scope(cx) == Some((ScopeKind::List, "in-a".into())));
        wait_until(cx, "llegan sus tareas", |cx| cx.update(|cx| Backend::core(cx).my_work().visible().len()) == 1);

        click(window, "tree-a", cx);
        wait_until(cx, "acota al espacio", |cx| scope(cx) == Some((ScopeKind::Space, "a".into())));

        click(window, "tree-everything", cx);
        wait_until(cx, "quita el filtro", |cx| scope(cx).is_none());
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El servidor de arriba, que además contesta la documentación de
    /// cualquier nodo: resumen escrito, runbook vacío, y dos decisiones, una
    /// de ellas sin efecto.
    fn server_with_docs() -> FakeServer {
        let server = server();
        let doc = json!({
            "doc": null, "orgId": "o1", "attachments": [],
            "tabs": [{ "key": "overview", "body": "# Qué es\n\nUn párrafo." }, { "key": "runbook", "body": "" }],
            "decisions": [
                { "id": "d0", "title": "La vieja", "body": "", "decidedAt": "2026-09-30T12:00:00Z", "supersededById": "d1" },
                { "id": "d1", "title": "La nueva", "body": "Porque sí.", "decidedAt": "2026-10-30T12:00:00Z", "authorName": "Ana" },
            ],
        });
        server.mount(
            Mock::given(method("GET"))
                .and(wiremock::matchers::path_regex("^/api/v1/docs/(space|folder|list)/[^/]+$"))
                .respond_with(ok(doc)),
        );
        server
    }

    fn doc_open(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).docs.target.as_ref().map(|target| target.id.clone()))
    }

    /// Las tareas a la vista, por su id.
    fn visible(cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| Backend::core(cx).my_work().visible().into_iter().map(|task| task.id).collect())
    }

    /// Pulsar una carpeta mirando el calendario deja en el calendario, con
    /// sólo las tareas de esa carpeta, y la cabecera lo dice y lo quita.
    ///
    /// Hasta el 17-nov-2026 esta prueba pedía lo contrario
    /// (`a_folder_opens_its_docs_and_the_list_brings_the_tasks_back`): una
    /// carpeta abría siempre su documentación. Jose, ese día: «si estabas
    /// viendo el calendario, que te deje el calendario filtrando según lo que
    /// se clique en el árbol» (command-and-control-382).
    ///
    /// Roturas que atrapa: una carpeta que saca del calendario a quien la
    /// pulsa (abre su documentación, o vuelve a la lista); acotar y seguir
    /// enseñando las tareas de fuera de la carpeta; acotar sin decirlo ni
    /// dejar quitarlo en la cabecera; una carpeta acotada sin camino a su
    /// documentación.
    #[gpui_kit::test]
    fn a_folder_clicked_from_the_calendar_keeps_the_calendar_with_its_tasks(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        wait_until(cx, "llegan las tareas", |cx| visible(cx).len() == 2);
        click(window, "layout-calendar", cx);
        wait_until(cx, "pasa al calendario", |cx| on_screen(window, "work-calendar", cx));

        click(window, "tree-f-a", cx);
        wait_until(cx, "acota a la carpeta", |cx| scope(cx) == Some((ScopeKind::Folder, "f-a".into())));
        settle_frames(window, cx);
        assert!(on_screen(window, "work-calendar", cx), "la carpeta sacó del calendario");
        assert!(!on_screen(window, "docs", cx), "la carpeta abrió su documentación a quien miraba tareas");
        assert_eq!(visible(cx), ["2"], "no son sólo las tareas de la carpeta");
        assert!(on_screen(window, "scope-clear", cx) && on_screen(window, "scope-copy-id", cx), "la cabecera no dice que está acotado");
        assert!(on_screen(window, "open-docs", cx), "una carpeta acotada no ofrece su documentación");
        // Y en la lista se ve lo mismo: la tarea de la carpeta, y no la otra.
        click(window, "layout-list", cx);
        wait_until(cx, "pasa a la lista", |cx| on_screen(window, "work-row-2", cx));
        assert!(!on_screen(window, "work-row-1", cx), "se pinta una tarea de fuera de la carpeta");

        click(window, "scope-clear", cx);
        wait_until(cx, "vuelven todas", |cx| scope(cx).is_none() && on_screen(window, "work-row-1", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Leyendo documentación, pulsar una carpeta abre la suya; su × devuelve
    /// a las tareas de la carpeta, y sus vistas también, en la que se pulse.
    ///
    /// Antes del 17-nov-2026 de la documentación de una carpeta se volvía a
    /// las tareas que hubiera, porque una carpeta no acotaba
    /// (command-and-control-382).
    ///
    /// Roturas que atrapa: que un clic en una carpeta saque de la
    /// documentación a quien la lee; abrir el documento en el core y seguir
    /// pintando la lista; no tener cómo volver; una documentación sin vistas,
    /// de la que sólo se sale cerrándola; volver de la de una carpeta a las
    /// tareas de otro nodo.
    #[gpui_kit::test]
    fn a_folder_clicked_while_reading_opens_its_docs_and_back_brings_its_tasks(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        assert!(on_screen(window, "view-title", cx) && !on_screen(window, "docs", cx));
        click(window, "tree-a", cx);
        wait_until(cx, "acotado al espacio, ofrece su documentación", |cx| on_screen(window, "open-docs", cx));
        click(window, "open-docs", cx);
        wait_until(cx, "llega la del espacio", |cx| doc_open(cx).as_deref() == Some("a") && on_screen(window, "doc-body", cx));

        click(window, "tree-f-a", cx);
        wait_until(cx, "abre la de la carpeta", |cx| doc_open(cx).as_deref() == Some("f-a") && on_screen(window, "doc-body", cx));
        assert!(!on_screen(window, "view-title", cx), "la lista sigue pintada debajo del documento");
        assert_eq!(scope(cx), Some((ScopeKind::Space, "a".into())), "leer la documentación de una carpeta cambió el ámbito");

        click(window, "doc-close", cx);
        wait_until(cx, "vuelven las tareas", |cx| on_screen(window, "view-title", cx));
        assert!(!on_screen(window, "docs", cx));
        assert_eq!(scope(cx), Some((ScopeKind::Folder, "f-a".into())), "no volvió a las tareas de la carpeta");

        // También lleva las vistas, para no tener que cerrarla: pulsar una
        // vuelve a las tareas de la carpeta en esa vista.
        click(window, "open-docs", cx);
        wait_until(cx, "llega otra vez", |cx| doc_open(cx).as_deref() == Some("f-a") && on_screen(window, "doc-body", cx));
        assert!(on_screen(window, "view-list", cx) && on_screen(window, "view-docs", cx), "la documentación de una carpeta no deja cambiar de vista");
        click(window, "view-board", cx);
        wait_until(cx, "vuelve a las tareas, en tablero", |cx| on_screen(window, "work-board", cx));
        assert!(!on_screen(window, "docs", cx));
        assert_eq!(scope(cx), Some((ScopeKind::Folder, "f-a".into())));
        assert_eq!(visible(cx), ["2"]);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Abre la documentación de la carpeta `f-a`, como quien la pulsa en el
    /// árbol y luego el botón de documentación.
    fn open_folder_docs(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        click(window, "tree-f-a", cx);
        wait_until(cx, "acotado a la carpeta, ofrece su documentación", |cx| on_screen(window, "open-docs", cx));
        click(window, "open-docs", cx);
        wait_until(cx, "abre la de la carpeta", |cx| doc_open(cx).as_deref() == Some("f-a"));
    }

    /// Abre la documentación de la lista `l-a`, como quien la pulsa en el
    /// árbol leyendo la de su espacio.
    fn open_list_docs(window: AnyWindowHandle, cx: &mut TestAppContext) {
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "tree-a", cx);
        wait_until(cx, "acotado al espacio, ofrece su documentación", |cx| on_screen(window, "open-docs", cx));
        click(window, "open-docs", cx);
        wait_until(cx, "llega la del espacio", |cx| on_screen(window, "doc-body", cx));
        click(window, "tree-l-a", cx);
        wait_until(cx, "abre la de la lista", |cx| doc_open(cx).as_deref() == Some("l-a") && on_screen(window, "doc-body", cx));
    }

    /// La documentación de una lista lleva sus vistas, y pulsar una
    /// devuelve a sus tareas en esa vista.
    ///
    /// Roturas que atrapa: unas pestañas de vista que no llevan a ningún
    /// sitio; volver siempre a la lista, se pulse «Tablero» o «Calendario».
    #[gpui_kit::test]
    fn the_views_of_a_list_bring_its_tasks_back_in_that_view(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        open_list_docs(window, cx);
        for view in ["view-board", "view-list", "view-calendar", "view-docs"] {
            assert!(on_screen(window, view, cx), "falta {view}");
        }
        assert_eq!(cx.update(|cx| Backend::core(cx).work_layout()), cac_core::WorkLayout::List);

        click(window, "view-board", cx);

        wait_until(cx, "vuelven sus tareas", |cx| on_screen(window, "view-title", cx));
        assert_eq!(cx.update(|cx| Backend::core(cx).work_layout()), cac_core::WorkLayout::Board);
        assert_eq!(scope(cx), Some((ScopeKind::List, "l-a".to_string())));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn width_on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> f32 {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f32::from(window.try_find(id).expect("no está a la vista").bounds().size.width)
        })
        .unwrap()
    }

    /// La medida de lectura es para la prosa: una tabla usa el ancho de la
    /// página.
    ///
    /// Roturas que atrapa: la medida puesta al cuerpo entero (una tabla
    /// ancha «cabe» partiendo las palabras y no se desliza nunca); un
    /// párrafo que cruza la ventana de lado a lado.
    #[gpui_kit::test]
    fn the_reading_measure_is_for_prose_only(cx: &mut TestAppContext) {
        let server = server();
        let doc = json!({
            "doc": null, "orgId": "o1", "attachments": [], "decisions": [],
            "tabs": [{ "key": "overview", "body": "Un párrafo.\n\n| a | b |\n| - | - |\n| 1 | 2 |" }],
        });
        server.mount(
            Mock::given(method("GET"))
                .and(wiremock::matchers::path_regex("^/api/v1/docs/(space|folder|list)/[^/]+$"))
                .respond_with(ok(doc)),
        );
        server.install(cx);
        let window = open_sized(1500., cx);
        open_folder_docs(window, cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-wide-1", cx));

        let (prose, table) = (width_on_screen(window, "doc-prose-0", cx), width_on_screen(window, "doc-wide-1", cx));
        assert!((600. ..=680.).contains(&prose), "la prosa mide {prose} px");
        assert!(table > prose + 150., "la tabla ({table} px) se quedó en la medida de la prosa ({prose} px)");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn copied(cx: &mut TestAppContext) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    /// El id del nodo se copia de un clic, y lo que la app de siempre tiene
    /// en esta página y aquí todavía no se hace está pintado en su sitio.
    ///
    /// Roturas que atrapa: una pastilla que no copia; dejar fuera compartir,
    /// las páginas, editar, «escribirlo» o registrar una decisión por no
    /// tener aún qué hacer con ellos.
    #[gpui_kit::test]
    fn the_node_id_is_copied_and_what_is_not_here_yet_is_painted(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        open_folder_docs(window, cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));

        click(window, "doc-id", cx);
        assert_eq!(copied(cx).as_deref(), Some("f-a"));
        for id in ["doc-pages", "doc-share", "doc-edit"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        assert!(!on_screen(window, "doc-write", cx) && !on_screen(window, "doc-record-decision", cx));

        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "el runbook vacío ofrece escribirlo", |cx| on_screen(window, "doc-write", cx));
        assert!(!on_screen(window, "doc-edit", cx), "ofrece editar lo que no está escrito");

        click(window, "doc-tab-decisions", cx);
        wait_until(cx, "el registro ofrece apuntar una", |cx| on_screen(window, "doc-record-decision", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Con un espacio acotado hay botón de documentación (372); leyendo, el
    /// árbol abre la del nodo que se pulse; y «Mi trabajo» saca de ahí.
    ///
    /// Roturas que atrapa: ofrecer la documentación sólo con una lista
    /// delante, o sin nada acotado; que un clic en el árbol saque de la
    /// documentación a quien la lee; pulsar «Mi trabajo» y seguir en un
    /// documento.
    #[gpui_kit::test]
    fn the_docs_button_opens_the_scoped_nodes_docs(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        assert!(!on_screen(window, "open-docs", cx), "ofrece documentación sin un nodo delante");

        click(window, "tree-a", cx);
        wait_until(cx, "acotado al espacio, ofrece su documentación", |cx| on_screen(window, "open-docs", cx));
        click(window, "open-docs", cx);
        wait_until(cx, "llega la del espacio", |cx| on_screen(window, "doc-body", cx));
        assert_eq!(doc_open(cx).as_deref(), Some("a"));

        click(window, "tree-l-a", cx);
        wait_until(cx, "abre la de la lista", |cx| doc_open(cx).as_deref() == Some("l-a"));
        assert!(on_screen(window, "docs", cx), "el clic en el árbol sacó de la documentación");

        click(window, "tree-everything", cx);
        wait_until(cx, "vuelve a «Mi trabajo» entero", |cx| on_screen(window, "view-title", cx));
        assert!(doc_open(cx).is_none() && scope(cx).is_none());
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Cada pestaña enseña lo suyo: el texto, «no hay nada» si está vacía, y
    /// en la de decisiones sólo las que siguen vigentes.
    ///
    /// Roturas que atrapa: pestañas que cambian de color y enseñan lo mismo;
    /// una pestaña vacía en blanco; el registro repetido bajo cada pestaña;
    /// una decisión sin efecto pintada como vigente.
    #[gpui_kit::test]
    fn each_tab_shows_its_own(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        open_folder_docs(window, cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));
        assert!(!on_screen(window, "doc-empty", cx) && !on_screen(window, "doc-decision-d1", cx));

        click(window, "doc-tab-runbook", cx);
        wait_until(cx, "el runbook está vacío y lo dice", |cx| on_screen(window, "doc-empty", cx));
        assert!(!on_screen(window, "doc-body", cx), "sigue el texto de la otra pestaña");

        click(window, "doc-tab-decisions", cx);
        wait_until(cx, "sale la decisión vigente", |cx| on_screen(window, "doc-decision-d1", cx));
        assert!(!on_screen(window, "doc-decision-d0", cx), "pinta una decisión que otra dejó sin efecto");
        assert!(!on_screen(window, "doc-empty", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    // ── Las páginas de un documento ─────────────────────────────────────

    fn page_item(id: &str, parent: Option<&str>, title: &str) -> serde_json::Value {
        let mut item = json!({ "id": id, "rank": "0.5", "title": title, "hasBody": true, "updatedAt": "2026-11-14T20:17:40Z" });
        if let Some(parent) = parent {
            item["parentId"] = json!(parent);
        }
        item
    }

    fn page_view(id: &str, title: &str, body: &str, trail: serde_json::Value, children: serde_json::Value, links: serde_json::Value) -> serde_json::Value {
        json!({
            "page": { "id": id, "title": title, "body": body, "docId": "d1", "orgId": "o1", "updatedAt": "2026-11-14T20:17:40Z" },
            "breadcrumb": trail, "children": children, "orgId": "o1", "referencedFrom": links,
        })
    }

    /// El servidor de la documentación, con las páginas de la carpeta `f-a`:
    /// «Runbooks» (vacía) y «Arquitectura», que lleva dentro «El core», que
    /// lleva dentro «Los tipos». En ese orden, que no es el alfabético. «El
    /// core» está enlazada desde «Los tipos» y desde una tarea; «Rota» no se
    /// puede abrir.
    fn server_with_pages() -> FakeServer {
        let server = server_with_docs();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let tree = json!([
            page_item("p-run", None, "Runbooks"),
            page_item("p-arq", None, "Arquitectura"),
            page_item("p-core", Some("p-arq"), "El core"),
            page_item("p-types", Some("p-core"), "Los tipos"),
            page_item("p-bad", None, "Rota"),
        ]);
        server.mount(get("/api/v1/docs/folder/f-a/pages").respond_with(ok(tree)));
        let links = json!([
            { "kind": "page", "title": "Los tipos", "where": "Planta", "link": "/tasks?doc=folder:f-a&page=p-types" },
            { "kind": "task", "title": "Subir el backend", "where": "tasks", "link": "/tasks?task=t1" },
        ]);
        let pages = [
            page_view("p-run", "Runbooks", "", json!([]), json!([]), json!([])),
            page_view("p-arq", "Arquitectura", "## Capas\n\nTres.", json!([]), json!([page_item("p-core", Some("p-arq"), "El core")]), json!([])),
            page_view(
                "p-core",
                "El core",
                "## Qué es\n\nLo compartido.",
                json!([{ "id": "p-arq", "title": "Arquitectura" }]),
                json!([page_item("p-types", Some("p-core"), "Los tipos")]),
                links,
            ),
            page_view(
                "p-types",
                "Los tipos",
                "Espejos.",
                json!([{ "id": "p-arq", "title": "Arquitectura" }, { "id": "p-core", "title": "El core" }]),
                json!([]),
                json!([]),
            ),
        ];
        for page in pages {
            let route = format!("/api/v1/docs/folder/f-a/pages/{}", page["page"]["id"].as_str().unwrap());
            server.mount(get(&route).respond_with(ok(page)));
        }
        server
    }

    fn page_open(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Backend::snapshot(cx).read(cx).docs.page_id.clone())
    }

    fn place_on_screen(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> (f32, f32, f32) {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.try_find(id).expect("no está a la vista").bounds();
            (f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width))
        })
        .unwrap()
    }

    /// La documentación de la carpeta, con su hoja «Páginas» abierta.
    fn open_pages_sheet(window: AnyWindowHandle, cx: &mut TestAppContext) {
        open_folder_docs(window, cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));
        click(window, "doc-pages", cx);
        wait_until(cx, "sale la hoja con sus páginas", |cx| on_screen(window, "doc-page-row-p-arq", cx));
    }

    /// El botón de páginas abre su árbol —la portada y las páginas en el
    /// orden del servidor, las de dentro plegadas y con su sangría—, una
    /// página se abre y se lee con sus migas, su título y lo que tiene
    /// dentro, sin las pestañas de la portada, y de ella se va a una hija y
    /// se vuelve a la portada.
    ///
    /// Roturas que atrapa: el botón pintado que no hace nada («no vi las
    /// páginas»); las páginas por orden alfabético; una página de dentro a
    /// la altura de su madre; que el chevrón abra la página o cierre la
    /// hoja en vez de desplegar; la hoja que se queda puesta tras elegir;
    /// las pestañas de la portada sobre una página; migas o hijas que no
    /// llevan a ningún sitio; no poder volver a la portada; abrir una página
    /// desde el pie de otra y aparecer a media página, donde se estaba.
    #[gpui_kit::test]
    fn the_pages_button_opens_the_tree_and_a_page_is_read(cx: &mut TestAppContext) {
        let server = server_with_pages();
        server.install(cx);
        let window = open(cx);
        open_pages_sheet(window, cx);

        assert!(on_screen(window, "doc-pages-home", cx), "falta la portada en la hoja");
        assert!(!on_screen(window, "doc-page-row-p-core", cx), "pinta lo de dentro de una página plegada");
        let (run, arq) = (place_on_screen(window, "doc-page-row-p-run", cx), place_on_screen(window, "doc-page-row-p-arq", cx));
        assert!(run.1 < arq.1, "las páginas no salen en el orden del servidor");

        click(window, "doc-page-toggle-p-arq", cx);
        wait_until(cx, "despliega lo que tiene dentro", |cx| on_screen(window, "doc-page-row-p-core", cx));
        settle_frames(window, cx);
        assert!(page_open(cx).is_none(), "el chevrón abrió la página");
        assert!(on_screen(window, "doc-pages-sheet", cx), "desplegar cerró la hoja");
        let (outer, inner) = (place_on_screen(window, "doc-page-toggle-p-arq", cx), place_on_screen(window, "doc-page-toggle-p-core", cx));
        assert_eq!(inner.0 - outer.0, 12., "la de dentro no va sangrada bajo su madre");

        click(window, "doc-page-row-p-core", cx);
        wait_until(cx, "se lee la página", |cx| on_screen(window, "doc-page-child-p-types", cx));
        assert!(!on_screen(window, "doc-pages-sheet", cx), "la hoja sigue puesta tras elegir");
        assert!(!on_screen(window, "doc-tab-overview", cx), "las pestañas de la portada siguen sobre la página");
        // `doc-scroll-<página>`: cada página lleva su propio desplazamiento.
        for id in ["doc-page-title", "doc-body", "doc-page-crumb-home", "doc-page-crumb-p-arq", "doc-scroll-p-core"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }

        click(window, "doc-page-child-p-types", cx);
        wait_until(cx, "abre la de dentro", |cx| on_screen(window, "doc-page-crumb-p-core", cx));
        click(window, "doc-page-crumb-p-arq", cx);
        wait_until(cx, "la miga lleva a su página", |cx| on_screen(window, "doc-page-child-p-core", cx));
        assert_eq!(page_open(cx).as_deref(), Some("p-arq"));

        click(window, "doc-page-crumb-home", cx);
        wait_until(cx, "vuelve a la portada", |cx| on_screen(window, "doc-tab-overview", cx));
        assert!(!on_screen(window, "doc-page-title", cx) && page_open(cx).is_none());
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// La hoja tapa la ventana entera, con su panel de 320 px a la derecha,
    /// y se cierra con la equis, con Escape, pulsando fuera o eligiendo la
    /// portada.
    ///
    /// Roturas que atrapa: una hoja que deja el panel lateral al aire (se
    /// pulsa otro nodo con ella abierta); una hoja de la que no se sale sin
    /// elegir una página; Escape sin efecto por no tener el foco.
    #[gpui_kit::test]
    fn the_pages_sheet_covers_the_window_and_closes_four_ways(cx: &mut TestAppContext) {
        let server = server_with_pages();
        server.install(cx);
        let window = open(cx);
        open_pages_sheet(window, cx);
        let shown = |cx: &mut TestAppContext| cx.update(|cx| Backend::snapshot(cx).read(cx).docs.pages_shown);
        let reopen = |cx: &mut TestAppContext| {
            click(window, "doc-pages", cx);
            wait_until(cx, "vuelve a salir la hoja", |cx| on_screen(window, "doc-pages-sheet", cx));
        };

        let (veil, sheet) = (place_on_screen(window, "doc-pages-veil", cx), place_on_screen(window, "doc-pages-sheet", cx));
        assert_eq!((veil.0, veil.1, veil.2), (0., 0., 1100.), "el velo no tapa la ventana entera");
        assert_eq!((sheet.0, sheet.2), (780., 320.), "el panel no está a la derecha con su ancho");

        click(window, "doc-pages-close", cx);
        wait_until(cx, "la equis la cierra", |cx| !on_screen(window, "doc-pages-sheet", cx));

        reopen(cx);
        press(window, "escape", cx);
        wait_until(cx, "Escape la cierra", |cx| !shown(cx));

        reopen(cx);
        // El centro del velo cae fuera del panel, que es el tercio derecho.
        click(window, "doc-pages-veil", cx);
        wait_until(cx, "pulsar fuera la cierra", |cx| !shown(cx));

        reopen(cx);
        click(window, "doc-page-row-p-run", cx);
        wait_until(cx, "abre la página", |cx| on_screen(window, "doc-page-title", cx));
        reopen(cx);
        click(window, "doc-pages-home", cx);
        wait_until(cx, "la portada cierra la hoja y la página", |cx| !shown(cx) && page_open(cx).is_none());
        assert!(on_screen(window, "doc-tab-overview", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Lo que la app de siempre tiene alrededor de las páginas y aquí
    /// todavía no se hace está pintado en su sitio: crear, la papelera y el
    /// menú en la hoja; editar, el historial, tirarla y «página nueva
    /// dentro» en una página; las plantillas y «escribirlo» en una vacía. Y
    /// una que no se pudo abrir lo dice, sin pie ni secciones de una página
    /// que no hay.
    ///
    /// Roturas que atrapa: dejar fuera esas piezas por no tener aún qué
    /// hacer con ellas; ofrecer «editar» en una página vacía; una página
    /// rota en blanco, o con el pie y «páginas dentro» de otra, o con el
    /// pie del registro de decisiones de la portada.
    #[gpui_kit::test]
    fn what_is_not_here_yet_around_pages_is_painted(cx: &mut TestAppContext) {
        let server = server_with_pages();
        server.install(cx);
        let window = open(cx);
        open_pages_sheet(window, cx);
        for id in ["doc-pages-new", "doc-pages-trash", "doc-page-new-p-arq", "doc-page-menu-p-arq"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        assert!(!on_screen(window, "doc-pages-error", cx), "dice que el árbol falló cuando llegó");
        // La portada, en el registro: su pie es «apuntar una decisión», que
        // no es de ninguna página.
        cx.update(|cx| Backend::core(cx).docs().show_tab("decisions"));

        click(window, "doc-page-row-p-arq", cx);
        wait_until(cx, "se lee la página", |cx| on_screen(window, "doc-page-child-p-core", cx));
        for id in ["doc-edit", "doc-page-history", "doc-page-trash", "doc-page-new-child", "doc-page-children"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        assert!(!on_screen(window, "doc-page-backlinks", cx), "«referenciado desde» sin nada que la enlace");

        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p-run").await }));
        wait_until(cx, "la vacía ofrece por dónde empezar", |cx| on_screen(window, "doc-page-template-component", cx));
        for id in ["doc-empty", "doc-page-template-procedure", "doc-write", "doc-page-history", "doc-page-children"] {
            assert!(on_screen(window, id, cx), "falta {id}");
        }
        assert!(!on_screen(window, "doc-edit", cx), "ofrece editar lo que no está escrito");

        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p-bad").await }));
        wait_until(cx, "la rota dice que no se pudo", |cx| page_open(cx).as_deref() == Some("p-bad") && on_screen(window, "doc-empty", cx));
        wait_until(cx, "y no sigue cargando", |cx| cx.update(|cx| !Backend::snapshot(cx).read(cx).docs.page_loading));
        assert!(on_screen(window, "doc-page-crumb-home", cx), "de una página rota no se puede volver");
        for id in ["doc-foot", "doc-page-children", "doc-page-template-component", "doc-body", "doc-record-decision"] {
            assert!(!on_screen(window, id, cx), "una página que no se abrió pinta {id}");
        }
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un árbol de páginas que no llegó se dice en la hoja, con la portada
    /// todavía a mano.
    ///
    /// Rotura que atrapa: una hoja con sólo «Portada», igual que la de un
    /// documento sin páginas, cuando lo que pasó es que no se pudieron traer.
    #[gpui_kit::test]
    fn a_page_tree_that_did_not_arrive_is_said(cx: &mut TestAppContext) {
        // Este servidor no tiene la ruta de las páginas: contesta 404.
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        open_folder_docs(window, cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));

        click(window, "doc-pages", cx);

        wait_until(cx, "la hoja dice que no llegaron", |cx| on_screen(window, "doc-pages-error", cx));
        assert!(on_screen(window, "doc-pages-home", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// De lo que enlaza a una página, otra página de este documento se abre
    /// al pulsarla; una tarea, que todavía no se abre desde aquí, no.
    ///
    /// Roturas que atrapa: «referenciado desde» que no se pinta; un enlace
    /// a una página hermana que no lleva a ella; pulsar una tarea y perder
    /// la página que se leía.
    #[gpui_kit::test]
    fn a_page_of_this_doc_that_links_here_opens_and_a_task_does_not(cx: &mut TestAppContext) {
        let server = server_with_pages();
        server.install(cx);
        let window = open(cx);
        open_folder_docs(window, cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.docs().open_page("p-core").await }));
        wait_until(cx, "sale lo que la enlaza", |cx| on_screen(window, "doc-page-backlinks", cx));

        click(window, "doc-page-backlink-1", cx);
        settle_frames(window, cx);
        assert_eq!(page_open(cx).as_deref(), Some("p-core"), "pulsar una tarea sacó de la página");

        click(window, "doc-page-backlink-0", cx);
        wait_until(cx, "abre la página que la enlaza", |cx| page_open(cx).as_deref() == Some("p-types"));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Cambiar de organización cambia el árbol entero.
    ///
    /// Rotura que atrapa: cambiar el nombre de arriba y seguir con el árbol
    /// de la organización anterior.
    #[gpui_kit::test]
    fn switching_organization_changes_the_tree(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));

        cx.update(|cx| Workspace::switch_org("o2".into(), cx));

        wait_until(cx, "llega el árbol de la otra", |cx| on_screen(window, "tree-z", cx));
        assert!(!on_screen(window, "tree-a", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Elegir tema viste la app en el acto y se recuerda para el arranque.
    ///
    /// Roturas que atrapa: un menú de tema que no cambia nada; cambiarlo sólo
    /// hasta cerrar la app; arrancar siempre en oscuro aunque se eligió claro.
    #[gpui_kit::test]
    fn choosing_a_theme_dresses_the_app_and_is_remembered(cx: &mut TestAppContext) {
        use cac_core::ThemePreference;
        use gpui_kit::component::ActiveTheme;
        let server = server();
        server.install(cx);
        let background = |cx: &mut TestAppContext| cx.update(|cx| cx.theme().background);
        let dark = background(cx);

        cx.update(|cx| Workspace::choose_theme(ThemePreference::Light, cx));

        let light = background(cx);
        assert_ne!(light, dark, "el tema no cambió");
        assert_eq!(cx.update(|cx| Backend::core(cx).theme()), ThemePreference::Light);

        // Como al arrancar: otro tema puesto, y se vuelve al elegido.
        cx.update(|cx| crate::theme::apply(gpui_kit::component::ThemeMode::Dark, cx));
        assert_eq!(background(cx), dark);
        cx.update(crate::theme::apply_preference);
        assert_eq!(background(cx), light);
    }

    /// Elegir idioma cambia los textos y se le dice al servidor.
    ///
    /// Roturas que atrapa: un menú de idioma que no hace nada; cambiar el
    /// idioma sólo en esta máquina.
    #[gpui_kit::test]
    fn choosing_a_language_changes_the_texts(cx: &mut TestAppContext) {
        use cac_core::Locale;
        let server = server();
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/auth/locale")).respond_with(ok(json!(null))));
        server.install(cx);
        sign_in(cx);
        assert_eq!(cx.update(|cx| crate::lang::tr(cx, "nav:item.myWork")), "Mi trabajo");

        cx.update(|cx| Workspace::choose_language(Some(Locale::En), cx));

        wait_until(cx, "los textos pasan a inglés", |cx| cx.update(|cx| crate::lang::tr(cx, "nav:item.myWork")) == "My work");
        wait_until(cx, "se le dice al servidor", |_| server.hits("/api/v1/auth/locale") == 1);
    }

    /// Un corte que dura se avisa, y «Reconectar» vuelve a pedirlo todo.
    ///
    /// Roturas que atrapa: una app que deja de ponerse al día sin decirlo;
    /// un aviso puesto desde el primer segundo; un botón de reconectar que
    /// no pide nada.
    #[gpui_kit::test]
    fn an_outage_that_lasts_is_announced(cx: &mut TestAppContext) {
        // Sin ruta de eventos: el stream se cae en cada intento.
        let server = server();
        server.install_with(cx, |config| config.stream_grace = Duration::from_millis(400));
        let window = open(cx);
        // Nada más abrir, sin esperar a nada: el corte todavía no puede durar.
        // (Esperar aquí al árbol hacía la prueba depender de lo cargada que
        // estuviera la máquina.)
        assert!(!on_screen(window, "connection-banner", cx), "avisa antes de que el corte dure");
        // Y el aviso sale exactamente cuando el core dice que dura.
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        let lasting = cx.update(|cx| Backend::snapshot(cx).read(cx).connection.lasting);
        assert_eq!(on_screen(window, "connection-banner", cx), lasting);

        wait_until(cx, "avisa del corte", |cx| on_screen(window, "connection-banner", cx));
        let asked = server.hits("/api/v1/organizations/");
        click(window, "reconnect", cx);
        wait_until(cx, "vuelve a pedirlo todo", |_| server.hits("/api/v1/organizations/") > asked);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }
    /// Escribir en el filtro del panel deja en el árbol lo que casa, con su
    /// carpeta abierta; y si nada casa, se dice.
    ///
    /// Roturas que atrapa: un filtro que no filtra; que lo que casa siga
    /// escondido en su carpeta plegada; un árbol en blanco sin decir por qué;
    /// decir «nada coincide» con todo a la vista; un chevrón que filtrando
    /// pliega lo que se buscaba.
    #[gpui_kit::test]
    fn typing_in_the_panel_filter_narrows_the_tree(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        assert!(!on_screen(window, "tree-no-matches", cx), "dice que nada coincide sin haber filtrado");
        let typed = |text: &str, cx: &mut TestAppContext| {
            let text = text.to_string();
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.click("tree-filter", cx);
                window.input(&text, cx);
            })
            .unwrap();
        };

        typed("en carp", cx);
        assert!(on_screen(window, "tree-in-a", cx), "lo que casa sigue dentro de su carpeta plegada");
        assert!(on_screen(window, "tree-a", cx) && on_screen(window, "tree-f-a", cx));
        assert!(!on_screen(window, "tree-l-a", cx), "el filtro no quitó lo que no casa");
        assert!(!on_screen(window, "tree-no-matches", cx), "dice que nada coincide con lo que casa a la vista");
        click(window, "tree-toggle-f-a", cx);
        assert!(on_screen(window, "tree-in-a", cx), "el chevrón plegó lo que se buscaba");

        typed("zzz", cx);
        assert!(on_screen(window, "tree-no-matches", cx), "un árbol en blanco y sin decir por qué");
        assert!(!on_screen(window, "tree-a", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Una organización sin espacios no dice «nada coincide»: nadie filtró.
    ///
    /// Rotura que atrapa: leer cualquier árbol vacío como un filtro sin
    /// resultados.
    #[gpui_kit::test]
    fn an_empty_tree_is_not_a_filter_without_matches(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio")]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.install(cx);
        let window = open(cx);

        wait_until(cx, "el árbol llega, vacío", |cx| {
            cx.update(|cx| {
                let tree = &Backend::snapshot(cx).read(cx).tree;
                tree.org_id.as_deref() == Some("o1") && !tree.loading
            })
        });

        assert!(on_screen(window, "tree-filter", cx));
        assert!(!on_screen(window, "tree-no-matches", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El «···» de una fila abre su documentación, y abrir el menú no acota
    /// las tareas a ese nodo.
    ///
    /// Roturas que atrapa: un espacio o una lista sin camino a su
    /// documentación desde el árbol (su clic acota); que pulsar el menú
    /// cuente como pulsar la fila; un chevrón en una lista, que no se abre.
    #[gpui_kit::test]
    fn a_spaces_menu_opens_its_docs_without_scoping(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        assert!(on_screen(window, "tree-menu-a", cx) && on_screen(window, "tree-menu-f-a", cx) && on_screen(window, "tree-menu-l-a", cx));
        // Una lista no se abre: no lleva chevrón.
        assert!(on_screen(window, "tree-toggle-f-a", cx) && !on_screen(window, "tree-toggle-l-a", cx));

        click(window, "tree-menu-a", cx);
        settle_frames(window, cx);
        assert_eq!(scope(cx), None, "abrir el menú acotó las tareas al espacio");

        wait_until(cx, "el menú se abre", |cx| on_screen(window, "tree-act-docs-a", cx));
        click(window, "tree-act-docs-a", cx);
        wait_until(cx, "llega la documentación del espacio", |cx| on_screen(window, "doc-body", cx));
        assert_eq!(doc_open(cx).as_deref(), Some("a"));
        assert_eq!(scope(cx), None);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El menú de la organización acaba en «Crear una organización», como en
    /// la app de siempre.
    ///
    /// Rotura que atrapa: que la entrada se caiga del menú.
    #[gpui_kit::test]
    fn the_org_menu_ends_with_creating_one(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        click(window, "org-switch", cx);
        wait_until(cx, "el menú se abre", |cx| on_screen(window, "org-create", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// «Claude Code», en el menú de la cuenta, abre la ventana que instala
    /// `cac-mcp` y lo mantiene al día: dice que no está y lo instala donde el
    /// script, enseña la orden para registrarlo sin ningún token, avisa con
    /// un punto en el menú cuando hay una versión nueva y la instala, y una
    /// descarga cuya suma no es la publicada se rechaza diciéndolo.
    ///
    /// La release es de mentira y está en esta máquina; la carpeta donde se
    /// instala es temporal.
    ///
    /// Roturas que atrapa: una entrada que sigue sin hacer nada; el menú
    /// abierto encima de la ventana; ofrecer «Instalar» con el binario ya
    /// puesto, o no ofrecer nada cuando hay versión nueva; un botón que no
    /// instala, o que da por hecho lo que no se instaló; un punto que no sale
    /// o que no se apaga al actualizar; una orden que apunta a otro binario o
    /// que lleva un token; un fallo que se queda mudo o que rompe lo
    /// instalado; un Escape que no cierra.
    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn claude_code_in_the_account_menu_installs_and_updates_the_mcp_binary(cx: &mut TestAppContext) {
        use crate::connect::{McpSetup, Work};
        use cac_mcp::update::{fake::FakeReleases, Machine};

        let releases = FakeReleases::start();
        releases.announce("1.34.0");
        releases.publish("1.34.0", b"#!/bin/sh\necho 'cac-mcp 1.34.0'\n");
        let scratch = std::env::temp_dir().join(format!("cac-desktop-connect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let binary = scratch.join("home").join(".local").join("bin").join("cac-mcp");
        let machine = Machine {
            source: releases.source(),
            state_dir: Some(scratch.join("state")),
            look_in: vec![binary.clone()],
            install_to: Some(binary.clone()),
        };

        let server = server();
        server.install(cx);
        cx.update(|cx| cx.set_global(McpSetup { machine, server_url: "https://cac.example".into() }));
        let (window, workspace) = open_with_view(1100., cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        let work = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).connect.read(cx).work.clone());
        let open_menu = |cx: &mut TestAppContext| {
            click(window, "account-menu", cx);
            wait_until(cx, "el menú se abre", |cx| on_screen(window, "account-connect", cx));
        };
        let open_connect = |cx: &mut TestAppContext| {
            open_menu(cx);
            click(window, "account-connect", cx);
            wait_until(cx, "la ventana se abre", |cx| on_screen(window, "connect-dialog", cx));
            assert!(!on_screen(window, "account-log-out", cx), "el menú se quedó abierto encima de la ventana");
        };
        let close = |cx: &mut TestAppContext| {
            press(window, "escape", cx);
            wait_until(cx, "Escape la cierra", |cx| !on_screen(window, "connect-dialog", cx));
        };

        // No está: se dice, y se ofrece instalarlo.
        wait_until(cx, "se mira la máquina al entrar", |cx| cx.update(|cx| workspace.read(cx).connect.read(cx).status.is_some()));
        open_menu(cx);
        assert!(!on_screen(window, "account-connect-update", cx), "el punto sale sin nada que actualizar");
        click(window, "account-connect", cx);
        wait_until(cx, "la ventana se abre", |cx| on_screen(window, "connect-dialog", cx));
        assert!(!on_screen(window, "account-log-out", cx), "el menú se quedó abierto encima de la ventana");
        wait_until(cx, "dice que no está", |cx| on_screen(window, "connect-missing", cx));
        assert!(!on_screen(window, "connect-installed", cx));

        // La orden apunta a donde va a quedar, con el sitio del token marcado.
        click(window, "connect-copy", cx);
        let copied = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert_eq!(copied, format!("claude mcp add cac -e CAC_URL=https://cac.example -e CAC_TOKEN=PASTE_YOUR_TOKEN -- \"{}\"", binary.display()));

        click(window, "connect-run", cx);
        wait_until(cx, "se instala", |cx| on_screen(window, "connect-done", cx));
        assert_eq!(std::fs::read(&binary).unwrap(), b"#!/bin/sh\necho 'cac-mcp 1.34.0'\n");
        assert!(on_screen(window, "connect-installed", cx) && on_screen(window, "connect-current", cx));
        assert!(!on_screen(window, "connect-run", cx), "ya instalado y al día, sigue ofreciendo instalar");
        close(cx);

        // Sale una versión nueva: al volver a mirar, el punto y el botón.
        releases.announce("1.35.0");
        releases.publish("1.35.0", b"#!/bin/sh\necho 'cac-mcp 1.35.0'\n");
        open_connect(cx);
        wait_until(cx, "dice que hay una nueva", |cx| on_screen(window, "connect-newer", cx));
        assert!(!on_screen(window, "connect-done", cx), "el «instalado» de antes sigue a la vista");
        close(cx);
        open_menu(cx);
        assert!(on_screen(window, "account-connect-update", cx), "hay versión nueva y el menú no lo dice");
        click(window, "account-connect", cx);
        wait_until(cx, "la ventana se abre", |cx| on_screen(window, "connect-run", cx));
        click(window, "connect-run", cx);
        wait_until(cx, "se actualiza", |cx| on_screen(window, "connect-done", cx));
        assert_eq!(std::fs::read(&binary).unwrap(), b"#!/bin/sh\necho 'cac-mcp 1.35.0'\n");
        close(cx);
        open_menu(cx);
        assert!(!on_screen(window, "account-connect-update", cx), "actualizado, y el punto sigue");
        click(window, "account-connect", cx);
        wait_until(cx, "la ventana se abre", |cx| on_screen(window, "connect-dialog", cx));
        close(cx);

        // Lo que se baja no es lo que se publicó: se rechaza y se dice.
        releases.announce("1.36.0");
        releases.publish("1.36.0", b"#!/bin/sh\necho 'cac-mcp 1.36.0'\n");
        releases.put(&format!("/download/mcp-v1.36.0/{}", cac_mcp::update::this_platform_asset().unwrap()), "manipulado");
        open_connect(cx);
        wait_until(cx, "dice que hay una nueva", |cx| on_screen(window, "connect-run", cx));
        click(window, "connect-run", cx);
        wait_until(cx, "el fallo se dice", |cx| on_screen(window, "connect-problem", cx));
        assert!(matches!(work(cx), Work::Failed(reason) if reason.contains("checksum mismatch")), "{:?}", work(cx));
        assert_eq!(std::fs::read(&binary).unwrap(), b"#!/bin/sh\necho 'cac-mcp 1.35.0'\n", "una descarga rechazada tocó lo instalado");
        assert!(on_screen(window, "connect-run", cx), "tras un fallo no se puede reintentar");

        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
        let _ = std::fs::remove_dir_all(scratch);
    }

    /// El menú de la cuenta ofrece los tres temas y los tres idiomas a la
    /// vez, y elegir uno lo cambia sin cerrar el menú; «Salir» saca.
    ///
    /// Roturas que atrapa: opciones pintadas que no hacen nada al pulsarlas;
    /// que elegir el tema cierre el menú (para probar otro hay que volver a
    /// abrirlo); un «Salir» que no cierra la sesión.
    #[gpui_kit::test]
    fn the_account_menu_changes_theme_and_language_in_place(cx: &mut TestAppContext) {
        use cac_core::{Locale, ThemePreference};
        let server = server();
        server.mount(Mock::given(method("PATCH")).and(path("/api/v1/auth/locale")).respond_with(ok(json!(null))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));

        click(window, "account-menu", cx);
        assert!(on_screen(window, "account-live", cx), "el avatar no lleva el punto del flujo en vivo");
        wait_until(cx, "el menú se abre", |cx| on_screen(window, "account-theme-1", cx));
        for row in ["account-profile", "account-password", "account-connect", "account-notifications", "account-check"] {
            assert!(on_screen(window, row, cx), "falta {row} en el menú de la cuenta");
        }
        click(window, "account-theme-1", cx);
        assert_eq!(cx.update(|cx| Backend::core(cx).theme()), ThemePreference::Light);
        assert!(on_screen(window, "account-language-1", cx), "elegir el tema cerró el menú");

        click(window, "account-language-1", cx);
        wait_until(cx, "pasa a inglés", |cx| cx.update(|cx| Backend::core(cx).locale_preference()) == Some(Locale::En));

        click(window, "account-log-out", cx);
        wait_until(cx, "sale", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).session.is_none()));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El panel se suelta con su botón y deja el sitio a la vista; suelto,
    /// se asoma al pasar el ratón por el riel, se queda mientras el ratón
    /// está en él y se va al salir; y su botón lo vuelve a fijar.
    ///
    /// Roturas que atrapa: un botón de plegar que no pliega; un panel
    /// plegado sin forma de volver; que se asome sin acercar el ratón; que
    /// desaparezca al ir del riel al panel, antes de poder pulsar nada; que
    /// se quede asomado para siempre.
    #[gpui_kit::test]
    fn the_panel_folds_and_peeks_when_the_pointer_is_on_the_rail(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        let hover = |id: &'static str, cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.hover(id, cx);
            })
            .unwrap();
        };

        // «Tarea nueva» queda a la derecha, fuera de donde se asoma el panel.
        click(window, "panel-toggle", cx);
        assert!(!on_screen(window, "tree-a", cx), "el botón no plegó el panel");
        assert!(!on_screen(window, "panel-peek", cx), "se asoma sin acercar el ratón");

        hover("rail", cx);
        assert!(on_screen(window, "panel-peek", cx) && on_screen(window, "tree-a", cx), "no se asoma con el ratón en el riel");
        hover("tree-a", cx);
        assert!(on_screen(window, "tree-a", cx), "se fue al pasar del riel al panel");
        hover("new-task", cx);
        assert!(!on_screen(window, "panel-peek", cx), "se queda asomado con el ratón fuera");

        hover("rail", cx);
        click(window, "panel-toggle", cx);
        hover("new-task", cx);
        assert!(on_screen(window, "tree-a", cx) && !on_screen(window, "panel-peek", cx), "su botón no lo volvió a fijar");

        // Y el módulo del riel hace lo mismo que el botón.
        click(window, "rail-work", cx);
        hover("new-task", cx);
        assert!(!on_screen(window, "tree-a", cx));
        click(window, "rail-work", cx);
        assert!(on_screen(window, "tree-a", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    fn press(window: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
    }

    fn pinned(cx: &mut TestAppContext) -> bool {
        cx.update(|cx| Backend::core(cx).panel_pinned())
    }

    /// Ctrl+B (⌘B en un Mac) pliega el panel y lo vuelve a fijar, sin haber
    /// pulsado antes en ningún sitio; una «b» suelta no, y con el filtro del
    /// árbol enfocado el atajo sigue siendo el del panel.
    ///
    /// Roturas que atrapa: un atajo anunciado que no hace nada; uno que sólo
    /// funciona después de pulsar algo con el ratón; plegar el panel al
    /// escribir una «b»; plegar con Ctrl+Mayús+B.
    #[gpui_kit::test]
    fn the_keyboard_folds_the_panel(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));

        press(window, "secondary-b", cx);
        assert!(!on_screen(window, "tree-a", cx), "el atajo no plegó el panel");
        press(window, "secondary-b", cx);
        assert!(on_screen(window, "tree-a", cx), "el atajo no lo volvió a fijar");

        press(window, "b", cx);
        press(window, "secondary-shift-b", cx);
        assert!(on_screen(window, "tree-a", cx), "se plegó con una tecla que no es su atajo");

        // Escribiendo en un campo corriente el atajo es el mismo: sólo el
        // editor de markdown tiene otro uso para esa tecla.
        click(window, "tree-filter", cx);
        press(window, "secondary-b", cx);
        assert!(!on_screen(window, "tree-a", cx), "con el filtro enfocado el atajo no pliega");
        // El filtro tenía el foco y ya no se pinta: el atajo tiene que seguir
        // oyéndose para poder volver.
        press(window, "secondary-b", cx);
        assert!(on_screen(window, "tree-a", cx), "plegado con el foco dentro, el atajo ya no lo despliega");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Plegado o fijo, el panel se recuerda: la app vuelve a abrir como se
    /// dejó, se plegara con el teclado o con su botón.
    ///
    /// Roturas que atrapa: plegar sólo hasta cerrar la app; recordar lo que
    /// hizo el botón y no lo que hizo el atajo; abrir siempre plegado una vez
    /// que se plegó (no recordar que se volvió a fijar).
    #[gpui_kit::test]
    fn the_folded_panel_is_remembered(cx: &mut TestAppContext) {
        let server = server();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-a", cx));
        // Otra ventana de la misma app, como al volver a abrirla: el core y
        // lo que guardó son los mismos. Se le pregunta a la pantalla y no a
        // lo pintado: en las pruebas el ratón está en la esquina, sobre el
        // riel, y un panel plegado se asoma.
        let reopens_pinned = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let options = WindowOptions::default();
                let (_, workspace) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window");
                workspace.read(cx).panel_pinned
            })
        };

        press(window, "secondary-b", cx);
        assert!(!pinned(cx), "el atajo no guardó que se plegó");
        assert!(!reopens_pinned(cx), "volvió a abrir con el panel fijo");

        // Con el ratón, que no depende de qué ventana tiene el teclado.
        click(window, "rail-work", cx);
        assert!(pinned(cx), "el botón no guardó que se volvió a fijar");
        assert!(reopens_pinned(cx), "volvió a abrir plegado lo que se dejó fijo");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Escribiendo un comentario, Ctrl+B es la negrita del editor y el panel
    /// no se mueve.
    ///
    /// Rotura que atrapa: que el atajo del panel se adelante al del editor
    /// (oírlo al bajar en vez de al subir, o como atajo global) y a quien
    /// pone una negrita se le pliegue el panel debajo.
    #[gpui_kit::test]
    fn writing_a_comment_the_shortcut_is_the_editors_bold(cx: &mut TestAppContext) {
        let server = server();
        let mut detail: serde_json::Value = serde_json::from_str(include_str!("../../core/tests/fixtures/task_detail.json")).unwrap();
        detail["task"]["id"] = json!("1");
        let mut column = detail["status"].clone();
        column["listId"] = detail["task"]["listId"].clone();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/tasks/1")).respond_with(ok(detail)));
        server.mount(
            Mock::given(method("GET")).and(wiremock::matchers::path_regex("^/api/v1/task-lists/[^/]+/statuses$")).respond_with(ok(json!([column]))),
        );
        server.install(cx);
        let window = open_sized(1700., cx);
        wait_until(cx, "las tareas llegan", |cx| on_screen(window, "work-row-1", cx));
        click(window, "work-row-1", cx);
        wait_until(cx, "llega el detalle", |cx| on_screen(window, "comment-input", cx));

        click(window, "comment-input", cx);
        press(window, "secondary-b", cx);
        assert!(pinned(cx), "poner una negrita plegó el panel");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El sidebar dice de qué organización es y marca lo que cada fila es:
    /// lo que tiene documentación escrita y lo que ve un cliente.
    ///
    /// Roturas que atrapa: una organización sin su «quién eres y cuánta
    /// gente»; marcar con documento lo que no lo tiene, o no marcar lo que
    /// sí; no avisar de que una lista la ve un cliente; un riel sin la
    /// cuenta de quien está dentro.
    #[gpui_kit::test]
    fn the_sidebar_says_what_each_thing_is(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let mut seen = space("a", "Producto");
        seen["lists"][0]["projectId"] = json!("p1");
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio")]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([seen]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({ "space:a": { "written": true }, "list:l-a": { "written": false } }))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.install(cx);
        let window = open(cx);

        wait_until(cx, "llegan el árbol y sus marcas", |cx| on_screen(window, "tree-l-a", cx) && on_screen(window, "tree-doc-a", cx));

        assert!(on_screen(window, "org-name", cx) && on_screen(window, "org-hint", cx));
        assert!(!on_screen(window, "tree-doc-l-a", cx), "marca con documento una lista que no lo tiene escrito");
        assert!(on_screen(window, "tree-client-l-a", cx), "no avisa de que la lista la ve un cliente");
        assert!(!on_screen(window, "tree-client-a", cx) && !on_screen(window, "tree-client-f-a", cx));
        assert!(on_screen(window, "account-menu", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    // ── Crear, renombrar y borrar nodos (command-and-control-376) ───────

    /// El servidor de siempre, que además acepta cualquier cambio en el árbol.
    fn server_accepting_edits() -> FakeServer {
        let server = server();
        accept_edits(&server);
        server
    }

    fn accept_edits(server: &FakeServer) {
        for verb in ["POST", "PATCH", "DELETE"] {
            server.mount(
                Mock::given(method(verb))
                    .and(wiremock::matchers::path_regex("^/api/v1/task-(spaces|folders|lists)/"))
                    .respond_with(ok(json!(null))),
            );
        }
    }

    /// Lo que se le pidió cambiar al servidor en el árbol: verbo, ruta y
    /// cuerpo, en orden.
    fn edits(server: &FakeServer) -> Vec<(String, String, Option<serde_json::Value>)> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() != "GET" && request.url.path().starts_with("/api/v1/task-"))
            .map(|request| (request.method.to_string(), request.url.path().to_string(), serde_json::from_slice(&request.body).ok()))
            .collect()
    }

    fn edit(verb: &str, route: &str, body: Option<serde_json::Value>) -> (String, String, Option<serde_json::Value>) {
        (verb.to_string(), route.to_string(), body)
    }

    /// Escribe en lo que tenga el foco.
    fn type_text(window: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input(text, cx);
        })
        .unwrap();
    }

    /// Abre el «···» de un nodo y pulsa una de sus entradas.
    fn choose(window: AnyWindowHandle, node: &str, action: &str, cx: &mut TestAppContext) {
        let item = format!("tree-act-{action}-{node}");
        click(window, &format!("tree-menu-{node}"), cx);
        wait_until(cx, "el menú de la fila se abre", |cx| on_screen(window, &item, cx));
        click(window, &item, cx);
    }

    /// A qué altura de la ventana está algo.
    fn top(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> f32 {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f32::from(window.try_find(id).expect("no está a la vista").bounds().origin.y)
        })
        .unwrap()
    }

    /// El «+» de «Espacios» abre un campo al final del árbol; Enter crea el
    /// espacio con lo escrito y deja el campo listo para otro; Escape lo
    /// cierra sin mandar nada, y un nombre en blanco no se manda.
    ///
    /// Roturas que atrapa: un «+» pintado que no hace nada; crear al pulsar
    /// el «+», sin haber escrito ni confirmado nada; un Escape que crea lo
    /// que había escrito; crear un espacio sin nombre; mandarlo a otra ruta
    /// o sin la organización; tener que volver a pulsar «+» para cada uno, o
    /// que el segundo arrastre el nombre del primero.
    #[gpui_kit::test]
    fn the_plus_of_spaces_names_a_new_space_where_it_will_be(cx: &mut TestAppContext) {
        let server = server_accepting_edits();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        assert!(!on_screen(window, "tree-name-input", cx), "hay un campo de nombre sin haberlo pedido");

        click(window, "spaces-new", cx);
        assert!(on_screen(window, "tree-name-input", cx), "el «+» no abrió el campo del nombre");
        assert!(top(window, "tree-name", cx) > top(window, "tree-l-a", cx), "el campo no está al final, donde nace un espacio");
        type_text(window, "Un borrador", cx);
        press(window, "escape", cx);
        assert!(!on_screen(window, "tree-name-input", cx), "Escape no cerró el campo");

        click(window, "spaces-new", cx);
        press(window, "enter", cx);
        type_text(window, "   ", cx);
        press(window, "enter", cx);
        settle_frames(window, cx);
        assert!(on_screen(window, "tree-name-input", cx), "un nombre en blanco cerró el campo");
        assert_eq!(edits(&server), [], "se mandó algo sin un nombre que mandar, o tras cancelar");

        type_text(window, "Diseño ", cx);
        press(window, "enter", cx);
        wait_until(cx, "se pide crear el espacio", |_| !edits(&server).is_empty());
        assert_eq!(edits(&server), [edit("POST", "/api/v1/task-spaces/", Some(json!({ "orgId": "o1", "name": "Diseño" })))]);

        // El campo sigue, vacío: el siguiente no arrastra el nombre anterior.
        wait_until(cx, "el campo queda listo para otro", |cx| {
            type_text(window, "", cx);
            on_screen(window, "tree-name-input", cx) && server.hits("/api/v1/task-spaces/") >= 3
        });
        settle_frames(window, cx);
        type_text(window, "Soporte", cx);
        press(window, "enter", cx);
        wait_until(cx, "se pide crear el segundo", |_| edits(&server).len() == 2);
        assert_eq!(edits(&server)[1], edit("POST", "/api/v1/task-spaces/", Some(json!({ "orgId": "o1", "name": "Soporte" }))));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El menú de un espacio o de una carpeta crea dentro: abre el nodo,
    /// pone el campo bajo lo que ya cuelga de él, y Enter lo manda por la
    /// ruta de su espacio. El de una lista no ofrece crear dentro.
    ///
    /// Roturas que atrapa: unas entradas de menú que no hacen nada; crear
    /// dentro de una carpeta plegada sin ver dónde nace; el campo en otro
    /// sitio que donde va a quedar la cosa; crear una lista donde se pidió
    /// una carpeta, o en el espacio lo que se pidió en la carpeta; ofrecer
    /// «lista nueva» dentro de una lista; que abrir el menú o crear acote
    /// «Mi trabajo».
    #[gpui_kit::test]
    fn the_row_menu_creates_inside_a_space_or_a_folder(cx: &mut TestAppContext) {
        let server = server_accepting_edits();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        assert!(!on_screen(window, "tree-in-a", cx));

        choose(window, "f-a", "new-folder", cx);
        assert!(on_screen(window, "tree-name-input", cx), "«Carpeta nueva» no abrió el campo");
        assert!(on_screen(window, "tree-in-a", cx), "la carpeta no se abrió para ver dónde nace");
        let field = top(window, "tree-name", cx);
        assert!(field > top(window, "tree-in-a", cx) && field < top(window, "tree-l-a", cx), "el campo no está dentro de la carpeta");
        settle_frames(window, cx);
        assert_eq!(edits(&server), [], "se creó algo al elegir la entrada, sin nombre ni confirmación");
        type_text(window, "Diseño", cx);
        press(window, "enter", cx);
        wait_until(cx, "se pide crear la carpeta", |_| !edits(&server).is_empty());
        assert_eq!(edits(&server), [edit("POST", "/api/v1/task-spaces/a/folders", Some(json!({ "name": "Diseño", "parentFolderId": "f-a" })))]);
        press(window, "escape", cx);

        choose(window, "a", "new-list", cx);
        assert!(top(window, "tree-name", cx) > top(window, "tree-l-a", cx), "el campo no está bajo lo último del espacio");
        type_text(window, "Un borrador", cx);
        press(window, "escape", cx);
        settle_frames(window, cx);
        assert_eq!(edits(&server).len(), 1, "Escape creó lo que había escrito");
        choose(window, "a", "new-list", cx);
        type_text(window, "Ideas", cx);
        press(window, "enter", cx);
        wait_until(cx, "se pide crear la lista", |_| edits(&server).len() == 2);
        assert_eq!(edits(&server)[1], edit("POST", "/api/v1/task-spaces/a/lists", Some(json!({ "name": "Ideas", "folderId": null }))));
        press(window, "escape", cx);
        assert!(!on_screen(window, "tree-name-input", cx));

        click(window, "tree-menu-l-a", cx);
        wait_until(cx, "el menú de la lista se abre", |cx| on_screen(window, "tree-act-rename-l-a", cx));
        assert!(on_screen(window, "tree-act-docs-l-a", cx) && on_screen(window, "tree-act-delete-l-a", cx));
        assert!(!on_screen(window, "tree-act-new-list-l-a", cx) && !on_screen(window, "tree-act-new-folder-l-a", cx), "una lista ofrece crear dentro");
        assert_eq!(scope(cx), None, "el menú o crear acotó «Mi trabajo»");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Renombrar pone el campo en la fila, con el nombre de ahora
    /// seleccionado: Enter guarda sólo si cambió y cierra; Escape cierra sin
    /// mandar nada.
    ///
    /// Roturas que atrapa: la fila pintada dos veces, con su campo al lado;
    /// un campo vacío que obliga a reescribir el nombre entero, o uno sin
    /// seleccionar en el que lo escrito se pega detrás del viejo; un Escape
    /// que renombra; gastar una petición para dejarlo igual; renombrar una
    /// lista por la ruta de otra clase de nodo; un campo que se queda
    /// abierto después de guardar; mandarlo dos veces con dos Enter seguidos.
    #[gpui_kit::test]
    fn renaming_happens_in_the_row(cx: &mut TestAppContext) {
        let server = server_accepting_edits();
        server.install(cx);
        let (window, workspace) = open_with_view(1100., cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        let row = top(window, "tree-l-a", cx);

        choose(window, "l-a", "rename", cx);
        assert!(on_screen(window, "tree-name-input", cx), "«Renombrar» no abrió el campo");
        assert!(!on_screen(window, "tree-l-a", cx), "la fila sigue pintada junto a su campo");
        assert!((top(window, "tree-name", cx) - row).abs() < 1., "el campo no está donde estaba la fila");
        type_text(window, "Un borrador", cx);
        press(window, "escape", cx);
        assert!(on_screen(window, "tree-l-a", cx) && !on_screen(window, "tree-name-input", cx), "Escape no devolvió la fila");

        // Enter sin tocar nada: es el nombre que ya tiene.
        choose(window, "l-a", "rename", cx);
        press(window, "enter", cx);
        wait_until(cx, "se cierra sin más", |cx| !on_screen(window, "tree-name-input", cx));
        settle_frames(window, cx);
        assert_eq!(edits(&server), [], "se mandó algo al cancelar, o para dejarlo igual");

        choose(window, "l-a", "rename", cx);
        type_text(window, "Entrada", cx);
        // Dos Enter seguidos, sin esperar al servidor ni a que se repinte:
        // se manda una vez. Llamando a lo que hace Enter y no pulsando la
        // tecla, porque entre dos pulsaciones la prueba repinta y el campo
        // ya sale apagado, que también lo para (se midió: quitando sólo la
        // guarda de `commit_name`, con teclas la prueba seguía en verde).
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.commit_name(window, cx);
                workspace.commit_name(window, cx);
            })
        })
        .unwrap();
        wait_until(cx, "se pide renombrar", |_| !edits(&server).is_empty());
        wait_until(cx, "guardado, el campo se cierra", |cx| !on_screen(window, "tree-name-input", cx));
        settle_frames(window, cx);
        assert_eq!(edits(&server), [edit("PATCH", "/api/v1/task-lists/l-a", Some(json!({ "name": "Entrada" })))]);

        choose(window, "f-a", "rename", cx);
        type_text(window, "Clientes", cx);
        press(window, "enter", cx);
        wait_until(cx, "se pide renombrar la carpeta", |_| edits(&server).len() == 2);
        wait_until(cx, "el campo se cierra", |cx| !on_screen(window, "tree-name-input", cx));
        choose(window, "a", "rename", cx);
        type_text(window, "Plataforma", cx);
        press(window, "enter", cx);
        wait_until(cx, "se pide renombrar el espacio", |_| edits(&server).len() == 3);
        assert_eq!(
            edits(&server)[1..],
            [
                edit("PATCH", "/api/v1/task-folders/f-a", Some(json!({ "name": "Clientes" }))),
                edit("PATCH", "/api/v1/task-spaces/a", Some(json!({ "name": "Plataforma" }))),
            ]
        );
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Borrar pregunta antes, nombrando el nodo y diciendo qué se va con
    /// él; «Cancelar» no borra, y el botón que borra dice qué borra.
    ///
    /// Roturas que atrapa: una entrada de menú que borra sin preguntar; una
    /// pregunta que no dice qué se borra ni cuánto se lleva; un «Cancelar»
    /// que borra igual; un botón de borrar que no hace nada, o que borra por
    /// la ruta de otra clase de nodo.
    #[gpui_kit::test]
    fn deleting_asks_first_naming_the_node_and_what_goes_with_it(cx: &mut TestAppContext) {
        let server = server_accepting_edits();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));

        choose(window, "l-a", "delete", cx);
        assert!(cx.has_pending_prompt(), "no preguntó antes de borrar");
        assert_eq!(
            cx.pending_prompt(),
            Some(("¿Borrar la lista «Pendientes»?".to_string(), "Borra sus 5 tareas y sus columnas. No se puede deshacer.".to_string()))
        );
        cx.simulate_prompt_answer("Cancelar");
        settle_frames(window, cx);
        assert_eq!(edits(&server), [], "borró aunque se canceló, o sin esperar la respuesta");

        choose(window, "a", "delete", cx);
        let (title, body) = cx.pending_prompt().expect("no preguntó antes de borrar el espacio");
        assert_eq!(title, "¿Borrar el espacio «Producto»?");
        assert!(body.ends_with("1 carpeta · 2 listas · 7 tareas"), "no dice qué se va con el espacio: {body}");
        cx.simulate_prompt_answer("Cancelar");
        choose(window, "f-a", "delete", cx);
        assert_eq!(cx.pending_prompt().map(|(title, _)| title).as_deref(), Some("¿Borrar la carpeta «Carpeta»?"));
        cx.simulate_prompt_answer("Cancelar");
        settle_frames(window, cx);
        assert_eq!(edits(&server), []);

        choose(window, "l-a", "delete", cx);
        cx.simulate_prompt_answer("Borrar la lista");
        wait_until(cx, "se pide borrarla", |_| !edits(&server).is_empty());
        assert_eq!(edits(&server), [edit("DELETE", "/api/v1/task-lists/l-a", None)]);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Una carpeta con carpetas dentro no se puede borrar: se explica por
    /// qué, sin botón de borrar, y no se manda nada.
    ///
    /// Roturas que atrapa: ofrecer «Borrar la carpeta» cuando el servidor
    /// dejaría a las de dentro fuera del árbol; callar por qué no se puede.
    #[gpui_kit::test]
    fn a_folder_with_folders_inside_explains_instead_of_asking(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let mut nested = space("a", "Producto");
        nested["folders"][0]["folders"] = json!([{ "id": "ff-a", "name": "Dentro", "folders": [], "lists": [] }]);
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio")]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([nested]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        accept_edits(&server);
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));

        choose(window, "f-a", "delete", cx);
        let (title, body) = cx.pending_prompt().expect("no dijo nada");
        assert_eq!(title, "¿Borrar la carpeta «Carpeta»?");
        assert_eq!(body, cac_core::text(cac_core::Locale::Es, "work:tree.folderHasFolders"));
        // Sólo se puede cerrar: si ofreciera borrar, «Cerrar» no estaría.
        cx.simulate_prompt_answer("Cerrar");
        settle_frames(window, cx);
        assert_eq!(edits(&server), []);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// A quien sólo puede mirar no se le ofrece cambiar el árbol: ni el «+»,
    /// ni crear, renombrar o borrar en el menú. La documentación sí.
    ///
    /// Roturas que atrapa: enseñar a un `viewer` acciones que el servidor le
    /// va a negar; quitarle de paso el camino a la documentación.
    #[gpui_kit::test]
    fn someone_who_can_only_look_gets_no_way_to_change_the_tree(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let mut looking = org("o1", "G Studio");
        looking["role"] = json!("viewer");
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([looking]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));

        assert!(!on_screen(window, "spaces-new", cx), "ofrece crear un espacio a quien sólo mira");
        click(window, "tree-menu-a", cx);
        wait_until(cx, "el menú se abre", |cx| on_screen(window, "tree-act-docs-a", cx));
        for action in ["new-folder", "new-list", "rename", "delete"] {
            assert!(!on_screen(window, &format!("tree-act-{action}-a"), cx), "ofrece {action} a quien sólo mira");
        }
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Lo que el servidor rechaza se dice con su motivo: bajo el campo, con
    /// lo escrito todavía en él; o sobre el árbol, si era un borrado.
    ///
    /// Roturas que atrapa: un cambio rechazado que parece hecho; perder lo
    /// escrito al fallar; un borrado que falla en silencio; un aviso que no
    /// se puede quitar.
    #[gpui_kit::test]
    fn a_change_the_server_refuses_is_said_and_what_was_typed_stays(cx: &mut TestAppContext) {
        let server = server();
        let refuse = |status: u16, code: &str| wiremock::ResponseTemplate::new(status).set_body_json(json!({ "success": false, "error": code, "message": "No" }));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/task-spaces/")).respond_with(refuse(403, "viewer-read-only")));
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/task-lists/l-a")).respond_with(refuse(409, "list-in-use-by-channel")));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));

        click(window, "spaces-new", cx);
        type_text(window, "Diseño", cx);
        press(window, "enter", cx);
        wait_until(cx, "dice por qué no se creó", |cx| on_screen(window, "tree-name-problem", cx));
        assert_eq!(label(window, "tree-name-problem", cx), "Aquí puedes mirar, pero no cambiar nada.");
        // Lo escrito sigue en el campo: otro Enter manda lo mismo.
        press(window, "enter", cx);
        wait_until(cx, "se vuelve a pedir", |_| edits(&server).len() == 2);
        assert_eq!(edits(&server)[1].2, Some(json!({ "orgId": "o1", "name": "Diseño" })), "lo escrito se perdió al fallar");
        press(window, "escape", cx);

        choose(window, "l-a", "delete", cx);
        cx.simulate_prompt_answer("Borrar la lista");
        wait_until(cx, "dice por qué no se borró", |cx| on_screen(window, "tree-problem", cx));
        assert_eq!(label(window, "tree-problem", cx), "Hay un canal apuntando a esta lista.");
        assert!(on_screen(window, "tree-l-a", cx));
        click(window, "tree-problem", cx);
        assert!(!on_screen(window, "tree-problem", cx), "el aviso no se quita");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Lo que un elemento dice de sí mismo: un contador, su número; un botón
    /// del riel, lo que enseña al pasar el ratón.
    fn label(window: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> String {
        let id = gpui_kit::SharedString::from(id.to_string());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().unwrap_or_default().to_string()
        })
        .unwrap()
    }

    const COUNTS: [&str; 4] = ["bell-count", "rail-comms-count", "rail-platform-count", "rail-organization-count"];

    /// La campana y el riel cuentan lo pendiente, cada uno lo suyo, y lo
    /// dicen también al pasar el ratón.
    ///
    /// Roturas que atrapa: los números del core que no llegan a la pantalla;
    /// un contador en el botón de otro (los servidores caídos en la
    /// comunicación); «12» en la campana o «120» en el riel, que no caben; un
    /// botón que no dice cuánto tiene, o que dice el número recortado.
    #[gpui_kit::test]
    fn the_bell_and_the_rail_count_what_is_pending(cx: &mut TestAppContext) {
        let server = server();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/notifications/").and(query_param("orgId", "o1")).respond_with(ok(json!({ "items": [], "unread": 12 }))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([{ "spaceId": "a", "count": 118 }]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([{ "conversationId": "c1", "orgId": "o1", "unread": 2 }]))));
        server.mount(get("/api/v1/servers/").respond_with(ok(json!([
            { "id": "s1", "orgId": "o1", "status": "offline" }, { "id": "s2", "orgId": "o1", "status": "online" },
        ]))));
        server.mount(get("/api/v1/invitations/").respond_with(ok(json!([{ "id": "i1" }, { "id": "i2" }, { "id": "i3" }]))));
        server.install(cx);
        let window = open(cx);

        wait_until(cx, "los cuatro contadores llegan a la pantalla", |cx| COUNTS.iter().all(|id| on_screen(window, id, cx)));

        assert_eq!(label(window, "bell-count", cx), "9+");
        assert_eq!(label(window, "rail-comms-count", cx), "99+");
        assert_eq!(label(window, "rail-platform-count", cx), "1");
        assert_eq!(label(window, "rail-organization-count", cx), "3");
        assert_eq!(label(window, "notifications-bell", cx), "Notificaciones (12)");
        assert_eq!(label(window, "rail-comms", cx), "Comunicación (120)");
        assert_eq!(label(window, "rail-platform", cx), "Plataforma (1)");
        assert_eq!(label(window, "rail-organization", cx), "Organización (3) · Todavía no está en esta app");
        assert!(!on_screen(window, "rail-docs-count", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un servidor como lo manda el backend. Con identidad: a uno sin ella
    /// la app le sondearía el agente, y aquí no hay agente que conteste.
    fn platform_server(id: &str, org_id: &str, name: &str, status: &str) -> serde_json::Value {
        json!({
            "id": id, "orgId": org_id, "name": name, "host": "203.0.113.7", "sshPort": 22, "sshUser": "deploy",
            "type": "docker-swarm", "agentPort": 8080, "status": status, "hasAgentToken": true, "agentVersion": 2,
        })
    }

    /// Pulsar Plataforma en el riel enseña su panel, con los servidores de
    /// la organización que se mira y su punto de estado, y la tabla; volver a
    /// «Trabajo» deja el árbol como estaba.
    ///
    /// Roturas que atrapa: el botón del riel que sigue apagado; los
    /// servidores de otra organización en el panel; un punto que no dice su
    /// estado; «Usuarios» a quien no administra la plataforma; quedarse en
    /// Plataforma al pulsar «Trabajo».
    #[gpui_kit::test]
    fn the_rail_opens_platform_with_the_servers_and_their_state(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/")).respond_with(ok(json!([
            platform_server("s1", "o1", "pixel-vps", "online"),
            platform_server("s2", "o1", "gstd", "offline"),
            platform_server("s3", "o2", "otro", "online"),
        ]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));

        click(window, "rail-platform", cx);
        wait_until(cx, "llegan los servidores", |cx| on_screen(window, "platform-server-s1", cx));
        assert!(on_screen(window, "platform-server-s2", cx));
        assert!(!on_screen(window, "platform-server-s3", cx), "un servidor de otra organización en el panel");
        assert_eq!(label(window, "platform-dot-s1", cx), "En línea");
        assert_eq!(label(window, "platform-dot-s2", cx), "Sin conexión");
        assert!(on_screen(window, "platform-row-s1", cx) && on_screen(window, "platform-row-s2", cx), "la tabla no enseña los servidores");
        assert!(!on_screen(window, "tree-l-a", cx), "el panel sigue siendo el árbol");
        assert!(on_screen(window, "platform-activity", cx));
        assert!(!on_screen(window, "platform-users", cx), "«Usuarios» a quien no es superadmin");

        click(window, "platform-server-s2", cx);
        assert_eq!(cx.update(|cx| Backend::core(cx).platform().state().open_server), Some("s2".into()));

        click(window, "rail-work", cx);
        wait_until(cx, "«Trabajo» devuelve el árbol", |cx| on_screen(window, "tree-l-a", cx));
        assert!(!on_screen(window, "platform-server-s1", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// ⌘N (Ctrl fuera del Mac) pulsa el icono del riel de ese puesto, como
    /// en `AppRail.tsx`, y el tooltip lo dice.
    ///
    /// Roturas que atrapa: un atajo que no lleva al módulo, o que cuenta los
    /// puestos por módulo con pantalla y no por icono (Docs es el 2).
    #[gpui_kit::test]
    fn ctrl_and_a_number_presses_that_rail_icon(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/")).respond_with(ok(json!([platform_server("s1", "o1", "pixel-vps", "online")]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));

        press(window, "secondary-4", cx);
        wait_until(cx, "llega Plataforma", |cx| on_screen(window, "platform-server-s1", cx));
        press(window, "secondary-1", cx);
        wait_until(cx, "vuelve el árbol", |cx| on_screen(window, "tree-l-a", cx));
        press(window, "secondary-3", cx);
        wait_until(cx, "⌘3 lleva a Comunicación", |cx| cx.update(|cx| Backend::snapshot(cx).read(cx).comms.on_screen));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un servidor de mentira donde la persona tiene ese papel en `o1`, con
    /// `pixel-vps` en la lista, y la app abierta en Plataforma.
    fn platform_as(role: &str, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let mut mine = org("o1", "G Studio");
        mine["role"] = json!(role);
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([mine]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/servers/").respond_with(ok(json!([platform_server("s1", "o1", "pixel-vps", "online")]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        click(window, "rail-platform", cx);
        wait_until(cx, "llega la tabla", |cx| on_screen(window, "platform-row-s1", cx));
        (server, window)
    }

    fn server_writes(server: &FakeServer) -> Vec<(String, String, serde_json::Value)> {
        server
            .requests()
            .iter()
            .filter(|request| request.method.as_str() != "GET" && request.url.path().starts_with("/api/v1/servers/"))
            .map(|request| (request.method.to_string(), request.url.path().to_string(), serde_json::from_slice(&request.body).unwrap_or_default()))
            .collect()
    }

    /// Un miembro da de alta desde el diálogo: se manda lo escrito con la
    /// organización, el diálogo se cierra y el nuevo sale en la tabla. Lo que
    /// no puede (borrar, kubernetes) no se le ofrece.
    ///
    /// Roturas que atrapa: un botón de alta que no abre nada; un diálogo que
    /// no manda lo escrito o no se cierra; ofrecer a un miembro lo que el
    /// servidor le va a negar.
    #[gpui_kit::test]
    fn a_member_adds_a_server_from_the_dialog(cx: &mut TestAppContext) {
        let (server, window) = platform_as("member", cx);
        assert!(on_screen(window, "platform-edit-s1", cx));
        assert!(!on_screen(window, "platform-delete-s1", cx), "ofrece borrar a un miembro");
        let created = json!({ "id": "s2", "orgId": "o1", "name": "prod-02", "host": "10.0.0.2", "sshPort": 22, "sshUser": "root",
            "type": "docker-swarm", "agentPort": 9090, "status": "pending", "hasAgentToken": true, "agentVersion": 2 });
        server.mount(Mock::given(method("POST")).and(path("/api/v1/servers/")).respond_with(wiremock::ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": created }))));

        click(window, "platform-add", cx);
        wait_until(cx, "se abre el diálogo", |cx| on_screen(window, "server-dialog", cx));
        assert!(!on_screen(window, "server-kind-kubernetes", cx), "ofrece kubernetes a quien no es superadmin");
        type_text(window, "prod-02", cx);
        click(window, "server-host", cx);
        type_text(window, "10.0.0.2", cx);
        click(window, "server-ssh-user", cx);
        type_text(window, "root", cx);
        click(window, "server-dialog-save", cx);

        wait_until(cx, "sale el nuevo", |cx| on_screen(window, "platform-row-s2", cx));
        assert!(!on_screen(window, "server-dialog", cx), "el diálogo se queda abierto");
        assert_eq!(
            server_writes(&server),
            [("POST".into(), "/api/v1/servers/".into(), json!({ "orgId": "o1", "name": "prod-02", "host": "10.0.0.2", "sshUser": "root", "sshPort": 22, "agentPort": 9090, "type": "docker-swarm" }))]
        );
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Lo que el core no deja mandar se dice en el diálogo, que sigue abierto
    /// con lo escrito.
    ///
    /// Roturas que atrapan: cerrar el diálogo, o callarse, cuando no se
    /// guardó; un fondo que deja pulsar el riel por debajo.
    #[gpui_kit::test]
    fn a_refused_edit_says_why_and_stays_open(cx: &mut TestAppContext) {
        let (server, window) = platform_as("member", cx);
        click(window, "platform-edit-s1", cx);
        wait_until(cx, "se abre el diálogo", |cx| on_screen(window, "server-dialog", cx));
        assert_eq!(label(window, "server-dialog-title", cx), "Editar pixel-vps");
        click(window, "rail-work", cx);
        assert!(cx.update(|cx| Backend::core(cx).platform().state().on_screen), "el riel se puede pulsar con el diálogo abierto");
        click(window, "server-host", cx);
        type_text(window, "9", cx);
        click(window, "server-dialog-save", cx);

        wait_until(cx, "dice por qué", |cx| on_screen(window, "server-dialog-problem", cx));
        assert!(on_screen(window, "server-dialog", cx));
        assert!(server_writes(&server).is_empty());
        click(window, "server-dialog-cancel", cx);
        wait_until(cx, "Cancelar lo cierra", |cx| !on_screen(window, "server-dialog", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// A quien sólo mira, ni alta, ni editar, ni borrar.
    ///
    /// Rotura que atrapa: enseñar a un lector acciones que el servidor le va
    /// a negar.
    #[gpui_kit::test]
    fn a_viewer_gets_no_server_actions(cx: &mut TestAppContext) {
        let (_server, window) = platform_as("viewer", cx);
        for id in ["platform-add", "platform-edit-s1", "platform-delete-s1"] {
            assert!(!on_screen(window, id, cx), "ofrece {id} a quien sólo mira");
        }
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Borrar pregunta nombrando el servidor; cancelar no manda nada, y
    /// confirmar lo quita de la tabla.
    ///
    /// Roturas que atrapa: borrar sin preguntar; una pregunta que no dice
    /// cuál; borrar al cancelar.
    #[gpui_kit::test]
    fn deleting_asks_naming_the_server(cx: &mut TestAppContext) {
        let (server, window) = platform_as("admin", cx);
        server.mount(Mock::given(method("DELETE")).and(path("/api/v1/servers/s1")).respond_with(ok(json!(null))));

        click(window, "platform-delete-s1", cx);
        let (title, body) = cx.pending_prompt().expect("borró sin preguntar");
        assert_eq!(title, "¿Borrar «pixel-vps»?");
        assert!(body.contains("se quedan intactos"), "{body}");
        cx.simulate_prompt_answer("Cancelar");
        cx.run_until_parked();
        // El borrado saldría por el runtime de tokio, no por el de la
        // ventana: hay que darle tiempo de llegar para poder decir que no
        // llegó.
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(server_writes(&server).is_empty(), "borró al cancelar");

        click(window, "platform-delete-s1", cx);
        cx.simulate_prompt_answer("Borrar");
        wait_until(cx, "sale de la tabla", |cx| !on_screen(window, "platform-row-s1", cx));
        assert_eq!(server_writes(&server).iter().map(|(verb, route, _)| (verb.as_str(), route.as_str())).collect::<Vec<_>>(), [("DELETE", "/api/v1/servers/s1")]);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un desplegable de nombre largo se recorta dentro de la tarjeta del
    /// resumen en vez de empujar su modo fuera (boaty, 22-nov).
    ///
    /// Rotura que atrapa: que el nombre o el entorno no se dejen recortar y
    /// «Apuntar» se pinte por fuera del borde de la tarjeta.
    #[gpui_kit::test]
    fn a_long_deployable_stays_inside_its_card(cx: &mut TestAppContext) {
        let (server, window) = platform_as("member", cx);
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/servers/s1/incidents").respond_with(ok(json!({ "incidents": [] }))));
        server.mount(get("/api/v1/servers/s1/deployables").respond_with(ok(json!([{
            "id": "d1",
            "name": "web boaty sync-worker prod (postgres) with a name that goes on and on",
            "environment": "production",
            "currentImage": "ghcr.io/o/sync-worker:4f1c2a9e0b7d",
            "onCINotify": "record"
        }]))));
        server.mount(get("/api/v1/servers/s1/provisioning-runs").respond_with(ok(json!([]))));

        click(window, "platform-open-s1", cx);
        wait_until(cx, "llega el desplegable", |cx| on_screen(window, "server-deployable-mode-d1", cx));
        let (card_x, _, card_width) = place_on_screen(window, "server-deploys", cx);
        let (mode_x, _, mode_width) = place_on_screen(window, "server-deployable-mode-d1", cx);
        assert!(mode_width > 0.0, "el modo no se pinta");
        assert!(mode_x + mode_width <= card_x + card_width, "el modo se sale de la tarjeta: {} > {}", mode_x + mode_width, card_x + card_width);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Abrir un servidor desde la tabla enseña su página: cabecera,
    /// pestañas y el resumen con lo que ha mordido, sus despliegues y cómo se
    /// llega a él; una pestaña que aún no está lo dice; volver deja la tabla.
    ///
    /// Roturas que atrapan: un nombre que no abre nada; un resumen que no
    /// pinta lo que llegó; pestañas que no cambian; no poder volver.
    #[gpui_kit::test]
    fn opening_a_server_shows_its_page_and_overview(cx: &mut TestAppContext) {
        let (server, window) = platform_as("member", cx);
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/servers/s1/incidents").respond_with(ok(json!({ "incidents": [
            { "id": "i1", "title": "cert-manager no renueva", "status": "open", "servers": [], "areas": ["cert-manager"], "recordedAt": "2026-11-01T10:00:00Z" }
        ] }))));
        server.mount(get("/api/v1/servers/s1/deployables").respond_with(ok(json!([{ "id": "d1", "name": "api", "currentImage": "ghcr.io/o/api:abc1234", "onCINotify": "deploy" }]))));
        server.mount(get("/api/v1/servers/s1/provisioning-runs").respond_with(ok(json!([]))));

        click(window, "platform-open-s1", cx);
        wait_until(cx, "llega el resumen", |cx| on_screen(window, "server-incident-i1", cx));
        assert_eq!(label(window, "server-title", cx), "pixel-vps");
        assert_eq!(label(window, "server-incident-i1", cx), "cert-manager no renueva");
        assert!(on_screen(window, "server-deployable-d1", cx));
        assert!(on_screen(window, "server-provisioning-empty", cx), "sin ejecuciones no dice cómo empezar");
        assert!(on_screen(window, "server-tab-backups", cx), "un swarm sin sus ocho pestañas");
        assert!(!on_screen(window, "server-agent-install", cx), "a un miembro, instalar el agente que sólo puede un admin");

        click(window, "server-tab-backups", cx);
        wait_until(cx, "la pestaña abre su vista", |cx| on_screen(window, "server-backups", cx) || on_screen(window, "backups-loading", cx));
        assert!(!on_screen(window, "server-tab-pending", cx), "una pestaña de un swarm dice que todavía no está");
        click(window, "server-back", cx);
        wait_until(cx, "vuelve la tabla", |cx| on_screen(window, "platform-row-s1", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un kubernetes no enseña las pestañas de swarm ni la tarjeta del
    /// agente de swarm.
    ///
    /// Rotura que atrapa: ofrecer servicios, nodos o el agente en un
    /// servidor que no los tiene.
    #[gpui_kit::test]
    fn a_kubernetes_server_shows_no_swarm_tabs(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([org("o1", "G Studio")]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        let mut kubernetes = platform_server("k1", "o1", "k8s-contabo", "pending");
        kubernetes["type"] = json!("kubernetes");
        server.mount(get("/api/v1/servers/").respond_with(ok(json!([kubernetes]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        click(window, "rail-platform", cx);
        wait_until(cx, "llega el panel", |cx| on_screen(window, "platform-server-k1", cx));

        click(window, "platform-server-k1", cx);
        wait_until(cx, "llega el resumen", |cx| on_screen(window, "server-overview", cx));
        assert!(on_screen(window, "server-tab-provision", cx));
        for id in ["server-tab-services", "server-tab-nodes", "server-agent", "server-deploys"] {
            assert!(!on_screen(window, id, cx), "un kubernetes enseña {id}");
        }
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un clúster con su consola, visto con este papel desde la tabla de
    /// servidores.
    fn cluster_as(role: &str, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let mut mine = org("o1", "G Studio");
        mine["role"] = json!(role);
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([mine]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        let mut kubernetes = platform_server("k1", "o1", "k8s-contabo", "pending");
        kubernetes["type"] = json!("kubernetes");
        server.mount(get("/api/v1/servers/").respond_with(ok(json!([kubernetes]))));
        for what in ["incidents", "deployables", "provisioning-runs"] {
            server.mount(get(&format!("/api/v1/servers/k1/{what}")).respond_with(ok(json!([]))));
        }
        server.mount(get("/api/v1/servers/k1/k8s/routes").respond_with(ok(json!({
            "gateways": [{ "name": "gw", "namespace": "default", "address": "203.0.113.7", "programmed": true }],
            "routes": [{ "name": "cac", "namespace": "cac", "gateway": "gw", "hostnames": ["cac.dev"], "links": ["https://cac.dev"], "certReady": true }],
        }))));
        server.mount(get("/api/v1/servers/k1/k8s/health").respond_with(ok(json!({
            "nodes": [{ "name": "vmi1", "kubeletVersion": "v1.35.10", "ready": true }],
            // Llegan con el que falla detrás: la consola lo sube.
            "workloads": [{ "name": "api", "namespace": "cac", "ready": 1, "desired": 1, "healthy": true }, { "name": "web", "namespace": "cac", "ready": 1, "desired": 2, "healthy": false }],
            "certs": [{ "name": "cac-tls", "namespace": "cac", "dnsNames": ["cac.dev"], "daysLeft": 12, "ready": true }],
            "datastores": [{ "kind": "postgres-cnpg", "name": "postgres", "namespace": "data", "instances": 1, "ready": 1, "phase": "Cluster in healthy state", "healthy": true }],
        }))));
        server.mount(get("/api/v1/servers/k1/integrations").respond_with(ok(json!([
            { "id": "g", "serverId": "k1", "kind": "grafana", "name": "Grafana", "url": "http://grafana.monitoring:3000", "authMethod": "header", "hasSecret": false },
            { "id": "p", "serverId": "k1", "kind": "pgadmin", "name": "pgAdmin", "url": "https://pg.dev", "authMethod": "none", "hasSecret": true },
        ]))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/servers/k1/integrations/p/reveal")).respond_with(ok(json!({ "secret": "admin:hunter2" }))));
        server.mount(Mock::given(method("POST")).and(path("/api/v1/servers/k1/integrations/g/launch")).respond_with(ok(json!({ "path": "/t/x", "url": "https://tools.guz-studio.dev/t/x" }))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        click(window, "rail-platform", cx);
        wait_until(cx, "llega la tabla", |cx| on_screen(window, "platform-row-k1", cx));
        (server, window)
    }

    /// La consola de un clúster: su etiqueta en la lista, y al abrirlo sus
    /// rutas, salud, cargas, certificados e integraciones; un miembro ve el
    /// secreto guardado y Grafana se abre por el proxy de cac.
    ///
    /// Roturas que atrapa: un clúster en «pendiente» en la lista; una consola
    /// que no se lee o se pinta a medias; un certificado a punto de caducar
    /// sin aviso; un secreto que no se enseña; Grafana abierto sin pasar por
    /// cac.
    #[gpui_kit::test]
    fn a_cluster_shows_its_badge_and_its_console(cx: &mut TestAppContext) {
        let (server, window) = cluster_as("member", cx);
        wait_until(cx, "la etiqueta del clúster", |cx| on_screen(window, "cluster-badge-k1", cx) && label(window, "cluster-badge-k1", cx) == "1 problema");
        assert_eq!(label(window, "platform-dot-k1", cx), "1 problema", "el panel dice el estado del agente, que un clúster no lleva");

        click(window, "platform-open-k1", cx);
        wait_until(cx, "llega la consola", |cx| on_screen(window, "k8s-route-cac.dev", cx));
        assert!(on_screen(window, "server-incidents", cx), "lo que ha mordido sigue arriba");
        assert_eq!(label(window, "k8s-gateway-default-gw", cx), "gw programado");
        assert_eq!(label(window, "k8s-node-vmi1", cx), "vmi1 · v1.35.10");
        assert_eq!(label(window, "k8s-cert-cac-cac-tls", cx), "cac-tls !", "a doce días no avisa");
        let place = |id: &str, cx: &mut TestAppContext| {
            let (x, y, _) = place_on_screen(window, id, cx);
            (y, x)
        };
        assert!(place("k8s-workload-cac-web", cx) < place("k8s-workload-cac-api", cx), "lo que falla, primero");
        assert!(on_screen(window, "k8s-store-data-postgres", cx));
        assert!(!on_screen(window, "k8s-reveal-g", cx), "una sin secreto ofrece revelarlo");

        click(window, "k8s-reveal-p", cx);
        wait_until(cx, "se ve el secreto", |cx| on_screen(window, "k8s-secret-p", cx));
        assert_eq!(label(window, "k8s-secret-p", cx), "admin:hunter2");

        click(window, "k8s-open-g", cx);
        wait_until(cx, "pide el enlace al proxy", |_| server.hits("/api/v1/servers/k1/integrations/g/launch") == 1);
        assert!(!on_screen(window, "k8s-error", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Rotura que atrapa: ofrecer a un lector el secreto guardado de una
    /// integración.
    #[gpui_kit::test]
    fn a_reader_is_not_offered_the_secret(cx: &mut TestAppContext) {
        let (server, window) = cluster_as("viewer", cx);
        click(window, "platform-open-k1", cx);
        wait_until(cx, "llega la consola", |cx| on_screen(window, "k8s-open-p", cx));
        assert!(!on_screen(window, "k8s-reveal-p", cx));
        assert_eq!(server.hits("/api/v1/servers/k1/integrations/p/reveal"), 0);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un swarm cuyo agente es el mismo servidor de mentira (sin
    /// identidad: no pide pase), con dos servicios, abierto en su pestaña.
    fn services_as(role: &str, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
        let (server, window) = swarm_server_as(role, json!([]), cx);
        click(window, "server-tab-services", cx);
        wait_until(cx, "llegan los servicios", |cx| on_screen(window, "service-row-svc1", cx));
        (server, window)
    }

    /// Un swarm abierto en su resumen, con estos nodos, cuyo agente es el
    /// servidor de mentira.
    fn swarm_server_as(role: &str, nodes: serde_json::Value, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
        swarm_servers_as(role, nodes, &["s1"], cx)
    }

    /// Lo mismo con varios swarms, todos con el mismo agente; abierto el
    /// primero.
    fn swarm_servers_as(role: &str, nodes: serde_json::Value, ids: &[&str], cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
        swarm_servers_with(role, nodes, ids, false, json!([]), cx)
    }

    /// Con `identity`, el agente tiene la suya (v14) y da pases: lo que
    /// despliega lo pide.
    fn swarm_servers_with(role: &str, nodes: serde_json::Value, ids: &[&str], identity: bool, deployables: serde_json::Value, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let mut mine = org("o1", "G Studio");
        mine["role"] = json!(role);
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([mine]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        let rows: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| {
                let name = if *id == "s1" { "pixel-vps".to_string() } else { format!("vps-{id}") };
                let mut vps = platform_server(id, "o1", &name, "online");
                (vps["host"], vps["agentPort"], vps["hasAgentToken"]) = (json!("127.0.0.1"), json!(server.port()), json!(identity));
                if identity {
                    vps["agentVersion"] = json!(14);
                }
                vps
            })
            .collect();
        server.mount(get("/api/v1/servers/").respond_with(ok(json!(rows))));
        server.mount(get("/api/v1/nodes").respond_with(ok(nodes)));
        for id in ids {
            if identity {
                let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
                server.mount(Mock::given(method("POST")).and(path(format!("/api/v1/servers/{id}/agent-session"))).respond_with(ok(json!({ "token": "p", "expiresAt": expires_at }))));
            }
            for what in ["incidents", "provisioning-runs"] {
                server.mount(get(&format!("/api/v1/servers/{id}/{what}")).respond_with(ok(json!([]))));
            }
            server.mount(get(&format!("/api/v1/servers/{id}/deployables")).respond_with(ok(deployables.clone())));
        }
        server.mount(get("/api/v1/services").respond_with(ok(json!([
            { "id": "svc1", "name": "shop_web", "image": "ghcr.io/o/web:v3", "stack": "shop", "replicas": { "running": 1, "desired": 2 }, "updatedAt": "2026-11-20T10:00:00Z" },
            { "id": "svc2", "name": "shop_db", "image": "postgres:16", "stack": "shop", "replicas": { "running": 1, "desired": 1 }, "updatedAt": "2026-11-20T10:00:00Z" },
        ]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        click(window, "rail-platform", cx);
        wait_until(cx, "llega la tabla", |cx| on_screen(window, "platform-row-s1", cx));
        click(window, "platform-open-s1", cx);
        wait_until(cx, "se abre", |cx| on_screen(window, "server-tab-services", cx));
        (server, window)
    }

    fn two_nodes() -> serde_json::Value {
        json!([
            { "id": "n1", "hostname": "manager-1", "role": "manager", "status": "ready", "availability": "active", "engineVersion": "27.3.1" },
            { "id": "n2", "hostname": "worker-2", "role": "worker", "status": "down", "availability": "drain", "engineVersion": "27.3.1" },
        ])
    }

    /// La pestaña de nodos pide los suyos al agente y los enseña.
    ///
    /// Rotura que atrapa: una pestaña que no pide los nodos, o que se queda
    /// en «todavía no está».
    #[gpui_kit::test]
    fn the_nodes_tab_lists_the_swarm_nodes(cx: &mut TestAppContext) {
        let (_server, window) = swarm_server_as("viewer", two_nodes(), cx);
        click(window, "server-tab-nodes", cx);
        wait_until(cx, "llegan los nodos", |cx| on_screen(window, "node-row-n1", cx));
        assert_eq!(label(window, "node-row-n2", cx), "worker-2");
        assert!(!on_screen(window, "server-tab-pending", cx));
    }

    /// La pestaña de recursos: los parados ocultos hasta pedirlos, el filtro,
    /// el orden por columna, los nombres de los nodos y la pausa.
    ///
    /// Roturas que atrapa: enseñar los parados sin pedirlos o no poder
    /// verlos; un filtro que no filtra; una columna que no ordena o no da
    /// la vuelta; el nodo por su identificador cuando se sabe su nombre; una
    /// pausa que no se nota.
    #[gpui_kit::test]
    fn the_resources_tab_hides_the_stopped_filters_sorts_and_pauses(cx: &mut TestAppContext) {
        let (server, window) = swarm_server_as("viewer", two_nodes(), cx);
        let container = |id: &str, service: &str, state: &str, cpu: f64, mem: u64| {
            json!({ "containerId": id, "nodeId": "n1", "serviceName": service, "stack": "shop", "state": state, "cpuPercent": cpu, "onlineCpus": 2, "memUsage": mem, "memLimit": 1000 })
        };
        server.mount(Mock::given(method("GET")).and(path("/api/v1/stats")).respond_with(ok(json!([
            container("aaa", "shop_web", "running", 10., 900),
            container("bbb", "shop_db", "running", 90., 100),
            container("ccc", "old_job", "exited", 0., 0),
        ]))));
        click(window, "server-tab-stats", cx);
        wait_until(cx, "llegan los contenedores", |cx| on_screen(window, "stats-row-aaa", cx));
        assert!(!on_screen(window, "stats-row-ccc", cx));
        assert_eq!(label(window, "stats-count", cx), "Contenedores (2 de 3)");
        assert_eq!(label(window, "stats-hidden", cx), "1 contenedor parado oculto — enseñarlos");
        assert!(top(window, "stats-row-bbb", cx) < top(window, "stats-row-aaa", cx), "por CPU, el que más gasta arriba");
        wait_until(cx, "los nodos ponen nombre", |cx| on_screen(window, "stats-node-aaa", cx) && label(window, "stats-node-aaa", cx) == "manager-1");

        click(window, "stats-sort-Memory", cx);
        wait_until(cx, "por memoria", |cx| top(window, "stats-row-aaa", cx) < top(window, "stats-row-bbb", cx));
        click(window, "stats-sort-Memory", cx);
        wait_until(cx, "la misma columna le da la vuelta", |cx| top(window, "stats-row-bbb", cx) < top(window, "stats-row-aaa", cx));

        click(window, "stats-hidden", cx);
        wait_until(cx, "se ven los parados", |cx| on_screen(window, "stats-row-ccc", cx));
        click(window, "stats-filter", cx);
        type_text(window, "OLD", cx);
        wait_until(cx, "el filtro deja uno", |cx| !on_screen(window, "stats-row-aaa", cx));
        assert!(on_screen(window, "stats-row-ccc", cx));

        assert!(label(window, "stats-status", cx).starts_with("Cada 5 s"));
        click(window, "stats-pause", cx);
        wait_until(cx, "se nota la pausa", |cx| label(window, "stats-status", cx).starts_with("En pausa"));
    }

    /// Lo que una tabla recorta se copia entero de un clic: el nombre y la
    /// imagen de un servicio, y el id completo de un contenedor aunque se vea
    /// el corto.
    ///
    /// Roturas que atrapa: que un clic no copie; que copie lo que se ve
    /// recortado o el id corto; que copie la celda de otra fila.
    #[gpui_kit::test]
    fn a_clipped_cell_copies_its_whole_text(cx: &mut TestAppContext) {
        let (server, window) = services_as("viewer", cx);
        click(window, "service-image-svc1", cx);
        assert_eq!(copied(cx).as_deref(), Some("ghcr.io/o/web:v3"));
        click(window, "service-name-svc1", cx);
        assert_eq!(copied(cx).as_deref(), Some("shop_web"));

        let full = "0123456789abcdef0123456789abcdef";
        server.mount(Mock::given(method("GET")).and(path("/api/v1/stats")).respond_with(ok(json!([
            { "containerId": full, "nodeId": "n1", "serviceName": "shop_web", "stack": "shop", "state": "running", "cpuPercent": 1., "onlineCpus": 2, "memUsage": 1, "memLimit": 10 }
        ]))));
        click(window, "server-tab-stats", cx);
        let cell = format!("stats-container-{full}");
        wait_until(cx, "llega el contenedor", |cx| on_screen(window, &cell, cx));
        click(window, &cell, cx);
        assert_eq!(copied(cx).as_deref(), Some(full));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// La pestaña de servicios: filtrar, reiniciar y seguir el log, que
    /// llega con sus colores y sin códigos crudos.
    ///
    /// Roturas que atrapa: un filtro que no filtra o no se puede vaciar; un
    /// reinicio que no llega al agente o no dice nada; un log que no se
    /// pinta, o que pinta `[32m`.
    #[gpui_kit::test]
    fn the_services_tab_filters_restarts_and_follows_a_log(cx: &mut TestAppContext) {
        let (server, window) = services_as("member", cx);
        server.mount(
            Mock::given(method("GET"))
                .and(path("/api/v1/services/svc1/logs"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw("data: \u{1b}[32mconexión\u{1b}[0m lista\n\n", "text/event-stream")),
        );
        server.mount(Mock::given(method("POST")).and(path("/api/v1/services/svc1/force-update")).respond_with(ok(json!({ "message": "service update triggered" }))));
        assert!(on_screen(window, "service-row-svc2", cx));

        click(window, "services-filter", cx);
        type_text(window, "POSTGRES", cx);
        wait_until(cx, "el filtro deja uno", |cx| !on_screen(window, "service-row-svc1", cx));
        assert!(on_screen(window, "service-row-svc2", cx));
        assert_eq!(label(window, "services-count", cx), "Mostrando 1 de 2");
        click(window, "services-filter-clear", cx);
        wait_until(cx, "vaciar el filtro los trae", |cx| on_screen(window, "service-row-svc1", cx));
        assert!(!on_screen(window, "services-count", cx));

        click(window, "service-restart-svc1", cx);
        wait_until(cx, "dice que reinicia", |cx| on_screen(window, "services-restart-result", cx));
        assert_eq!(label(window, "services-restart-result", cx), "Reiniciando shop_web");
        assert_eq!(server.hits("/api/v1/services/svc1/force-update"), 1);

        click(window, "service-logs-svc1", cx);
        wait_until(cx, "llega el log", |cx| on_screen(window, "service-log-line-0", cx));
        assert_eq!(label(window, "service-log-line-0", cx), "conexión lista");
        click(window, "service-logs-close", cx);
        wait_until(cx, "se cierra", |cx| !on_screen(window, "service-logs", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Rotura que atrapa: que la pestaña de servicios sea una para todos los
    /// servidores y el filtro de uno aparezca en el siguiente.
    #[gpui_kit::test]
    fn each_server_has_its_own_services_tab(cx: &mut TestAppContext) {
        let (_server, window) = swarm_servers_as("viewer", json!([]), &["s1", "s2"], cx);
        click(window, "server-tab-services", cx);
        wait_until(cx, "llegan los servicios", |cx| on_screen(window, "service-row-svc1", cx));
        click(window, "services-filter", cx);
        type_text(window, "POSTGRES", cx);
        wait_until(cx, "el filtro deja uno", |cx| !on_screen(window, "service-row-svc1", cx));

        click(window, "server-back", cx);
        wait_until(cx, "vuelve la tabla", |cx| on_screen(window, "platform-open-s2", cx));
        click(window, "platform-open-s2", cx);
        wait_until(cx, "se abre el otro", |cx| on_screen(window, "server-tab-services", cx));
        click(window, "server-tab-services", cx);
        wait_until(cx, "llegan sus servicios, sin filtro", |cx| on_screen(window, "service-row-svc1", cx));
        assert!(!on_screen(window, "services-count", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un swarm con agente y `shop_web` registrado: dos commits (el de
    /// abajo corre; el de arriba tiene imagen publicada con el sha corto) y
    /// un despliegue de Ana que se puede deshacer.
    fn deploys_as(role: &str, cx: &mut TestAppContext) -> (FakeServer, AnyWindowHandle) {
        let web = json!({ "id": "d1", "orgId": "o1", "serverId": "s1", "name": "web", "stack": "shop", "serviceName": "shop_web", "imageRepo": "ghcr.io/o/web", "repoFullName": "o/web", "onCINotify": "record", "currentImage": "ghcr.io/o/web:aaa1111" });
        let (server, window) = swarm_servers_with(role, json!([]), &["s1"], true, json!([web]), cx);
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/servers/s1/deployables/d1/deployments").respond_with(ok(json!([
            { "id": "x1", "createdAt": "2026-11-21T10:00:00Z", "deployableId": "d1", "serverId": "s1", "image": "ghcr.io/o/web:aaa1111", "previousImage": "ghcr.io/o/web:9990000", "requestedBy": "user", "requestedByName": "Ana", "status": "succeeded" },
        ]))));
        server.mount(get("/api/v1/servers/s1/deployables/d1/builds").respond_with(ok(json!([{ "id": "b1", "deployableId": "d1", "sha": "bbb2222", "image": "ghcr.io/o/web:bbb2222", "source": "ci" }]))));
        server.mount(get("/api/v1/servers/s1/deployables/d1/commits").respond_with(ok(json!([
            { "sha": "bbb2222ffffffff", "message": "arregla el login", "author": "ana", "date": "2026-11-21T09:00:00Z" },
            { "sha": "aaa1111ffffffff", "message": "lo que corre", "author": "luis", "date": "2026-11-20T09:00:00Z" },
        ]))));
        click(window, "server-tab-deploy", cx);
        wait_until(cx, "salen los servicios", |cx| on_screen(window, "deploy-service-svc1", cx));
        (server, window)
    }

    /// La pestaña de despliegue: elegir un servicio registrado enseña sus
    /// commits con el publicado elegido, desplegarlo manda el tag con el que
    /// se publicó y abre su log; volver atrás pregunta.
    ///
    /// Roturas que atrapa: desplegar otro sha (o el entero, que no existe
    /// como tag); no abrir el log del nuevo; volver atrás sin preguntar o
    /// mandando otro despliegue.
    #[gpui_kit::test]
    fn a_member_deploys_the_published_commit_and_rolls_back_asking(cx: &mut TestAppContext) {
        let (server, window) = deploys_as("member", cx);
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/servers/s1/deployables/d1/deploy"))
                .and(wiremock::matchers::body_json(json!({ "sha": "bbb2222" })))
                .respond_with(ok(json!({ "id": "x2", "createdAt": "2026-11-21T11:00:00Z", "deployableId": "d1", "serverId": "s1", "image": "ghcr.io/o/web:bbb2222", "requestedBy": "user", "status": "queued" }))),
        );
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/s1/deployables/d1/deployments/x2")).respond_with(ok(json!({ "id": "x2", "log": "pull ghcr.io/o/web:bbb2222\n" }))));
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/servers/s1/deployables/d1/deployments/x1/rollback"))
                .respond_with(ok(json!({ "id": "x3", "createdAt": "2026-11-21T12:00:00Z", "deployableId": "d1", "serverId": "s1", "image": "ghcr.io/o/web:9990000", "requestedBy": "user", "status": "queued" }))),
        );
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/s1/deployables/d1/deployments/x3")).respond_with(ok(json!({ "id": "x3", "log": "" }))));

        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "llegan los commits", |cx| on_screen(window, "deploy-commit-bbb2222", cx));
        assert_eq!(label(window, "deploy-behind", cx), "1 sin desplegar");
        assert!(on_screen(window, "deploy-row-x1", cx));
        assert!(!on_screen(window, "deploy-settings", cx), "los ajustes son de quien administra");

        click(window, "deploy-rollback-x1", cx);
        let (title, _) = cx.pending_prompt().expect("volvió atrás sin preguntar");
        assert_eq!(title, "¿Volver a lo que había antes de este deploy?");
        cx.simulate_prompt_answer("Cancelar");
        cx.run_until_parked();

        click(window, "deploy-go", cx);
        wait_until(cx, "se abre el log del nuevo", |cx| on_screen(window, "deploy-log-line-0", cx));
        assert_eq!(label(window, "deploy-log-line-0", cx), "pull ghcr.io/o/web:bbb2222");
        assert_eq!(server.hits("/api/v1/servers/s1/deployables/d1/deploy"), 1);
        assert_eq!(server.hits("/api/v1/servers/s1/deployables/d1/deployments/x1/rollback"), 0, "volvió atrás al cancelar");

        // El nuevo termina, por el stream, y se puede volver atrás del de antes.
        let finished = cac_core::ServerEvent {
            event: "deploy:status".into(),
            data: json!({ "id": "x2", "deployableId": "d1", "serverId": "s1", "image": "ghcr.io/o/web:bbb2222", "requestedBy": "user", "status": "failed" }).to_string(),
        };
        cx.update(|cx| Backend::run(cx, move |core| async move { core.stream().deliver(finished) }));
        wait_until(cx, "el log dice cómo acabó", |cx| label(window, "deploy-log-status", cx) == "Falló");
        click(window, "deploy-log-back", cx);
        wait_until(cx, "vuelve el historial", |cx| on_screen(window, "deploy-row-x2", cx));
        click(window, "deploy-rollback-x1", cx);
        cx.simulate_prompt_answer("Volver a lo de antes");
        wait_until(cx, "vuelve atrás", |_| server.hits("/api/v1/servers/s1/deployables/d1/deployments/x1/rollback") == 1);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Rotura que atrapa: ofrecerle desplegar o volver atrás a un lector, o
    /// registrar un servicio a quien no administra; o que el botón de una
    /// fila de Servicios no lleve a su despliegue.
    #[gpui_kit::test]
    fn a_reader_sees_what_was_deployed_but_cannot_deploy(cx: &mut TestAppContext) {
        let (_server, window) = deploys_as("viewer", cx);
        // Desde su fila en Servicios se llega con él elegido.
        click(window, "server-tab-services", cx);
        wait_until(cx, "llegan los servicios", |cx| on_screen(window, "service-deploy-svc1", cx));
        click(window, "service-deploy-svc1", cx);
        wait_until(cx, "llegan los commits", |cx| on_screen(window, "deploy-commit-bbb2222", cx));
        assert!(on_screen(window, "deploy-row-x1", cx));
        assert!(!on_screen(window, "deploy-go", cx));
        assert!(!on_screen(window, "deploy-rollback-x1", cx));
        click(window, "deploy-service-svc2", cx);
        wait_until(cx, "el otro no está registrado", |cx| on_screen(window, "deploy-not-yet", cx));
        assert!(!on_screen(window, "deploy-register", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El PAT de mentira: apunta lo que se guardaría en GitHub.
    #[derive(Default)]
    struct FakeRepoSecrets {
        configured: bool,
        put: std::sync::Mutex<Vec<(String, String, String, String)>>,
    }

    impl crate::platform::deploy_settings::RepoSecrets for FakeRepoSecrets {
        fn configured(&self) -> bool {
            self.configured
        }
        fn put(&self, owner: String, repo: String, name: String, value: String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
            self.put.lock().unwrap().push((owner, repo, name, value));
            Box::pin(async { Ok(()) })
        }
    }

    fn with_repo_secrets(configured: bool, cx: &mut TestAppContext) -> std::sync::Arc<FakeRepoSecrets> {
        let fake = std::sync::Arc::new(FakeRepoSecrets { configured, ..Default::default() });
        let shared: std::sync::Arc<dyn crate::platform::deploy_settings::RepoSecrets> = fake.clone();
        cx.update(|cx| cx.set_global(crate::platform::deploy_settings::LocalRepoSecrets(shared)));
        fake
    }

    fn mount_mint(server: &FakeServer) {
        server.mount(Mock::given(method("POST")).and(path("/api/v1/servers/s1/deployables/d1/ci-key")).respond_with(ok(json!({ "key": "cac_dk_ENTERA", "preview": "cac_dk_EN" }))));
    }

    /// Sin PAT, la llave del CI se enseña una vez y no vuelve al dejar el
    /// servicio; con PAT, va sola al repo y no se enseña.
    ///
    /// Roturas que atrapa: guardar la llave en otro repo u otro secret;
    /// enseñarla habiéndola guardado; que siga a la vista al volver.
    #[gpui_kit::test]
    fn the_ci_key_goes_to_the_repo_or_is_shown_once(cx: &mut TestAppContext) {
        let secrets = with_repo_secrets(false, cx);
        let (server, window) = deploys_as("admin", cx);
        mount_mint(&server);
        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "salen los ajustes", |cx| on_screen(window, "deploy-ci-mint", cx));
        reveal(window, "server-body", "deploy-ci-mint", cx);
        click(window, "deploy-ci-mint", cx);
        wait_until(cx, "se enseña la llave", |cx| on_screen(window, "deploy-ci-key", cx));
        assert!(secrets.put.lock().unwrap().is_empty());
        wheel_in(window, "server-body", -3000., cx);
        click(window, "deploy-service-svc2", cx);
        wait_until(cx, "el otro", |cx| on_screen(window, "deploy-register", cx));
        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "vuelve", |cx| on_screen(window, "deploy-ci-mint", cx));
        assert!(!on_screen(window, "deploy-ci-key", cx), "la llave siguió a la vista");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    #[gpui_kit::test]
    fn with_a_pat_the_ci_key_is_saved_in_the_repo(cx: &mut TestAppContext) {
        let secrets = with_repo_secrets(true, cx);
        let (server, window) = deploys_as("admin", cx);
        mount_mint(&server);
        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "salen los ajustes", |cx| on_screen(window, "deploy-ci-mint", cx));
        reveal(window, "server-body", "deploy-ci-mint", cx);
        click(window, "deploy-ci-mint", cx);
        wait_until(cx, "dice dónde quedó", |cx| on_screen(window, "deploy-ci-saved", cx));
        assert!(!on_screen(window, "deploy-ci-key", cx));
        assert_eq!(*secrets.put.lock().unwrap(), [("o".to_string(), "web".to_string(), "CAC_DEPLOY_KEY".to_string(), "cac_dk_ENTERA".to_string())]);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Roturas que atrapa: guardar el build sin la fila entera o sin el
    /// cambio; pasar a que cac despliegue cada aviso del CI sin preguntar.
    #[gpui_kit::test]
    fn an_admin_saves_the_build_and_is_asked_before_cac_deploys_the_ci(cx: &mut TestAppContext) {
        with_repo_secrets(false, cx);
        let (server, window) = deploys_as("admin", cx);
        let mut saved = json!({ "id": "d1", "orgId": "o1", "serverId": "s1", "name": "web", "stack": "shop", "serviceName": "shop_web", "imageRepo": "ghcr.io/o/web", "repoFullName": "o/web", "onCINotify": "record", "currentImage": "ghcr.io/o/web:aaa1111" });
        saved["branch"] = json!("main");
        saved["buildOnHost"] = json!(true);
        server.mount(
            Mock::given(method("PATCH"))
                .and(path("/api/v1/servers/s1/deployables/d1"))
                .and(wiremock::matchers::body_json(json!({ "name": "web", "environment": "", "repoFullName": "o/web", "onCINotify": "record", "buildWorkflow": "", "shortTags": false, "migrateCommand": "", "rotateCommand": "", "branch": "main", "buildOnHost": true, "dockerfile": "", "buildContext": "" })))
                .respond_with(ok(saved)),
        );
        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "salen los ajustes", |cx| on_screen(window, "deploy-build-save", cx));
        reveal(window, "server-body", "deploy-ci-deploy", cx);
        click(window, "deploy-ci-deploy", cx);
        let (title, _) = cx.pending_prompt().expect("cambió quién despliega sin preguntar");
        assert_eq!(title, "¿Que cac despliegue cada aviso del CI?");
        cx.simulate_prompt_answer("Cancelar");
        cx.run_until_parked();
        assert!(server_writes(&server).iter().all(|(method, _, _)| method != "PATCH"), "cambió el modo al cancelar");

        reveal(window, "server-body", "deploy-branch", cx);
        click(window, "deploy-branch", cx);
        type_text(window, "main", cx);
        reveal(window, "server-body", "deploy-build-on-host", cx);
        click(window, "deploy-build-on-host", cx);
        wait_until(cx, "se marca construir aquí", |cx| on_screen(window, "deploy-build-context", cx));
        reveal(window, "server-body", "deploy-build-save", cx);
        click(window, "deploy-build-save", cx);
        wait_until(cx, "dice cómo acabó", |cx| on_screen(window, "deploy-settings-result", cx));
        assert_eq!(label(window, "deploy-settings-result", cx), "Guardado", "{:?}", server_writes(&server));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// 1Password y el ssh de mentira: apunta lo que se le pidió.
    #[derive(Default)]
    struct FakeOnePassword {
        unreadable: Vec<String>,
        rotated: std::sync::Mutex<Vec<(String, Vec<String>, Option<String>)>>,
    }

    impl crate::platform::secret_refs::OnePassword for FakeOnePassword {
        fn check(&self, op_ref: &str) -> Result<(), String> {
            if self.unreadable.iter().any(|bad| bad == op_ref) { Err("1Password está bloqueado".into()) } else { Ok(()) }
        }
        fn item_fields(&self, item: &str) -> Result<Vec<(String, String)>, String> {
            Ok(vec![("api key".into(), format!("{item}/api key")), ("smtp pass".into(), format!("{item}/smtp pass"))])
        }
        fn rotate(&self, _: &cac_ops::secrets::SshTarget, service: &str, refs: &[cac_ops::secrets::SecretRef], command: Option<&str>, progress: Box<dyn Fn(&'static str, usize, usize) + Send>) -> Result<crate::platform::secret_refs::Rotated, String> {
            progress("read", refs.len(), refs.len());
            self.rotated.lock().unwrap().push((service.into(), refs.iter().map(|r| r.name.clone()).collect(), command.map(str::to_string)));
            Ok(crate::platform::secret_refs::Rotated { items: refs.iter().map(|r| (r.name.clone(), format!("{}_v2", r.name), true)).collect(), updated: true })
        }
        fn capture(&self, _: &cac_ops::secrets::SshTarget, _: &str, vault: &str, title: &str) -> Result<(String, Vec<String>), String> {
            Ok((format!("op://{vault}/{title}"), vec!["REDIS_URL".into()]))
        }
    }

    fn with_one_password(fake: FakeOnePassword, cx: &mut TestAppContext) -> std::sync::Arc<FakeOnePassword> {
        let fake = std::sync::Arc::new(fake);
        let shared: std::sync::Arc<dyn crate::platform::secret_refs::OnePassword> = fake.clone();
        cx.update(|cx| cx.set_global(crate::platform::secret_refs::LocalOnePassword(shared)));
        fake
    }

    fn mount_secret_refs(server: &FakeServer, refs: serde_json::Value) {
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/s1/deployables/d1/secret-refs")).respond_with(ok(refs)));
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/s1/deployables/d1/secret-rotations")).respond_with(ok(json!([]))));
    }

    /// Un miembro ve de dónde salen los secrets pero no lo cambia, y no rota
    /// donde el CI despliega: diciendo por qué. «Probar» dice cuál no se lee.
    ///
    /// Roturas que atrapa: dejar que un miembro guarde referencias; rotar en
    /// un servicio que despliega el CI (su stack deploy tiraría los secrets);
    /// un «Probar» que no dice cuál falla ni por qué.
    #[gpui_kit::test]
    fn a_member_reads_the_refs_checks_them_and_cannot_rotate_under_ci(cx: &mut TestAppContext) {
        with_repo_secrets(false, cx);
        with_one_password(FakeOnePassword { unreadable: vec!["op://Prod/api/smtp".into()], ..Default::default() }, cx);
        let (server, window) = deploys_as("member", cx);
        mount_secret_refs(&server, json!([{ "name": "DATABASE_URL", "opRef": "op://Prod/api/url", "usage": "runtime" }, { "name": "SMTP_PASS", "opRef": "op://Prod/api/smtp", "usage": "runtime" }]));
        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "llegan los secrets", |cx| on_screen(window, "secret-row-1", cx));
        assert!(!on_screen(window, "secrets-save", cx));
        reveal(window, "server-body", "secrets-rotate", cx);
        assert!(on_screen(window, "secrets-rotate-why", cx), "no dijo por qué no se rota");
        reveal(window, "server-body", "secrets-check", cx);
        click(window, "secrets-check", cx);
        wait_until(cx, "dice cuál no se lee", |cx| on_screen(window, "secrets-unreadable", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Roturas que atrapa: importar campos de 1Password con nombres que no
    /// valen o repetidos; guardar sin el cambio o sin recortar.
    #[gpui_kit::test]
    fn an_admin_imports_an_item_and_saves_the_refs(cx: &mut TestAppContext) {
        with_repo_secrets(false, cx);
        with_one_password(FakeOnePassword::default(), cx);
        let (server, window) = deploys_as("admin", cx);
        mount_secret_refs(&server, json!([{ "name": "API_KEY", "opRef": "op://Prod/old/api key", "usage": "runtime" }]));
        server.mount(
            Mock::given(method("PUT"))
                .and(path("/api/v1/servers/s1/deployables/d1/secret-refs"))
                .and(wiremock::matchers::body_json(json!({ "refs": [{ "name": "API_KEY", "opRef": "op://Prod/old/api key", "usage": "runtime" }, { "name": "SMTP_PASS", "opRef": "op://Prod/api/smtp pass", "usage": "runtime" }] })))
                .respond_with(ok(json!(null))),
        );
        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "llegan los secrets", |cx| on_screen(window, "secret-row-0", cx));
        reveal(window, "server-body", "secrets-item", cx);
        click(window, "secrets-item", cx);
        type_text(window, "op://Prod/api", cx);
        click(window, "secrets-import", cx);
        wait_until(cx, "se añade el que faltaba", |cx| on_screen(window, "secret-row-1", cx));
        assert!(!on_screen(window, "secret-row-2", cx), "repitió el que ya estaba");
        reveal(window, "server-body", "secrets-save", cx);
        click(window, "secrets-save", cx);
        let saved = json!({ "refs": [{ "name": "API_KEY", "opRef": "op://Prod/old/api key", "usage": "runtime" }, { "name": "SMTP_PASS", "opRef": "op://Prod/api/smtp pass", "usage": "runtime" }] });
        wait_until(cx, "guarda la lista entera", |_| server.requests().iter().any(|r| r.method.as_str() == "PUT" && serde_json::from_slice::<serde_json::Value>(&r.body).ok() == Some(saved.clone())));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Roturas que atrapa: rotar sin preguntar; llevar otros secrets que los
    /// guardados; apuntar en cac algo más que nombres y versiones.
    #[gpui_kit::test]
    fn a_member_rotates_where_cac_deploys_and_only_names_are_recorded(cx: &mut TestAppContext) {
        with_repo_secrets(false, cx);
        let op = with_one_password(FakeOnePassword::default(), cx);
        let web = json!({ "id": "d1", "orgId": "o1", "serverId": "s1", "name": "web", "stack": "shop", "serviceName": "shop_web", "imageRepo": "ghcr.io/o/web", "repoFullName": "o/web", "onCINotify": "deploy", "currentImage": "ghcr.io/o/web:aaa1111" });
        let (server, window) = swarm_servers_with("member", json!([]), &["s1"], true, json!([web]), cx);
        for what in ["deployments", "builds", "commits"] {
            server.mount(Mock::given(method("GET")).and(path(format!("/api/v1/servers/s1/deployables/d1/{what}"))).respond_with(ok(json!([]))));
        }
        mount_secret_refs(&server, json!([{ "name": "DATABASE_URL", "opRef": "op://Prod/api/url", "usage": "runtime" }]));
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/servers/s1/deployables/d1/secret-rotations"))
                .and(wiremock::matchers::body_json(json!({ "names": ["DATABASE_URL"], "versions": { "DATABASE_URL": "DATABASE_URL_v2" }, "status": "succeeded", "error": "" })))
                .respond_with(ok(json!(null))),
        );
        click(window, "server-tab-deploy", cx);
        wait_until(cx, "salen los servicios", |cx| on_screen(window, "deploy-service-svc1", cx));
        click(window, "deploy-service-svc1", cx);
        wait_until(cx, "llegan los secrets", |cx| on_screen(window, "secret-row-0", cx));
        reveal(window, "server-body", "secrets-rotate", cx);
        click(window, "secrets-rotate", cx);
        let (title, _) = cx.pending_prompt().expect("rotó sin preguntar");
        assert_eq!(title, "¿Rotar los secrets de shop_web?");
        cx.simulate_prompt_answer("Cancelar");
        settle_frames(window, cx);
        assert!(op.rotated.lock().unwrap().is_empty(), "rotó al cancelar");
        click(window, "secrets-rotate", cx);
        cx.simulate_prompt_answer("Rotar en el servidor");
        wait_until(cx, "dice lo que rotó", |cx| on_screen(window, "secrets-rotated", cx));
        assert_eq!(*op.rotated.lock().unwrap(), [("shop_web".to_string(), vec!["DATABASE_URL".to_string()], None)]);
        let recorded = json!({ "names": ["DATABASE_URL"], "versions": { "DATABASE_URL": "DATABASE_URL_v2" }, "status": "succeeded", "error": "" });
        wait_until(cx, "apunta en cac sólo nombres y versiones", |_| {
            server.requests().iter().any(|r| r.method.as_str() == "POST" && r.url.path().ends_with("secret-rotations") && serde_json::from_slice::<serde_json::Value>(&r.body).ok() == Some(recorded.clone()))
        });
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// GitHub y el llavero de mentira: un PAT, y lo que se guardaría.
    #[derive(Default)]
    struct FakeGitHub {
        token: std::sync::Mutex<Option<String>>,
        put: std::sync::Mutex<Vec<(String, String, String, String, Option<bool>)>>,
        deleted: std::sync::Mutex<Vec<(String, bool)>>,
    }

    impl crate::platform::stack_secrets::GitHubRepo for FakeGitHub {
        fn configured(&self) -> bool {
            self.token.lock().unwrap().is_some()
        }
        fn op_reference(&self) -> Option<String> {
            None
        }
        fn set_token(&self, token: &str) -> Result<(), String> {
            *self.token.lock().unwrap() = Some(token.into());
            Ok(())
        }
        fn delete_token(&self) -> Result<(), String> {
            *self.token.lock().unwrap() = None;
            Ok(())
        }
        fn load_from_1password(&self, _: &str) -> Result<(), String> {
            Err("no".into())
        }
        fn refresh_from_1password(&self) -> Result<(), String> {
            Err("no".into())
        }
        fn list(&self, owner: String, repo: String, variables: bool) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<crate::platform::stack_secrets::RepoEntry>, String>> + Send>> {
            assert_eq!((owner.as_str(), repo.as_str()), ("o", "web"), "pidió otro repo");
            let entry = |name: &str, value: Option<&str>| crate::platform::stack_secrets::RepoEntry { name: name.into(), value: value.map(str::to_string), created_at: "2026-11-01T00:00:00Z".into(), updated_at: "2026-11-02T00:00:00Z".into() };
            let list = if variables { vec![entry("NODE_ENV", Some("production"))] } else { vec![entry("CAC_DEPLOY_KEY", None)] };
            Box::pin(async move { Ok(list) })
        }
        fn put(&self, owner: String, repo: String, name: String, value: String, variable: Option<bool>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
            self.put.lock().unwrap().push((owner, repo, name, value, variable));
            Box::pin(async { Ok(()) })
        }
        fn delete(&self, _: String, _: String, name: String, variable: bool) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
            self.deleted.lock().unwrap().push((name, variable));
            Box::pin(async { Ok(()) })
        }
    }

    /// La pestaña Secrets: sin PAT lo pide; con él lista los secrets del repo
    /// que sale de la imagen, guarda uno nuevo sin que su valor se quede y
    /// borra preguntando.
    ///
    /// Roturas que atrapa: pedir los de otro repo; guardar el PAT o el
    /// secret con otro valor, o un secret como variable; borrar sin
    /// preguntar o borrar una variable creyendo que es un secret.
    #[gpui_kit::test]
    fn the_secrets_tab_keeps_the_pat_and_writes_secrets_without_keeping_them(cx: &mut TestAppContext) {
        let fake = std::sync::Arc::new(FakeGitHub::default());
        let shared: std::sync::Arc<dyn crate::platform::stack_secrets::GitHubRepo> = fake.clone();
        cx.update(|cx| cx.set_global(crate::platform::stack_secrets::LocalGitHub(shared)));
        let (_server, window) = swarm_server_as("member", json!([]), cx);
        click(window, "server-tab-secrets", cx);
        wait_until(cx, "pide el PAT", |cx| on_screen(window, "github-pat-status", cx) && label(window, "github-pat-status", cx) == "Sin configurar");
        click(window, "github-token", cx);
        type_text(window, "ghp_DEMENTIRA", cx);
        click(window, "github-token-save", cx);
        wait_until(cx, "lista los del repo", |cx| on_screen(window, "github-entry-CAC_DEPLOY_KEY", cx));
        assert_eq!(fake.token.lock().unwrap().as_deref(), Some("ghp_DEMENTIRA"));

        click(window, "github-new", cx);
        click(window, "github-form-name", cx);
        type_text(window, "SENTRY_DSN", cx);
        click(window, "github-form-value", cx);
        type_text(window, "https://valor", cx);
        click(window, "github-form-save", cx);
        wait_until(cx, "se cierra el formulario", |cx| !on_screen(window, "github-form", cx));
        assert_eq!(*fake.put.lock().unwrap(), [("o".into(), "web".into(), "SENTRY_DSN".into(), "https://valor".into(), None)]);

        click(window, "github-tab-variables", cx);
        wait_until(cx, "las variables", |cx| on_screen(window, "github-entry-NODE_ENV", cx));
        reveal(window, "server-body", "github-delete-NODE_ENV", cx);
        click(window, "github-delete-NODE_ENV", cx);
        assert!(cx.pending_prompt().is_some(), "borró sin preguntar");
        cx.simulate_prompt_answer("Cancelar");
        settle_frames(window, cx);
        assert!(fake.deleted.lock().unwrap().is_empty(), "borró al cancelar");
        click(window, "github-delete-NODE_ENV", cx);
        cx.simulate_prompt_answer("Borrar");
        wait_until(cx, "borra la variable", |_| !fake.deleted.lock().unwrap().is_empty());
        assert_eq!(*fake.deleted.lock().unwrap(), [("NODE_ENV".to_string(), true)]);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Rotura que atrapa: registrar con lo que no es (el nombre largo, la
    /// imagen con su tag) o no pasar a desplegarlo al registrarlo.
    #[gpui_kit::test]
    fn an_admin_registers_a_service_prefilled_from_what_runs(cx: &mut TestAppContext) {
        let (server, window) = deploys_as("admin", cx);
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/servers/s1/deployables"))
                .and(wiremock::matchers::body_json(json!({ "name": "db", "stack": "shop", "serviceName": "shop_db", "imageRepo": "postgres", "environment": "", "shortTags": false })))
                .respond_with(ok(json!({ "id": "d2", "orgId": "o1", "serverId": "s1", "name": "db", "stack": "shop", "serviceName": "shop_db", "imageRepo": "postgres", "onCINotify": "record" }))),
        );
        for what in ["deployments", "builds", "commits"] {
            server.mount(Mock::given(method("GET")).and(path(format!("/api/v1/servers/s1/deployables/d2/{what}"))).respond_with(ok(json!([]))));
        }
        click(window, "deploy-service-svc2", cx);
        wait_until(cx, "se ofrece registrarlo", |cx| on_screen(window, "deploy-register", cx));
        click(window, "deploy-register", cx);
        wait_until(cx, "pasa a desplegarlo", |cx| on_screen(window, "deploy-history-empty", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Rotura que atrapa: ofrecerle reiniciar a un lector, cuyo pase al
    /// agente es de lectura.
    #[gpui_kit::test]
    fn a_reader_sees_the_services_but_no_restart(cx: &mut TestAppContext) {
        let (_server, window) = services_as("viewer", cx);
        assert!(on_screen(window, "service-logs-svc1", cx));
        assert!(!on_screen(window, "service-restart-svc1", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// El agente SSH y el llavero, de mentira: dos agentes (uno sin
    /// contestar) y dos llaves; lo guardado, apuntado.
    #[derive(Default)]
    struct FakeKeys {
        saved: std::sync::Mutex<Option<String>>,
        writes: std::sync::Mutex<Vec<(String, String)>>,
        /// El llavero bloqueado: leer falla.
        locked: std::sync::atomic::AtomicBool,
    }

    impl crate::platform::ssh_key::SshKeys for FakeKeys {
        fn agents(&self) -> Vec<cac_ops::ssh::SshAgent> {
            let agent = |socket: &str, label: &str, key_count, status: &str| cac_ops::ssh::SshAgent { socket: socket.into(), label: label.into(), key_count, status: status.into() };
            vec![agent("/run/gnome", "GNOME", 0, "refused"), agent("/run/1password", "1Password", 2, "ok")]
        }
        fn keys(&self, socket: Option<String>) -> Result<Vec<cac_ops::ssh::SshKeyItem>, String> {
            assert_eq!(socket.as_deref(), Some("/run/1password"), "no eligió el agente que contesta con llaves");
            let key = |title: &str| cac_ops::ssh::SshKeyItem { public_key: format!("ssh-ed25519 AAAA {title}"), title: title.into(), fingerprint: "SHA256:x".into(), key_type: "ED25519".into() };
            Ok(vec![key("contabo"), key("pixel")])
        }
        fn saved(&self, _: &str) -> Result<Option<String>, String> {
            if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("keychain: locked".into());
            }
            Ok(self.saved.lock().unwrap().clone())
        }
        fn save(&self, server_id: &str, public_key: &str) -> Result<(), String> {
            self.writes.lock().unwrap().push((server_id.into(), public_key.into()));
            Ok(())
        }
    }

    /// La llave de un servidor se elige entre las del agente que contesta y
    /// se guarda en el llavero; sin cambiar nada no se guarda. También a
    /// quien sólo mira: la llave es de su máquina, no de cac.
    ///
    /// Roturas que atrapa: preguntarle al agente caído; guardar otra llave
    /// que la elegida, o en otro servidor; guardar sin haber cambiado nada;
    /// esconderle el botón a un lector.
    #[gpui_kit::test]
    fn the_ssh_key_is_chosen_from_the_agent_and_kept_locally(cx: &mut TestAppContext) {
        let keys = std::sync::Arc::new(FakeKeys::default());
        let shared: std::sync::Arc<dyn crate::platform::ssh_key::SshKeys> = keys.clone();
        cx.update(|cx| cx.set_global(crate::platform::ssh_key::LocalKeys(shared)));
        let (_server, window) = platform_as("viewer", cx);

        click(window, "platform-ssh-s1", cx);
        wait_until(cx, "llegan las llaves", |cx| on_screen(window, "ssh-key-1", cx));
        assert_eq!(label(window, "ssh-key-title", cx), "Llave SSH de pixel-vps");
        click(window, "ssh-key-save", cx);
        cx.run_until_parked();
        assert!(keys.writes.lock().unwrap().is_empty(), "guardó sin cambiar nada");

        click(window, "ssh-key-1", cx);
        click(window, "ssh-key-save", cx);
        wait_until(cx, "se cierra", |cx| !on_screen(window, "ssh-key-dialog", cx));
        assert_eq!(*keys.writes.lock().unwrap(), [("s1".to_string(), "ssh-ed25519 AAAA pixel".to_string())]);
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Lo que se llevaría al servidor, apuntado en vez de entrar por SSH.
    #[derive(Default)]
    struct FakeInstaller {
        carried: std::sync::Mutex<Vec<(String, String, Option<String>, String)>>,
    }

    impl crate::platform::agent_install::AgentInstaller for FakeInstaller {
        fn install(&self, install: &cac_ops::agent_install::AgentInstall) -> Result<(), String> {
            self.carried.lock().unwrap().push((install.server_id.clone(), install.agent_token.clone(), install.identity_key.clone(), install.backend_url.clone()));
            Ok(())
        }
        fn wait(&self) -> cac_core::AgentWait {
            cac_core::AgentWait { total: std::time::Duration::from_secs(10), every: std::time::Duration::from_millis(20) }
        }
    }

    /// Instalar el agente pregunta nombrando el servidor; al confirmar lleva
    /// la identidad recién acuñada con la llave elegida para él y dice cómo
    /// volvió.
    ///
    /// Roturas que atrapa: reacuñar sin preguntar (tumba el agente que hay)
    /// o al cancelar; llevar otra identidad, otra llave u otro backend; no
    /// decir cómo acabó.
    #[gpui_kit::test]
    fn installing_the_agent_asks_and_says_how_it_came_back(cx: &mut TestAppContext) {
        let keys = std::sync::Arc::new(FakeKeys::default());
        *keys.saved.lock().unwrap() = Some("ssh-ed25519 AAAA pixel".into());
        let shared: std::sync::Arc<dyn crate::platform::ssh_key::SshKeys> = keys.clone();
        let installer = std::sync::Arc::new(FakeInstaller::default());
        let installing: std::sync::Arc<dyn crate::platform::agent_install::AgentInstaller> = installer.clone();
        cx.update(|cx| {
            cx.set_global(crate::platform::ssh_key::LocalKeys(shared));
            cx.set_global(crate::platform::agent_install::LocalInstaller(installing));
        });
        let (server, window) = platform_as("admin", cx);
        server.mount(
            Mock::given(method("POST"))
                .and(path("/api/v1/servers/s1/agent-token"))
                .respond_with(wiremock::ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": { "token": "cac_agent_NEW", "sessionKey": "cd".repeat(32) } }))),
        );
        let mut back = platform_server("s1", "o1", "pixel-vps", "online");
        back["agentVersion"] = json!(14);
        back["agentSeenAt"] = json!("2026-11-20T10:05:00Z");
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/s1")).respond_with(ok(back)));

        click(window, "platform-open-s1", cx);
        wait_until(cx, "sale el botón", |cx| on_screen(window, "server-agent-install", cx));
        click(window, "server-agent-install", cx);
        let (title, body) = cx.pending_prompt().expect("instaló sin preguntar");
        assert_eq!(title, "¿Actualizar el agente de pixel-vps a la v14?", "uno con identidad en la v2 se actualiza, no se instala");
        assert!(body.contains("deja de valer"), "{body}");
        cx.simulate_prompt_answer("Cancelar");
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(server_writes(&server).is_empty(), "reacuñó al cancelar");

        click(window, "server-agent-install", cx);
        cx.simulate_prompt_answer("Actualizar");
        wait_until(cx, "dice cómo volvió", |cx| on_screen(window, "server-agent-result", cx));
        assert_eq!(label(window, "server-agent-result", cx), "pixel-vps: agente v2 → v14, en línea");
        let backend = cx.update(|cx| Backend::core(cx).base_url().to_string());
        assert_eq!(
            *installer.carried.lock().unwrap(),
            [("s1".to_string(), "cac_agent_NEW".to_string(), Some("ssh-ed25519 AAAA pixel".to_string()), backend)]
        );
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Un agente en la versión que instala esta app dice «al día» y su botón
    /// ofrece reinstalar, avisando de que ya tiene la última; uno atrasado
    /// dice cuál hay (boaty en la v14, 22-nov).
    ///
    /// Roturas que atrapan: no decir nunca que está al día; seguir invitando
    /// a actualizar o preguntar «instalar» a uno que ya tiene la última.
    #[gpui_kit::test]
    fn an_agent_on_the_latest_says_so_and_only_offers_a_reinstall(cx: &mut TestAppContext) {
        let server = FakeServer::start();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        let mut mine = org("o1", "G Studio");
        mine["role"] = json!("admin");
        server.mount(get("/api/v1/organizations/").respond_with(ok(json!([mine]))));
        server.mount(get("/api/v1/task-spaces/").respond_with(ok(json!([space("a", "Producto")]))));
        server.mount(get("/api/v1/docs/").respond_with(ok(json!({}))));
        server.mount(get("/api/v1/tasks/").respond_with(ok(json!([]))));
        let mut latest = platform_server("s1", "o1", "boaty", "online");
        latest["agentVersion"] = json!(cac_core::AGENT_VERSION_LATEST);
        let old = platform_server("s2", "o1", "tds", "online");
        server.mount(get("/api/v1/servers/").respond_with(ok(json!([latest, old]))));
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-l-a", cx));
        click(window, "rail-platform", cx);
        wait_until(cx, "sale la tabla", |cx| on_screen(window, "platform-open-s1", cx));

        click(window, "platform-open-s1", cx);
        wait_until(cx, "sale el agente", |cx| on_screen(window, "server-agent-facts", cx));
        assert!(label(window, "server-agent-facts", cx).contains("versión 14 · al día"), "{}", label(window, "server-agent-facts", cx));
        click(window, "server-agent-install", cx);
        let (title, body) = cx.pending_prompt().expect("reinstaló sin preguntar");
        assert_eq!(title, "¿Reinstalar el agente de boaty?");
        assert!(body.starts_with("Ya tiene la última versión (v14)"), "{body}");
        cx.simulate_prompt_answer("Cancelar");
        cx.run_until_parked();

        click(window, "server-back", cx);
        wait_until(cx, "vuelve la tabla", |cx| on_screen(window, "platform-open-s2", cx));
        click(window, "platform-open-s2", cx);
        wait_until(cx, "sale el agente atrasado", |cx| on_screen(window, "server-agent-facts", cx) && label(window, "server-agent-facts", cx).contains("versión 2"));
        assert!(label(window, "server-agent-facts", cx).contains("hay v14"), "{}", label(window, "server-agent-facts", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Rotura que atrapa: reacuñar (y tumbar el agente) con el llavero
    /// bloqueado, para luego entrar por SSH ofreciendo todas las llaves.
    #[gpui_kit::test]
    fn a_locked_keychain_leaves_the_agent_alone(cx: &mut TestAppContext) {
        let keys = std::sync::Arc::new(FakeKeys::default());
        keys.locked.store(true, std::sync::atomic::Ordering::SeqCst);
        let shared: std::sync::Arc<dyn crate::platform::ssh_key::SshKeys> = keys.clone();
        let installer = std::sync::Arc::new(FakeInstaller::default());
        let installing: std::sync::Arc<dyn crate::platform::agent_install::AgentInstaller> = installer.clone();
        cx.update(|cx| {
            cx.set_global(crate::platform::ssh_key::LocalKeys(shared));
            cx.set_global(crate::platform::agent_install::LocalInstaller(installing));
        });
        let (server, window) = platform_as("admin", cx);
        click(window, "platform-open-s1", cx);
        wait_until(cx, "sale el botón", |cx| on_screen(window, "server-agent-install", cx));
        click(window, "server-agent-install", cx);
        cx.simulate_prompt_answer("Actualizar");
        wait_until(cx, "dice por qué no", |cx| on_screen(window, "server-agent-result", cx));
        assert!(label(window, "server-agent-result", cx).contains("llavero"), "{}", label(window, "server-agent-result", cx));
        assert!(server_writes(&server).is_empty(), "reacuñó con el llavero bloqueado");
        assert!(installer.carried.lock().unwrap().is_empty());
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// «Usuarios» sale a quien administra la plataforma entera.
    ///
    /// Rotura que atrapa: esconderlo también al superadmin, o decidirlo con
    /// otra cosa que su sesión.
    #[gpui_kit::test]
    fn users_shows_for_a_superadmin(cx: &mut TestAppContext) {
        let server = server();
        server.mount(Mock::given(method("GET")).and(path("/api/v1/servers/")).respond_with(ok(json!([]))));
        server.install(cx);
        let window = open(cx);
        cx.update(|cx| {
            let boss = serde_json::from_value(json!({ "id": "u1", "username": "ana", "superadmin": true })).unwrap();
            Backend::core(cx).update_session(boss).unwrap();
        });
        click(window, "rail-platform", cx);
        wait_until(cx, "sale «Usuarios»", |cx| on_screen(window, "platform-users", cx));
        wait_until(cx, "dice que no hay servidores", |cx| on_screen(window, "platform-no-servers", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    /// Sin nada pendiente no hay número, y lo que no se pudo contar tampoco
    /// se pinta.
    ///
    /// Roturas que atrapa: un «0» en la esquina de cada botón; un número
    /// inventado cuando el servidor no contestó; «Comunicación (0)» al pasar
    /// el ratón.
    #[gpui_kit::test]
    fn nothing_pending_or_unknown_paints_no_count(cx: &mut TestAppContext) {
        let server = server();
        let get = |route: &str| Mock::given(method("GET")).and(path(route.to_string()));
        server.mount(get("/api/v1/notifications/").respond_with(ok(json!({ "items": [], "unread": 0 }))));
        server.mount(get("/api/v1/chat/unread").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/dm/").respond_with(ok(json!([]))));
        server.mount(get("/api/v1/servers/").respond_with(ok(json!([{ "id": "s1", "orgId": "o1", "status": "online" }]))));
        // Las invitaciones no contestan: no se sabe cuántas hay.
        server.install(cx);
        let window = open(cx);

        wait_until(cx, "llegan los ceros", |cx| {
            let counters = cx.update(|cx| Backend::snapshot(cx).read(cx).counters.clone());
            (counters.notifications, counters.comms, counters.servers) == (Some(0), Some(0), Some(0)) && on_screen(window, "tree-l-a", cx)
        });
        settle_frames(window, cx);

        assert_eq!(cx.update(|cx| Backend::snapshot(cx).read(cx).counters.invitations), None);
        for id in COUNTS {
            assert!(!on_screen(window, id, cx), "{id} se pinta sin nada que contar");
        }
        assert_eq!(label(window, "rail-comms", cx), "Comunicación");
        assert_eq!(label(window, "notifications-bell", cx), "Notificaciones");
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
    }

    // La pestaña de backups, en su fichero (ver su cabecera).
    include!("platform/backups_tests.rs");

    // Actividad, Diagnóstico y Usuarios, en su fichero.
    include!("platform/tools_tests.rs");

    // Las DevTools, en su fichero.
    include!("platform/devtools_tests.rs");
}
