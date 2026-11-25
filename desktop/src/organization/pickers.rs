//! Un menú para elegir una cosa de entre varias, como chip: lo que se ve es
//! lo elegido, y al pulsarlo salen las demás. Es el desplegable de las
//! pantallas de la organización (`Picker` en la app de siempre).

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::*;

/// Una fila del menú: una opción, o la cabecera de un grupo.
#[derive(Clone)]
pub(crate) enum Choice {
    Option { value: String, label: String },
    Header(String),
}

impl Choice {
    pub(crate) fn option(value: impl Into<String>, label: impl Into<String>) -> Choice {
        Choice::Option { value: value.into(), label: label.into() }
    }
}

/// Cada opción lleva por id `{id}-{valor}`, que es por donde la encuentran
/// las pruebas.
pub(crate) fn choice_chip(id: impl Into<SharedString>, shown: String, choices: Vec<Choice>, checked: String, disabled: bool, on_pick: impl Fn(String, &mut Window, &mut App) + 'static) -> impl IntoElement {
    let on_pick = std::rc::Rc::new(on_pick);
    let id: SharedString = id.into();
    let menu_id = id.clone();
    Button::new(id)
        .outline()
        .small()
        .label(shown)
        .icon(Icon::new(Lucide::ChevronDown))
        .disabled(disabled)
        .dropdown_menu(move |mut menu, _, _| {
            for choice in &choices {
                match choice {
                    Choice::Header(label) => menu = menu.label(label.clone()),
                    Choice::Option { value, label } => {
                        let (on_pick, value, label) = (on_pick.clone(), value.clone(), label.clone());
                        let item = SharedString::from(format!("{menu_id}-{value}"));
                        let is_checked = value == checked;
                        menu = menu.item(
                            PopupMenuItem::element(move |_, _| div().id(item.clone()).child(label.clone()).test_support())
                                .checked(is_checked)
                                .on_click({
                                    let value = value.clone();
                                    move |_, window, cx| on_pick(value.clone(), window, cx)
                                }),
                        );
                    }
                }
            }
            menu.max_h(px(320.)).scrollable(true)
        })
}
