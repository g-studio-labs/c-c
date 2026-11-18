//! La lista de a quién nombrar con `@`, la misma en la caja de comentar una
//! tarea y en la de escribir en el chat.
//!
//! Quién sale, quién va marcado y cuándo se cierra lo decide el core
//! (`cac_core::mention_menu` y compañía). Aquí queda lo que es de una
//! pantalla: recordar el marcado y el desplazamiento de la lista, y
//! pintarla.

use std::rc::Rc;

use cac_core::{MentionMark, MentionMenu, MentionPerson};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::theme::Leading as _;

/// Lo que una caja recuerda de su lista de menciones.
#[derive(Default)]
pub struct MentionList {
    /// Cuál va marcado, o que se cerró, para el `@` que se está escribiendo.
    pub mark: Option<MentionMark>,
    scroll: ScrollHandle,
}

impl MentionList {
    /// Mueve el marcado y deja a la vista el que queda marcado.
    pub fn moved(&mut self, menu: &MentionMenu, by: i32) {
        let mark = cac_core::mention_moved(menu, by);
        self.scroll.scroll_to_item(mark.highlighted as usize);
        self.mark = Some(mark);
    }

    /// Cierra la lista de ese `@` sin nombrar a nadie.
    pub fn closed(&mut self, menu: &MentionMenu) {
        self.mark = Some(cac_core::mention_closed(menu));
    }

    /// Lo recordado es de un `@`: cuando deja de escribirse, se olvida.
    pub fn keep(&mut self, text: &str, cursor: usize) {
        if self.mark.is_some() {
            self.mark = cac_core::mention_mark_kept(text, cursor, self.mark.take());
        }
    }

    /// Se olvida todo: otra caja, otra conversación.
    pub fn reset(&mut self) {
        self.mark = None;
    }
}

/// La lista pintada: nombre y usuario de cada uno, el marcado con el fondo
/// del que se señala con el ratón (`bg-accent` en la app de siempre), que es
/// a quien Enter va a nombrar. Pulsar a alguien llama a `pick`.
pub fn render_mentions<V: 'static>(
    menu: MentionMenu,
    list: &MentionList,
    pick: Rc<dyn Fn(&mut V, &MentionPerson, &mut Window, &mut Context<V>)>,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let highlighted = menu.highlighted as usize;
    let theme = cx.theme();
    let (border, popover, hover, muted, radius) = (theme.border, theme.popover, theme.accent, theme.muted_foreground, theme.radius);
    div()
        .id("mentions")
        .flex()
        .flex_col()
        .p_1()
        .max_h(px(220.))
        .overflow_y_scroll()
        .track_scroll(&list.scroll)
        .rounded(radius)
        .border_1()
        .border_color(border)
        .bg(popover)
        .children(menu.people.into_iter().enumerate().map(|(at, person)| {
            let named = person.name != person.username;
            let picked = person.clone();
            let pick = pick.clone();
            div()
                .id(SharedString::from(format!("mention-{}", person.id)))
                .test_support()
                .when(at == highlighted, |this| this.bg(hover))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .leading_sm()
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |this, _, window, cx| pick(this, &picked, window, cx)))
                .when(named, |this| this.child(person.name.clone()))
                .child(div().text_color(muted).child(format!("@{}", person.username)))
        }))
        .test_support()
}
