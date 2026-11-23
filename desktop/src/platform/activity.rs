//! Actividad (`app/src/pages/Activity.tsx`): los runs de CI y los despliegues
//! de la organización, lo más nuevo primero. Qué entra y en qué orden lo
//! decide el core (`cac_core::platform::activity`); aquí se pinta.

use cac_core::{group_attempts, short_ref, text_with, time_ago, ActivityDeployment, ActivityFilter, ActivityRow, ActivityTone, ServerTab, WorkflowRun};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::deploy::{status_colors, who};
use super::services::pill;
use crate::backend::Backend;
use crate::lang::{locale, tr};
use crate::theme::Leading as _;

fn tone_colors(tone: ActivityTone, cx: &App) -> (Hsla, Hsla) {
    let theme = cx.theme();
    match tone {
        ActivityTone::Success => (theme.success.opacity(0.15), theme.success),
        ActivityTone::Failure => (theme.danger.opacity(0.15), theme.danger),
        ActivityTone::Running => (theme.secondary, theme.secondary_foreground),
        ActivityTone::Muted => (theme.border, theme.muted_foreground),
    }
}

/// Un texto del catálogo, o la clave cruda si GitHub estrena un estado que
/// el catálogo aún no tiene.
fn run_label(run: &WorkflowRun, cx: &App) -> String {
    let key = run.label_key();
    let text = tr(cx, &key);
    if text != key {
        text
    } else if run.status == "completed" && !run.conclusion.is_empty() {
        run.conclusion.clone()
    } else {
        run.status.clone()
    }
}

/// Al servidor del despliegue, en su pestaña de servicios.
fn open_service(server_id: String, cx: &App) {
    Backend::run(cx, move |core| async move {
        core.platform().open_server_and_load(&server_id).await;
        core.platform().open_tab(ServerTab::Services).await;
    });
}

pub(crate) fn page(cx: &App) -> AnyElement {
    let state = Backend::snapshot(cx).read(cx).platform.activity.clone().unwrap_or_default();
    let theme = cx.theme();
    let (background, foreground, muted, border, danger) = (theme.background, theme.foreground, theme.muted_foreground, theme.border, theme.danger);
    let (secondary, secondary_foreground) = (theme.secondary, theme.secondary_foreground);
    let language = locale(cx);

    let filter = (!state.filter.is_empty()).then(|| {
        let name = state.deployable_name();
        div()
            .id("activity-filter")
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .when(!state.filter.repo.is_empty(), |row| {
                row.child(pill(text_with(language, "activity:filter.repo", &[("repo", &state.filter.repo)]), secondary, secondary_foreground))
            })
            .when(!state.filter.deployable_id.is_empty(), |row| {
                let label = if name.is_empty() { tr(cx, "activity:filter.deployable") } else { format!("{}: {name}", tr(cx, "activity:filter.deployable")) };
                row.child(pill(label, secondary, secondary_foreground))
            })
            .child(
                Button::new("activity-filter-clear")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::X))
                    .label(tr(cx, "activity:filter.clear"))
                    .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.platform().open_activity(ActivityFilter::default()).await })),
            )
            .test_support()
    });

    let rows = group_attempts(&state.entries);
    let list = if state.loaded && rows.is_empty() {
        let phrase = tr(cx, if state.filter.is_empty() { "activity:empty" } else { "activity:emptyFiltered" });
        div().id("activity-empty").p_6().rounded(px(8.)).border_1().border_dashed().border_color(border).text_center().text_color(muted).child(phrase).test_support().into_any_element()
    } else {
        div()
            .id("activity-list")
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .children(rows.iter().enumerate().map(|(index, row)| activity_row(row, index > 0, cx)))
            .test_support()
            .into_any_element()
    };

    let more = (state.has_more || state.loading).then(|| {
        div().flex().justify_center().child(
            Button::new("activity-more")
                .outline()
                .small()
                .label(tr(cx, if state.loading { "activity:loading" } else { "activity:loadMore" }))
                .disabled(state.loading)
                .on_click(|_, _, cx| Backend::run(cx, |core| async move { core.platform().load_more_activity().await })),
        )
    });

    div()
        .id("activity-page")
        .size_full()
        .overflow_y_scroll()
        .bg(background)
        .text_color(foreground)
        .child(
            div()
                .mx_auto()
                .w_full()
                .max_w(px(896.))
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .leading_sm()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().id("activity-title").flex().items_center().gap_2().text_size(rems(1.19)).font_weight(FontWeight::SEMIBOLD).child(Icon::new(Lucide::Workflow).size_5()).child(tr(cx, "activity:title")).test_support())
                        .child(div().text_color(muted).child(tr(cx, "activity:lead"))),
                )
                .children(filter)
                .children(state.error.clone().map(|error| div().id("activity-error").text_color(danger).child(error).test_support()))
                .child(list)
                .children(more),
        )
        .into_any_element()
}

fn activity_row(row: &ActivityRow, separated: bool, cx: &App) -> AnyElement {
    let entry = &row.entry;
    let line = match (&entry.run, &entry.deployment) {
        (Some(run), _) if entry.is_run() => run_row(run, &row.earlier, &entry.deployments, cx),
        (_, Some(deployment)) => deployment_row(deployment, cx),
        _ => div().id("activity-unknown"),
    };
    let border = cx.theme().border;
    line.when(separated, |line| line.border_t_1().border_color(border)).test_support().into_any_element()
}

fn run_row(run: &WorkflowRun, earlier: &[WorkflowRun], deployments: &[ActivityDeployment], cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (muted, mono, hover, border) = (theme.muted_foreground, theme.mono_font_family.clone(), theme.muted, theme.border);
    let (bg, fg) = tone_colors(run.tone(), cx);
    let label = run_label(run, cx);
    let language = locale(cx);
    let when = time_ago(&run.occurred_at, &chrono::Local::now(), language);
    let open = run.opens_on_github().then(|| {
        let url = run.html_url.clone();
        Button::new(SharedString::from(format!("activity-open-run-{}", run.id))).ghost().xsmall().icon(Icon::new(Lucide::ExternalLink)).tooltip(tr(cx, "activity:openRun")).on_click(move |_, _, cx| cx.open_url(&url))
    });
    let earlier_line = (!earlier.is_empty()).then(|| {
        div()
            .id(SharedString::from(format!("activity-earlier-{}", run.id)))
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .pl_1()
            .text_xs()
            .text_color(muted)
            .child(tr(cx, "activity:earlierAttempts"))
            .children(earlier.iter().map(|attempt| {
                let (bg, fg) = tone_colors(attempt.tone(), cx);
                let chip = div().flex().items_center().gap_1().px_1().child(pill(run_label(attempt, cx), bg, fg)).child(text_with(language, "activity:attempt", &[("n", &attempt.run_attempt.to_string())]));
                let url = attempt.html_url.clone();
                chip.id(SharedString::from(format!("activity-attempt-{}", attempt.id)))
                    .when(attempt.opens_on_github(), |chip| chip.cursor_pointer().rounded(px(4.)).hover(move |style| style.bg(hover)).on_click(move |_, _, cx| cx.open_url(&url)))
            }))
            .test_support()
    });
    let hung = (!deployments.is_empty()).then(|| {
        div().w_full().flex().flex_wrap().gap_2().pl_1().children(deployments.iter().map(|deployment| {
            let (bg, fg) = status_colors(deployment.deployment.status, cx);
            let server = deployment.deployment.server_id.clone();
            let env = if deployment.deployable_env.is_empty() { "—".to_string() } else { deployment.deployable_env.clone() };
            div()
                .id(SharedString::from(format!("activity-triggered-{}", deployment.deployment.id)))
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .text_xs()
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .tooltip({
                    let tip = tr(cx, "activity:openService");
                    move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                })
                .on_click(move |_, _, cx| open_service(server.clone(), cx))
                .child(Icon::new(Lucide::Rocket).size_3())
                .child(text_with(language, "activity:triggered", &[("service", &deployment.deployable_name), ("env", &env)]))
                .child(pill(tr(cx, deployment.deployment.status.label_key()), bg, fg))
                .test_support()
        }))
    });
    let right = div()
        .ml_auto()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(muted)
        .when(run.run_attempt > 1, |right| right.child(text_with(language, "activity:attempt", &[("n", &run.run_attempt.to_string())])))
        .when(!run.actor.is_empty(), |right| right.child(text_with(language, "activity:by", &[("actor", &run.actor)])))
        .child(when)
        .children(open);
    div()
        .id(SharedString::from(format!("activity-run-{}", run.id)))
        .aria_label(label.clone())
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x_3()
        .gap_y_1()
        .px_3()
        .py_2()
        .child(pill(label, bg, fg))
        .child(div().font_weight(FontWeight::MEDIUM).child(if run.workflow_name.is_empty() { run.path.clone() } else { run.workflow_name.clone() }))
        .child(div().font_family(mono.clone()).text_xs().text_color(muted).child(run.repo_full_name.clone()))
        .when(!run.head_branch.is_empty(), |line| line.child(div().flex().items_center().gap_1().text_xs().text_color(muted).child(Icon::new(Lucide::GitBranch).size_3()).child(run.head_branch.clone())))
        .when(!run.head_sha.is_empty(), |line| line.child(div().font_family(mono.clone()).text_xs().child(run.head_sha.chars().take(7).collect::<String>())))
        .when(!run.commit_title.is_empty(), |line| line.child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(run.commit_title.clone())))
        .child(right)
        .children(earlier_line)
        .children(hung)
}

fn deployment_row(row: &ActivityDeployment, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (muted, mono, danger) = (theme.muted_foreground, theme.mono_font_family.clone(), theme.danger);
    let deployment = &row.deployment;
    let (bg, fg) = status_colors(deployment.status, cx);
    let label = tr(cx, deployment.status.label_key());
    let who = who(deployment, cx);
    let language = locale(cx);
    let when = time_ago(&deployment.created_at, &chrono::Local::now(), language);
    let image = short_ref(if deployment.final_image.is_empty() { &deployment.image } else { &deployment.final_image }).to_string();
    let server = deployment.server_id.clone();
    div()
        .id(SharedString::from(format!("activity-deployment-{}", deployment.id)))
        .aria_label(label.clone())
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x_3()
        .gap_y_1()
        .px_3()
        .py_2()
        .child(pill(label, bg, fg))
        .child(Icon::new(Lucide::Rocket).size_3p5().text_color(muted))
        .child(div().font_weight(FontWeight::MEDIUM).child(if row.deployable_name.is_empty() { deployment.deployable_id.clone() } else { row.deployable_name.clone() }))
        .when(!row.deployable_env.is_empty(), |line| line.child(div().text_xs().text_color(muted).child(row.deployable_env.clone())))
        .child(div().font_family(mono).text_xs().child(image))
        .when(!deployment.error.is_empty(), |line| line.child(div().flex_1().min_w_0().truncate().text_xs().text_color(danger).child(deployment.error.clone())))
        .child(
            div()
                .ml_auto()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(muted)
                .when(!who.is_empty(), |right| right.child(text_with(language, "activity:deployment.by", &[("who", &who)])))
                .child(when)
                .child(
                    Button::new(SharedString::from(format!("activity-open-server-{}", deployment.id)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Server))
                        .tooltip(tr(cx, "activity:openService"))
                        .on_click(move |_, _, cx| open_service(server.clone(), cx)),
                ),
        )
}
