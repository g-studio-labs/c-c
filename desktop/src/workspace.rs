//! La app por dentro: el riel de módulos, el panel con la organización y el
//! árbol de espacios, y el sitio de la vista.
//!
//! Qué filas tiene el árbol, cuáles van abiertas, qué hace un clic y qué se
//! trae al entrar lo dice el core. Aquí se pinta y se le pasa cada gesto.

use cac_core::{Locale, NodeKind, ThemePreference, TreeRow};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Side, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::backend::Backend;
use crate::docs::DocsView;
use crate::lang::{locale, tr};
use crate::mywork::MyWorkView;
use crate::task::TaskView;

/// Las medidas de la app de siempre: el riel y el panel.
const RAIL_WIDTH: f32 = 48.;
const PANEL_WIDTH: f32 = 256.;
const ROW_HEIGHT: f32 = 28.;
/// El lado de un botón del riel.
const RAIL_BUTTON: f32 = 36.;
/// El ancho del menú de la cuenta: el de la app de siempre (`w-64`).
const ACCOUNT_MENU_WIDTH: f32 = 256.;
/// Lo que entra una fila por cada nivel que cuelga: lo de dentro empieza
/// bajo la marca de su padre (el hueco del chevrón más la separación).
const INDENT: f32 = 22.;

/// Casi sin estado propio: lo que pinta lo lee del core al pintarse, y GPUI
/// repinta una vista cuando cambia algo que leyó. No hace falta suscribirse.
pub struct Workspace {
    pub(crate) my_work: Entity<MyWorkView>,
    docs: Entity<DocsView>,
    pub(crate) task: Entity<TaskView>,
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
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Quien entra, entra a trabajar: organizaciones, árbol, tareas y el
        // stream de eventos.
        Backend::run(cx, |core| async move { core.open_workspace().await });
        let placeholder = tr(cx, "nav:panel.filter");
        Self {
            my_work: cx.new(|cx| MyWorkView::new(window, cx)),
            docs: cx.new(|_| DocsView),
            task: cx.new(|cx| TaskView::new(window, cx)),
            tree_filter: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            panel_pinned: true,
            over_rail: false,
            over_panel: false,
        }
    }

    /// Fija el panel o lo suelta. Al cambiar, el ratón no cuenta como
    /// encima de nada: soltarlo desde su botón lo esconde aunque el puntero
    /// siga ahí, que es lo que se acaba de pedir.
    pub(crate) fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        self.panel_pinned = !self.panel_pinned;
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
            .child(div().flex().items_center().gap_2().text_xs().text_color(muted).child(Icon::new(icon).size_3p5()).child(label))
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
            .text_xs()
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
        let trigger = Button::new("account-menu").ghost().p_0().size(px(RAIL_BUTTON)).rounded_lg().tooltip(tr(cx, "nav:rail.account")).child(
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
        Popover::new("account-popover").appearance(false).anchor(Anchor::LeftCenter).offset(px(8.)).trigger(trigger).content(move |_, _, cx| {
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
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(popover)
                .shadow_lg()
                .child(
                    div().flex().items_center().gap_2().px_2().py_1p5().child(in_menu(28.)).child(
                        div()
                            .min_w_0()
                            .child(div().truncate().text_xs().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                            .children(email.clone().map(|email| div().truncate().text_size(px(11.)).text_color(muted).child(email))),
                    ),
                )
                .child(rule())
                .child(Self::options_row("account-theme", Lucide::Moon, cac_core::text(here, "nav:account.theme"), themes, cx))
                .child(Self::options_row("account-language", Lucide::Languages, cac_core::text(here, "nav:account.language"), languages, cx))
                .child(Self::pending_row("account-profile", Lucide::UserRound, tr(cx, "nav:profile.title"), cx))
                .child(Self::pending_row("account-password", Lucide::KeyRound, tr(cx, "nav:account.changePassword"), cx))
                .child(Self::pending_row("account-connect", Lucide::Bot, tr(cx, "nav:account.connectMcp"), cx))
                .child(Self::pending_row("account-notifications", Lucide::Bell, tr(cx, "nav:account.notifications"), cx))
                .child(rule())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1p5()
                        .text_xs()
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
            .text_sm()
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
            .rounded_lg()
            .text_color(if active { primary } else { muted })
            .when(active, |this| {
                this.bg(accent).child(div().absolute().left(px(-6.)).top_2().bottom_2().w(px(2.)).rounded_full().bg(primary))
            })
            .when(!active, |this| this.hover(move |style| style.bg(accent).text_color(foreground)))
            .child(Icon::new(icon).size_5())
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
    }

    /// Un módulo que la app de siempre tiene y ésta todavía no: está en su
    /// sitio y lo dice al pasar el ratón.
    fn pending_module(id: &'static str, icon: Lucide, label_key: &str, cx: &App) -> impl IntoElement {
        let label = format!("{} · {}", tr(cx, label_key), tr(cx, "common:misc.notInThisApp"));
        Self::rail_button(id, icon, label, false, cx).test_support()
    }

    /// Una fila del panel que la app de siempre tiene y ésta todavía no.
    fn pending_panel_row(id: &'static str, icon: Lucide, label_key: &str, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, foreground) = (theme.muted_foreground, theme.sidebar_accent.opacity(0.5), theme.sidebar_foreground);
        let not_yet = tr(cx, "common:misc.notInThisApp");
        div()
            .id(id)
            .h(px(ROW_HEIGHT))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_2()
            .mx_1()
            .rounded_md()
            .text_sm()
            .text_color(muted)
            .hover(move |style| style.bg(hover).text_color(foreground))
            .child(Icon::new(icon).small())
            .child(tr(cx, label_key))
            .tooltip(move |window, cx| Tooltip::new(not_yet.clone()).build(window, cx))
            .test_support()
    }

    /// Un botón pequeño de cabecera que todavía no hace nada aquí: dice qué
    /// es y que aún no está.
    fn pending_tool(id: &'static str, icon: Lucide, label_key: &str, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, foreground) = (theme.muted_foreground, theme.sidebar_accent, theme.foreground);
        let label = format!("{} · {}", tr(cx, label_key), tr(cx, "common:misc.notInThisApp"));
        div()
            .id(id)
            .size(px(24.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_md()
            .text_color(muted)
            .hover(move |style| style.bg(hover).text_color(foreground))
            .child(Icon::new(icon).size_3p5())
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .test_support()
    }

    /// La barra de arriba de la app de siempre: el buscador en el centro y
    /// la campana a la derecha (`AppLayout.tsx`). Ninguno de los dos tiene
    /// todavía nada detrás en esta app, y lo dicen al pasar el ratón.
    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, surface, hover, foreground) = (theme.border, theme.muted_foreground, theme.muted, theme.accent, theme.foreground);
        let mono = theme.mono_font_family.clone();
        let not_yet = tr(cx, "common:misc.notInThisApp");
        let bell_hint = format!("{} · {}", tr(cx, "common:last.notifications"), not_yet);
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
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .bg(surface.opacity(0.3))
                    .text_xs()
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
                    .ml_2()
                    .p_1p5()
                    .flex_none()
                    .rounded_md()
                    .text_color(muted)
                    .hover(move |style| style.bg(hover).text_color(foreground))
                    .child(Icon::new(Lucide::Bell).small())
                    .tooltip(move |window, cx| Tooltip::new(bell_hint.clone()).build(window, cx))
                    .test_support(),
            )
            .test_support()
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (sidebar, border) = (theme.sidebar, theme.sidebar_border);
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
            // El único módulo con pantalla en la primera entrega. Pulsarlo
            // fija el panel o lo suelta.
            .child(
                Self::rail_button("rail-work", Lucide::Briefcase, tr(cx, "nav:module.work"), true, cx)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(cx)))
                    .test_support(),
            )
            .child(Self::pending_module("rail-docs", Lucide::BookOpen, "nav:module.docs", cx))
            .child(Self::pending_module("rail-comms", Lucide::MessagesSquare, "nav:module.comms", cx))
            .child(Self::pending_module("rail-platform", Lucide::Server, "nav:module.platform", cx))
            .child(div().flex_1())
            .child(Self::pending_module("rail-organization", Lucide::Building2, "nav:rail.organization", cx))
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
        let several = orgs.orgs.len() > 1;
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
                    .rounded_lg()
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
                    .child(div().id("org-name").test_support().w_full().truncate().text_sm().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                    .children(hint.map(|hint| div().id("org-hint").test_support().w_full().truncate().text_xs().text_color(muted).child(hint))),
            )
            .when(several, |this| this.child(Icon::new(Lucide::ChevronsUpDown).xsmall().text_color(muted)));
        let label = tr(cx, "common:misc.organizations");
        Button::new("org-switch").ghost().w_full().h(px(44.)).child(face).dropdown_menu(move |menu, _, _| {
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

    fn render_row(&self, row: TreeRow, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (hover, active_bg, muted, foreground, primary) =
            (theme.sidebar_accent.opacity(0.5), theme.sidebar_accent, theme.muted_foreground, theme.sidebar_foreground, theme.primary);
        let chevron = if row.open { IconName::ChevronDown } else { IconName::ChevronRight };
        let toggle_id = row.id.clone();
        let clicked = row.clone();
        let count = (row.kind == NodeKind::List && row.open_count > 0).then(|| row.open_count.to_string());
        // Qué es cada fila, a la vista: el color de su espacio, una carpeta
        // o una lista.
        let mark = match row.kind {
            NodeKind::Space => {
                div().size(px(10.)).flex_none().rounded_full().bg(crate::theme::space_color(&row.color, row.hue, cx)).into_any_element()
            }
            NodeKind::Folder => Icon::new(Lucide::Folder).xsmall().text_color(muted).into_any_element(),
            NodeKind::List => Icon::new(Lucide::ListChecks).xsmall().text_color(muted).into_any_element(),
        };
        let (space, list) = (row.kind == NodeKind::Space, row.kind == NodeKind::List);
        let menu_label = tr(
            cx,
            match row.kind {
                NodeKind::Space => "work:tree.spaceMenu",
                NodeKind::Folder => "work:tree.folderMenu",
                NodeKind::List => "work:tree.listMenu",
            },
        );
        let (menu_id, docs_label) = (row.id.clone(), tr(cx, "work:tree.openDocs"));

        div()
            .id(SharedString::from(format!("tree-{}", row.id)))
            .test_support()
            .h(px(ROW_HEIGHT))
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            // Una lista lleva margen para su fondo redondeado: se le
            // descuenta, o su icono y su «···» no caen en la columna de los demás.
            .pl(px(8. + INDENT * row.depth as f32 - if list { 4. } else { 0. }))
            .when(!list, |this| this.pr_1())
            .text_sm()
            .text_color(foreground)
            .when(space, |this| this.font_weight(FontWeight::MEDIUM))
            // El espacio al que se acota se dice con su nombre en el color de
            // la marca; la lista, con la fila entera.
            .when(space && row.active, |this| this.text_color(primary))
            .when(list, |this| this.mx_1().rounded_md())
            .when(list && row.active, |this| this.bg(active_bg).font_weight(FontWeight::MEDIUM))
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .on_click(move |_, _, cx| Self::click_row(&clicked, cx))
            // Una lista no se abre: su icono va donde el chevrón de las demás.
            .when(!list, |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!("tree-toggle-{}", row.id)))
                        .test_support()
                        .size(px(16.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .when(row.expandable, |this| {
                            this.child(Icon::new(chevron).xsmall().text_color(muted)).on_click(move |_, _, cx| {
                                // El chevrón abre y cierra; no es un clic en la fila.
                                cx.stop_propagation();
                                Backend::core(cx).tree().toggle(&toggle_id);
                            })
                        }),
                )
            })
            .child(mark)
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(row.name))
            // Lo ve un cliente: es su canal.
            .when(row.client, |this| {
                this.child(div().id(SharedString::from(format!("tree-client-{}", row.id))).test_support().flex_none().child(Icon::new(IconName::Eye).xsmall().text_color(primary)))
            })
            // Tiene documentación escrita.
            .when(row.has_doc, |this| {
                this.child(div().id(SharedString::from(format!("tree-doc-{}", row.id))).test_support().flex_none().child(Icon::new(Lucide::FileText).xsmall().text_color(muted)))
            })
            .children(count.map(|count| div().flex_none().text_xs().font_weight(FontWeight::NORMAL).text_color(muted).child(count)))
            // El menú de la fila. Pulsar un espacio o una lista acota sus
            // tareas, así que su documentación se abre desde aquí
            // (command-and-control-372). Abrirlo no es un clic en la fila: el
            // desplegable del kit ya se queda con él (medido en
            // `popover.rs`); si deja de hacerlo, lo dice la prueba del menú.
            .child(
                div().id(SharedString::from(format!("tree-menu-{}", row.id))).test_support().flex_none().child(
                    Button::new(SharedString::from(format!("tree-more-{}", row.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Ellipsis).text_color(muted))
                        .tooltip(menu_label)
                        .dropdown_menu(move |menu, _, _| {
                            let id = menu_id.clone();
                            menu.item(
                                PopupMenuItem::new(docs_label.clone())
                                    .icon(Icon::new(Lucide::FileText))
                                    .on_click(move |_, _, cx| Self::open_docs(id.clone(), cx)),
                            )
                        }),
                ),
            )
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (sidebar, border, muted, hover, foreground) =
            (theme.sidebar, theme.sidebar_border, theme.muted_foreground, theme.sidebar_accent, theme.sidebar_foreground);
        let snapshot = Backend::snapshot(cx).read(cx);
        // «Mi trabajo» es la pantalla, esté acotada o no: a qué, lo dice el
        // árbol. Leyendo documentación ya no se está en ella.
        let on_my_work = snapshot.docs.target.is_none();
        let pinned = self.panel_pinned;
        // El texto del catálogo anuncia un atajo de teclado que esta app
        // todavía no tiene: se dice sólo lo que el botón hace.
        let toggle_hint = tr(cx, "nav:rail.collapsePanel").split(" (").next().unwrap_or_default().to_string();
        let query = self.tree_filter.read(cx).value().to_string();
        let rows = Backend::core(cx).tree().rows_matching(&query);
        // Filtrando y sin nada que enseñar se dice: un árbol en blanco se lee
        // como que la organización no tiene espacios.
        let nothing = rows.is_empty() && !query.trim().is_empty();

        div()
            .w(px(PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_none()
            .flex_col()
            .bg(sidebar)
            .text_color(foreground)
            .border_r_1()
            .border_color(border)
            .child(div().p_2().border_b_1().border_color(border).child(self.render_org(cx)))
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl_3()
                    .pr_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(div().flex_1().child(tr(cx, "nav:module.work").to_uppercase()))
                    .child(
                        Button::new("panel-toggle")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if pinned { Lucide::PanelLeftClose } else { Lucide::PanelLeftOpen }).text_color(muted))
                            .tooltip(toggle_hint)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(cx))),
                    ),
            )
            .child(
                div()
                    .id("tree-everything")
                    .test_support()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .mx_1()
                    .rounded_md()
                    .text_sm()
                    .when(on_my_work, |this| this.bg(hover).font_weight(FontWeight::MEDIUM))
                    .hover(move |style| style.bg(hover))
                    .cursor_pointer()
                    .on_click(|_, _, cx| Backend::core(cx).show_all_work())
                    .child(Icon::new(IconName::Inbox).small())
                    .child(tr(cx, "nav:item.myWork")),
            )
            .child(Self::pending_panel_row("panel-overview", Lucide::LayoutDashboard, "nav:item.overview", cx))
            .child(Self::pending_panel_row("panel-notes", Lucide::NotebookPen, "nav:item.notes", cx))
            // Filtra lo que ya está en el panel; no es el buscador.
            .child(
                div().px_2().py_1().child(
                    Input::new(&self.tree_filter)
                        .id("tree-filter")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(muted)),
                ),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl_4()
                    .pr_1()
                    .mt_2()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(foreground.opacity(0.7))
                    .child(div().flex_1().child(tr(cx, "nav:panel.spaces")))
                    // Reordenar y crear un espacio: están donde en la app de
                    // siempre, y todavía no hacen nada aquí.
                    .child(Self::pending_tool("spaces-rearrange", Lucide::Lock, "work:tree.rearrange", cx))
                    .child(Self::pending_tool("spaces-new", Lucide::Plus, "work:tree.newSpace", cx)),
            )
            .when(nothing, |this| {
                this.child(div().id("tree-no-matches").test_support().px_3().py_3().text_xs().text_color(muted).child(tr(cx, "nav:panel.noMatches")))
            })
            .child(
                div()
                    .id("tree")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().map(|row| self.render_row(row, cx))),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = Backend::snapshot(cx).read(cx);
        let lasting = snapshot.connection.lasting;
        // Con un documento abierto se lee documentación; si no, las tareas.
        let main: AnyView = if snapshot.docs.target.is_some() { self.docs.clone().into() } else { self.my_work.clone().into() };
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
                    .children(task.map(|task| div().id("task-over").absolute().inset_0().occlude().child(task))),
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
        sign_in(cx);
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(width), px(760.)),
                })),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(window, cx))).expect("open test window").0
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

    /// Pulsar una carpeta abre su documentación en el sitio de la vista, y
    /// su × devuelve a las tareas.
    ///
    /// Roturas que atrapa: una carpeta cuyo clic no hace nada; abrir el
    /// documento en el core y seguir pintando la lista; no tener cómo volver.
    #[gpui_kit::test]
    fn a_folder_opens_its_docs_and_the_list_brings_the_tasks_back(cx: &mut TestAppContext) {
        let server = server_with_docs();
        server.install(cx);
        let window = open(cx);
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        assert!(on_screen(window, "view-title", cx) && !on_screen(window, "docs", cx));

        click(window, "tree-f-a", cx);
        wait_until(cx, "llega su documentación", |cx| on_screen(window, "doc-body", cx));
        assert!(!on_screen(window, "view-title", cx), "la lista sigue pintada debajo del documento");
        assert_eq!(scope(cx), None, "una carpeta no acota");

        // Una carpeta no tiene tarjetas detrás: no ofrece vistas de lista.
        assert!(!on_screen(window, "view-list", cx) && !on_screen(window, "view-board", cx), "ofrece el tablero de una carpeta");
        click(window, "doc-close", cx);
        wait_until(cx, "vuelven las tareas", |cx| on_screen(window, "view-title", cx));
        assert!(!on_screen(window, "docs", cx));
        cx.update(|cx| Backend::run(cx, |core| async move { core.stream().stop() }));
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
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        click(window, "tree-f-a", cx);
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
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        click(window, "tree-f-a", cx);
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
        wait_until(cx, "el árbol llega", |cx| on_screen(window, "tree-f-a", cx));
        click(window, "tree-f-a", cx);
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

        cx.update(|cx| Workspace::open_docs("a".into(), cx));
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

}
