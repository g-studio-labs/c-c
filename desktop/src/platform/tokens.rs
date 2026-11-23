//! Tokens, en las DevTools (`app/src/pages/CryptoTools.tsx`): leer y
//! verificar un JWT, generar ids, hashes y HMAC, hashes de contraseña y
//! codificar. Todo lo hace `cac_ops::crypto`, en esta máquina; aquí se pinta.

use cac_ops::crypto::{self, JwtDecoded, TimeClaim};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::lang::{locale, tr};
use crate::theme::Leading as _;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Jwt,
    Ids,
    Hash,
    Password,
    Encode,
}

const SECTIONS: [(Section, &str, &str); 5] = [
    (Section::Jwt, "tokens-section-jwt", "common:devtools.tokens.sections.jwt"),
    (Section::Ids, "tokens-section-ids", "common:devtools.tokens.sections.ids"),
    (Section::Hash, "tokens-section-hash", "common:devtools.tokens.sections.hash"),
    (Section::Password, "tokens-section-password", "common:devtools.tokens.sections.password"),
    (Section::Encode, "tokens-section-encode", "common:devtools.tokens.sections.encode"),
];

const HASHES: [(&str, &str); 6] = [("md5", "MD5"), ("sha256", "SHA-256"), ("sha384", "SHA-384"), ("sha512", "SHA-512"), ("hmac-sha256", "HMAC-SHA256"), ("hmac-sha512", "HMAC-SHA512")];

pub struct TokensView {
    section: Section,
    token: Entity<InputState>,
    secret: Entity<InputState>,
    algorithm: &'static str,
    decoded: Option<JwtDecoded>,
    id_kind: &'static str,
    id_count: Entity<InputState>,
    ids: Vec<String>,
    hash_input: Entity<InputState>,
    hash_algorithm: &'static str,
    hmac_key: Entity<InputState>,
    hash_output: String,
    password: Entity<InputState>,
    password_algorithm: &'static str,
    cost: Entity<InputState>,
    password_output: String,
    busy: bool,
    verify_hash: Entity<InputState>,
    verified: Option<bool>,
    encode_input: Entity<InputState>,
    codec: &'static str,
    encode_output: String,
    error: Option<String>,
    /// Lo último copiado, para su marca.
    copied: Option<String>,
}

impl TokensView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (hash_placeholder, password_placeholder, encode_placeholder) =
            (tr(cx, "common:devtools.tokens.hashPlaceholder"), tr(cx, "common:devtools.tokens.passwordPlaceholder"), tr(cx, "common:devtools.tokens.encodePlaceholder"));
        let verify_placeholder = "$2b$12$... / $argon2id$...".to_string();
        let mut field = |placeholder: String, masked: bool, value: &str| {
            let value = value.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(masked).default_value(value))
        };
        TokensView {
            section: Section::Jwt,
            token: field("eyJhbGciOiJIUzI1NiIs...".into(), false, ""),
            secret: field("your-secret-key".into(), true, ""),
            algorithm: "HS256",
            decoded: None,
            id_kind: "uuid-v4",
            id_count: field(String::new(), false, "1"),
            ids: Vec::new(),
            hash_input: field(hash_placeholder, false, ""),
            hash_algorithm: "sha256",
            hmac_key: field("HMAC key".into(), true, ""),
            hash_output: String::new(),
            password: field(password_placeholder, false, ""),
            password_algorithm: "bcrypt",
            cost: field(String::new(), false, "12"),
            password_output: String::new(),
            busy: false,
            verify_hash: field(verify_placeholder, false, ""),
            verified: None,
            encode_input: field(encode_placeholder, false, ""),
            codec: "base64",
            encode_output: String::new(),
            error: None,
            copied: None,
        }
    }

    fn value(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().to_string()
    }

    fn decode(&mut self, cx: &mut Context<Self>) {
        let token = Self::value(&self.token, cx).trim().to_string();
        if token.is_empty() {
            return;
        }
        let secret = Self::value(&self.secret, cx).trim().to_string();
        let secret = (!secret.is_empty()).then_some(secret);
        let algorithm = secret.as_ref().map(|_| self.algorithm);
        self.decoded = Some(crypto::jwt_decode(&token, secret.as_deref(), algorithm));
        cx.notify();
    }

    fn generate(&mut self, cx: &mut Context<Self>) {
        let count = Self::value(&self.id_count, cx).trim().parse::<usize>().unwrap_or(1).clamp(1, 100);
        self.error = None;
        self.ids = (0..count).filter_map(|_| crypto::generate_id(self.id_kind).map_err(|error| self.error = Some(error)).ok()).collect();
        cx.notify();
    }

    fn hash(&mut self, cx: &mut Context<Self>) {
        let input = Self::value(&self.hash_input, cx);
        if input.trim().is_empty() {
            return;
        }
        let result = if self.hash_algorithm.starts_with("hmac-") { crypto::hmac_sign(&input, &Self::value(&self.hmac_key, cx), self.hash_algorithm) } else { crypto::hash_text(&input, self.hash_algorithm) };
        match result {
            Ok(result) => {
                self.hash_output = result.hash;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    /// bcrypt y argon2 tardan a propósito: fuera del hilo que pinta.
    fn password_hash(&mut self, verify: bool, cx: &mut Context<Self>) {
        let input = Self::value(&self.password, cx);
        if input.trim().is_empty() || self.busy {
            return;
        }
        let (algorithm, cost) = (self.password_algorithm, Self::value(&self.cost, cx).trim().parse::<u32>().unwrap_or(12).clamp(4, 31));
        let against = Self::value(&self.verify_hash, cx).trim().to_string();
        if verify && against.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        self.verified = None;
        cx.notify();
        let work = cx.background_executor().spawn(async move {
            match (verify, algorithm) {
                (true, "bcrypt") => crypto::bcrypt_verify(&input, &against).map(|ok| (None, Some(ok))),
                (true, _) => crypto::argon2_verify(&input, &against).map(|ok| (None, Some(ok))),
                (false, "bcrypt") => crypto::bcrypt_hash(&input, cost).map(|hash| (Some(hash), None)),
                (false, _) => crypto::argon2_hash(&input).map(|hash| (Some(hash), None)),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((hash, verified)) => {
                        if let Some(hash) = hash {
                            this.password_output = hash;
                        }
                        this.verified = verified;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn encode(&mut self, direction: &str, cx: &mut Context<Self>) {
        let input = Self::value(&self.encode_input, cx);
        if input.trim().is_empty() {
            return;
        }
        match crypto::encode_decode(&input, self.codec, direction) {
            Ok(output) => {
                self.encode_output = output;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn copy(&mut self, id: String, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(id);
        cx.notify();
    }

    /// Un resultado en su caja, con su botón de copiar.
    fn output(&self, id: &'static str, value: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        if value.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let (panel, mono, success) = (theme.muted, theme.mono_font_family.clone(), theme.success);
        let copied = self.copied.as_deref() == Some(id);
        let text = value.to_string();
        Some(
            div()
                .id(id)
                .aria_label(text.clone())
                .relative()
                .max_h(px(300.))
                .overflow_y_scroll()
                .rounded(px(6.))
                .bg(panel.opacity(0.5))
                .p_3()
                .pr_10()
                .font_family(mono)
                .text_xs()
                .child(text.clone())
                .child(
                    div().absolute().top_1().right_1().child(
                        Button::new(SharedString::from(format!("{id}-copy")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if copied { Lucide::Check } else { Lucide::Copy }).when(copied, |icon| icon.text_color(success)))
                            .tooltip(tr(cx, if copied { "common:devtools.copied" } else { "common:devtools.copy" }))
                            .on_click(cx.listener(move |this, _, _, cx| this.copy(id.to_string(), text.clone(), cx))),
                    ),
                )
                .test_support()
                .into_any_element(),
        )
    }

    fn chips(id: &str, options: &[(&'static str, &'static str)], chosen: &'static str, cx: &mut Context<Self>, pick: fn(&mut Self, &'static str)) -> Div {
        div().flex().flex_wrap().gap_1().children(options.iter().map(|(value, label)| {
            let value: &'static str = value;
            Button::new(SharedString::from(format!("{id}-{value}")))
                .small()
                .when(chosen == value, |chip| chip.primary())
                .when(chosen != value, |chip| chip.outline())
                .label(*label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    pick(this, value);
                    cx.notify();
                }))
        }))
    }

    fn label(key: &str, cx: &App) -> Div {
        div().text_xs().font_weight(FontWeight::MEDIUM).child(tr(cx, key))
    }

    fn field(id: &'static str, key: &str, input: &Entity<InputState>, cx: &App) -> Div {
        div().flex_1().min_w_0().flex().flex_col().gap_1p5().child(Self::label(key, cx)).child(div().id(id).child(Input::new(input)).test_support())
    }

    fn jwt(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (muted, danger, success, warning, border, mono) = (theme.muted_foreground, theme.danger, theme.success, theme.warning, theme.border, theme.mono_font_family.clone());
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(Self::field("tokens-jwt-token", "common:devtools.tokens.token", &self.token, cx))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(Self::field("tokens-jwt-secret", "common:devtools.tokens.secret", &self.secret, cx))
                    .child(div().flex().flex_col().gap_1p5().child(Self::label("common:devtools.tokens.algorithm", cx)).child(Self::chips("tokens-jwt-alg", &[("HS256", "HS256"), ("HS384", "HS384"), ("HS512", "HS512")], self.algorithm, cx, |this, value| this.algorithm = value)))
                    .child(Button::new("tokens-jwt-decode").primary().small().label(tr(cx, "common:devtools.tokens.decode")).on_click(cx.listener(|this, _, _, cx| this.decode(cx)))),
            );
        let Some(decoded) = self.decoded.clone() else { return body };
        if let Some(error) = decoded.error.clone() {
            return body.child(div().id("tokens-jwt-error").text_color(danger).child(error).test_support());
        }
        if let Some(valid) = decoded.signature_valid {
            let (icon, key, color) = if valid { (Lucide::ShieldCheck, "common:devtools.tokens.valid", success) } else { (Lucide::ShieldX, "common:devtools.tokens.invalid", danger) };
            let phrase = tr(cx, key);
            body = body.child(
                div()
                    .id("tokens-jwt-signature")
                    .aria_label(phrase.clone())
                    .flex()
                    .child(div().flex().items_center().gap_1().px_2().py_0p5().rounded(px(4.)).bg(color.opacity(0.15)).text_color(color).text_xs().font_weight(FontWeight::MEDIUM).child(Icon::new(icon).size_3()).child(phrase))
                    .test_support(),
            );
        }
        body = body.child(div().flex().flex_col().gap_1().child(Self::label("common:devtools.tokens.header", cx).text_color(muted)).children(self.output("tokens-jwt-header", &decoded.header, cx)));
        // Las fechas, en palabras: el payload crudo sigue debajo tal cual.
        let claims = crypto::time_claims(&decoded.payload, chrono::Utc::now().timestamp());
        if !claims.is_empty() {
            let language = locale(cx);
            let line = |claim: &TimeClaim| {
                let at = chrono::DateTime::from_timestamp(claim.at, 0).map(|at| cac_core::date_and_time(language, at.with_timezone(&chrono::Local).naive_local())).unwrap_or_else(|| claim.at.to_string());
                let note = if claim.expired {
                    Some((tr(cx, "common:devtools.tokens.expired"), danger))
                } else if claim.not_yet {
                    Some((tr(cx, "common:devtools.tokens.notYet"), warning))
                } else if claim.key == "exp" {
                    Some((tr(cx, "common:devtools.tokens.stillValid"), success))
                } else {
                    None
                };
                div()
                    .id(SharedString::from(format!("tokens-claim-{}", claim.key)))
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().w(px(80.)).flex_none().text_color(muted).child(tr(cx, &format!("common:devtools.tokens.claims.{}", claim.key))))
                    .child(div().font_family(mono.clone()).child(at))
                    .children(note.map(|(note, color)| div().text_color(color).child(note)))
                    .test_support()
            };
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::label("common:devtools.tokens.inWords", cx).text_color(muted))
                    .child(div().flex().flex_col().gap_1().p_2().rounded(px(4.)).border_1().border_color(border).text_xs().children(claims.iter().map(line))),
            );
        }
        body.child(div().flex().flex_col().gap_1().child(Self::label("common:devtools.tokens.payload", cx).text_color(muted)).children(self.output("tokens-jwt-payload", &decoded.payload, cx)))
    }

    fn ids(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (panel, mono) = (theme.muted, theme.mono_font_family.clone());
        let list = (!self.ids.is_empty()).then(|| {
            let all = self.ids.join("\n");
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(self.ids.iter().enumerate().map(|(index, id)| {
                    let text = id.clone();
                    let key = format!("tokens-id-{index}");
                    let shown_copied = self.copied.as_deref() == Some(key.as_str());
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .rounded(px(4.))
                        .bg(panel.opacity(0.5))
                        .font_family(mono.clone())
                        .text_xs()
                        .child(div().flex_1().child(id.clone()))
                        .child(Button::new(SharedString::from(format!("{key}-copy"))).ghost().xsmall().icon(Icon::new(if shown_copied { Lucide::Check } else { Lucide::Copy })).on_click(cx.listener(move |this, _, _, cx| this.copy(key.clone(), text.clone(), cx))))
                }))
                .when(self.ids.len() > 1, |list| {
                    list.child(div().flex().child(Button::new("tokens-ids-copy-all").outline().small().icon(Icon::new(Lucide::Copy)).label(tr(cx, "common:devtools.copyAll")).on_click(cx.listener(move |this, _, _, cx| this.copy("tokens-ids-all".into(), all.clone(), cx)))))
                })
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().flex().flex_col().gap_1p5().child(Self::label("common:devtools.tokens.kind", cx)).child(Self::chips("tokens-id-kind", &[("uuid-v4", "UUID v4"), ("uuid-v7", "UUID v7"), ("cuid2", "CUID2")], self.id_kind, cx, |this, value| this.id_kind = value)))
                    .child(div().w(px(96.)).child(Self::field("tokens-id-count", "common:devtools.tokens.count", &self.id_count, cx)))
                    .child(Button::new("tokens-generate").primary().small().icon(Icon::new(Lucide::RefreshCw)).label(tr(cx, "common:devtools.tokens.generate")).on_click(cx.listener(|this, _, _, cx| this.generate(cx)))),
            )
            .children(list)
    }

    fn hashing(&mut self, cx: &mut Context<Self>) -> Div {
        let hmac = self.hash_algorithm.starts_with("hmac-");
        let output = self.hash_output.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(Self::field("tokens-hash-input", "common:devtools.tokens.input", &self.hash_input, cx))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().flex().flex_col().gap_1p5().child(Self::label("common:devtools.tokens.algorithm", cx)).child(Self::chips("tokens-hash-alg", &HASHES, self.hash_algorithm, cx, |this, value| this.hash_algorithm = value)))
                    .when(hmac, |row| row.child(Self::field("tokens-hash-key", "common:devtools.tokens.key", &self.hmac_key, cx)))
                    .child(Button::new("tokens-hash-run").primary().small().label(tr(cx, "common:devtools.tokens.hash")).on_click(cx.listener(|this, _, _, cx| this.hash(cx)))),
            )
            .children(self.output("tokens-hash-output", &output, cx))
    }

    fn passwords(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (border, success, danger, muted) = (theme.border, theme.success, theme.danger, theme.muted_foreground);
        let output = self.password_output.clone();
        let verified = self.verified.map(|ok| {
            let (icon, key, color) = if ok { (Lucide::ShieldCheck, "common:devtools.tokens.match", success) } else { (Lucide::ShieldX, "common:devtools.tokens.noMatch", danger) };
            let phrase = tr(cx, key);
            div().id("tokens-verified").aria_label(phrase.clone()).flex().child(div().flex().items_center().gap_1().px_2().py_0p5().rounded(px(4.)).bg(color.opacity(0.15)).text_color(color).text_xs().child(Icon::new(icon).size_3()).child(phrase)).test_support()
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(Self::field("tokens-password", "common:devtools.tokens.password", &self.password, cx))
                    .child(div().flex().flex_col().gap_1p5().child(Self::label("common:devtools.tokens.algorithm", cx)).child(Self::chips("tokens-password-alg", &[("bcrypt", "Bcrypt"), ("argon2", "Argon2")], self.password_algorithm, cx, |this, value| this.password_algorithm = value)))
                    .when(self.password_algorithm == "bcrypt", |row| row.child(div().w(px(80.)).child(Self::field("tokens-cost", "common:devtools.tokens.cost", &self.cost, cx))))
                    .child(Button::new("tokens-password-hash").primary().small().label(tr(cx, "common:devtools.tokens.hash")).disabled(self.busy).on_click(cx.listener(|this, _, _, cx| this.password_hash(false, cx)))),
            )
            .children(self.output("tokens-password-output", &output, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .pt_4()
                    .border_t_1()
                    .border_color(border)
                    .child(Self::label("common:devtools.tokens.verify", cx))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap_2()
                            .child(div().flex_1().flex().flex_col().gap_1p5().child(div().text_xs().text_color(muted).child(tr(cx, "common:devtools.tokens.verifyAgainst"))).child(div().id("tokens-verify-hash").child(Input::new(&self.verify_hash)).test_support()))
                            .child(Button::new("tokens-verify").outline().small().label(tr(cx, "common:devtools.tokens.verify")).disabled(self.busy).on_click(cx.listener(|this, _, _, cx| this.password_hash(true, cx)))),
                    )
                    .children(verified),
            )
    }

    fn encoding(&mut self, cx: &mut Context<Self>) -> Div {
        let output = self.encode_output.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(Self::field("tokens-encode-input", "common:devtools.tokens.input", &self.encode_input, cx))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().flex().flex_col().gap_1p5().child(Self::label("common:devtools.tokens.codec", cx)).child(Self::chips("tokens-codec", &[("base64", "Base64"), ("url", "URL"), ("hex", "Hex")], self.codec, cx, |this, value| this.codec = value)))
                    .child(Button::new("tokens-encode").primary().small().label(tr(cx, "common:devtools.tokens.encode")).on_click(cx.listener(|this, _, _, cx| this.encode("encode", cx))))
                    .child(Button::new("tokens-decode").outline().small().label(tr(cx, "common:devtools.tokens.decode")).on_click(cx.listener(|this, _, _, cx| this.encode("decode", cx)))),
            )
            .children(self.output("tokens-encode-output", &output, cx))
    }
}

impl Render for TokensView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, background, danger) = (theme.muted_foreground, theme.border, theme.background, theme.danger);
        // Chips y no pestañas: son cinco herramientas que viven juntas, no
        // cinco vistas de lo mismo.
        let chips = div().flex().flex_wrap().gap_1p5().pb_3().children(SECTIONS.iter().map(|(section, id, key)| {
            let section = *section;
            let chosen = self.section == section;
            Button::new(*id)
                .small()
                .rounded_full()
                .when(chosen, |chip| chip.primary())
                .when(!chosen, |chip| chip.ghost())
                .label(tr(cx, key))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    this.error = None;
                    cx.notify();
                }))
        }));
        let body = match self.section {
            Section::Jwt => self.jwt(cx),
            Section::Ids => self.ids(cx),
            Section::Hash => self.hashing(cx),
            Section::Password => self.passwords(cx),
            Section::Encode => self.encoding(cx),
        };
        div().id("tokens-tool").size_full().overflow_y_scroll().child(
            div()
                .mx_auto()
                .w_full()
                .max_w(px(896.))
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .leading_sm()
                .child(div().flex().items_center().gap_3().text_size(rems(1.25)).font_weight(FontWeight::SEMIBOLD).child(Icon::new(Lucide::KeyRound).size_5().text_color(muted)).child(tr(cx, "common:devtools.tokens.title")))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .p_4()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(border)
                        .bg(background)
                        .child(chips)
                        .child(div().pt_4().border_t_1().border_color(border).flex().flex_col().gap_3().child(body).children(self.error.clone().map(|error| div().id("tokens-error").text_color(danger).child(error).test_support()))),
                ),
        )
    }
}
