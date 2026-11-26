//! Registrar `cac-mcp` en Claude Code con el CLI `claude`, y saber cómo está.
//!
//! `claude mcp get cac` dice si hay un registro `cac`, a qué programa apunta
//! y con qué alcance; `claude mcp add` lo registra y `claude mcp remove` lo
//! quita. La app nativa lo usa desde «Conectar Claude Code».
//!
//! **El token no sale de aquí.** Va como argumento del proceso hijo y nada
//! más: lo que `claude mcp get` contesta lleva el token en claro (su bloque
//! «Environment»), así que de esa salida sólo se lee la orden y el alcance; y
//! un fallo se cuenta con el token tapado ([`RegisterError`]), porque el
//! texto acaba en pantalla o en un log.
//!
//! Se lanza por [`crate::host::command`]: dentro de la AppImage, un programa
//! del sistema que herede su entorno puede no arrancar (CLAUDE.md).

use std::ffi::OsString;
use std::path::Path;

/// El nombre con el que se registra.
pub const SERVER_NAME: &str = "cac";

/// Lo que contestó un `claude …`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Quien corre `claude`. En las pruebas, uno de mentira.
pub trait Cli: Send + Sync {
    /// `Err` con `NotFound` si `claude` no está.
    fn run(&self, args: &[OsString]) -> std::io::Result<Output>;
}

/// El `claude` del sistema, buscado en el `PATH`.
pub struct SystemClaude;

impl Cli for SystemClaude {
    fn run(&self, args: &[OsString]) -> std::io::Result<Output> {
        let output = crate::host::command("claude").args(args).stdin(std::process::Stdio::null()).output()?;
        Ok(Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Cómo está registrado `cac` en Claude Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// `claude` no está en el `PATH`: sólo queda el paso a mano.
    NoClaude,
    /// No hay registro `cac`.
    Missing,
    /// Hay uno, que lanza `command` y vive en ese alcance (`user`, `local`,
    /// `project`): es el que hay que nombrar para quitarlo.
    Present { command: String, scope: String },
    /// `claude` contestó algo que no se entiende. El texto no lleva token.
    Unknown(String),
}

/// Lo que dice la tarjeta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardState {
    NoClaude,
    Missing,
    /// Apunta a este `cac-mcp`.
    ThisBinary,
    /// Apunta a otro programa (el AppImage de la app de Tauri, otro binario).
    Other { command: String },
    Unknown,
}

impl Registration {
    /// Qué decir de este registro, sabiendo cuál es este `cac-mcp`.
    pub fn card(&self, binary: Option<&Path>) -> CardState {
        match self {
            Registration::NoClaude => CardState::NoClaude,
            Registration::Missing => CardState::Missing,
            Registration::Unknown(_) => CardState::Unknown,
            Registration::Present { command, .. } => match binary {
                Some(binary) if same_program(command, binary) => CardState::ThisBinary,
                _ => CardState::Other { command: command.clone() },
            },
        }
    }
}

fn same_program(command: &str, binary: &Path) -> bool {
    let command = Path::new(command.trim());
    if command == binary {
        return true;
    }
    // Un enlace o una ruta con `..` siguen siendo el mismo fichero.
    matches!((command.canonicalize(), binary.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// Lee lo que contesta `claude mcp get cac`. Sólo la orden y el alcance: el
/// resto (las variables, con el token) no se mira.
pub fn parse_get(output: &Output) -> Registration {
    if !output.success {
        let said = format!("{}{}", output.stdout, output.stderr);
        if said.contains("No MCP server") {
            return Registration::Missing;
        }
        // La salida de `get` lleva el token del registro que haya: se tapa
        // antes de recortar.
        return Registration::Unknown(first_line(&redact(&said, "")));
    }
    let mut command = None;
    let mut scope = None;
    for line in output.stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Command:") {
            command = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Scope:") {
            let rest = rest.trim().to_ascii_lowercase();
            scope = scope.or_else(|| ["user", "local", "project"].into_iter().find(|s| rest.starts_with(s)).map(str::to_string));
        } else if let Some((_, after)) = line.split_once("claude mcp remove") {
            // «To remove this server, run: claude mcp remove cac -s user» es
            // lo más fiable: es la orden misma.
            if let Some((_, s)) = after.split_once("-s ") {
                scope = s.split_whitespace().next().map(str::to_string);
            }
        }
    }
    match command {
        Some(command) if !command.is_empty() => Registration::Present { command, scope: scope.unwrap_or_else(|| "user".into()) },
        _ => Registration::Unknown("claude mcp get cac: no command".into()),
    }
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("").chars().take(200).collect()
}

/// Mira cómo está registrado.
pub fn look(cli: &dyn Cli) -> Registration {
    match cli.run(&args(&["mcp", "get", SERVER_NAME])) {
        Ok(output) => parse_get(&output),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Registration::NoClaude,
        Err(error) => Registration::Unknown(error.to_string()),
    }
}

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// Los argumentos de `claude` que registran el binario para toda la cuenta.
/// El nombre va antes que los `-e`: `--env` acepta varios valores y se
/// comería lo que viniera detrás.
pub fn add_args(cac_url: &str, token: &str, binary: &Path) -> Vec<OsString> {
    let mut list = args(&["mcp", "add", SERVER_NAME, "-s", "user"]);
    list.push("-e".into());
    list.push(format!("CAC_URL={}", cac_url.trim_end_matches('/')).into());
    list.push("-e".into());
    list.push(format!("CAC_TOKEN={token}").into());
    list.push("--".into());
    list.push(binary.as_os_str().to_owned());
    list
}

/// Los de quitar un registro de ese alcance.
pub fn remove_args(scope: &str) -> Vec<OsString> {
    args(&["mcp", "remove", SERVER_NAME, "-s", scope])
}

/// Por qué no se registró. Ningún texto lleva el token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    NoClaude,
    /// Ya hay un registro `cac`: se pregunta antes de quitarlo.
    NeedsConfirmation { command: String },
    Failed(String),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::NoClaude => f.write_str("claude is not on the PATH"),
            RegisterError::NeedsConfirmation { command } => write!(f, "cac is already registered with {command}"),
            RegisterError::Failed(reason) => f.write_str(reason),
        }
    }
}

const MASK: &str = "•••";

/// Lo más corto de un token que todavía se tapa al aparecer suelto: un trozo
/// de 8 ya es medio secreto, y uno más corto choca con texto cualquiera.
const SHORTEST_PIECE: usize = 8;

/// Tapa el token en un texto que viene de `claude`: entero, cualquier trozo
/// suyo de [`SHORTEST_PIECE`] o más (por si alguien ya lo partió al recortar)
/// y cualquier `cac_pat_…` o `CAC_TOKEN=…`, que puede ser el de otro registro.
/// Siempre antes de recortar: recortado primero, el corte puede caer en mitad
/// del token y el trozo que queda ya no es «el token».
fn redact(text: &str, token: &str) -> String {
    let token: Vec<char> = token.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let piece = longest_piece_of(&token, &text[i..]);
        if piece >= SHORTEST_PIECE {
            out.push_str(MASK);
            i += piece;
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    redact_marked(&out)
}

/// Lo largo del principio de `text` que es un trozo seguido de `token`.
fn longest_piece_of(token: &[char], text: &[char]) -> usize {
    (0..token.len()).map(|start| token[start..].iter().zip(text).take_while(|(a, b)| a == b).count()).max().unwrap_or(0)
}

fn redact_marked(text: &str) -> String {
    let mut out = text.to_string();
    for marker in ["CAC_TOKEN=", "cac_pat_"] {
        let mut from = 0;
        while let Some(found) = out[from..].find(marker) {
            let start = from + found + marker.len();
            let end = out[start..].find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',').map_or(out.len(), |n| start + n);
            if start == end || &out[start..end] == MASK {
                from = start;
                continue;
            }
            out.replace_range(start..end, MASK);
            from = start + MASK.len();
        }
    }
    out
}

fn failed(what: &str, output: &Output, token: &str) -> RegisterError {
    let said = redact(&format!("{}\n{}", output.stderr, output.stdout), token);
    RegisterError::Failed(first_line(&format!("{what}: {}", first_line(&said))))
}

/// Registra `binary` como `cac` en Claude Code, para toda la cuenta.
///
/// Si ya hay un registro `cac`, sólo lo quita con `replace` (la persona dijo
/// que sí); si no, contesta [`RegisterError::NeedsConfirmation`] sin tocar
/// nada. Después vuelve a mirar: lo registrado tiene que apuntar a `binary`.
pub fn register(cli: &dyn Cli, cac_url: &str, token: &str, binary: &Path, replace: bool) -> Result<(), RegisterError> {
    match look(cli) {
        Registration::NoClaude => return Err(RegisterError::NoClaude),
        Registration::Present { command, scope } => {
            if !replace {
                return Err(RegisterError::NeedsConfirmation { command });
            }
            let removed = cli.run(&remove_args(&scope)).map_err(|error| RegisterError::Failed(error.to_string()))?;
            if !removed.success {
                return Err(failed("claude mcp remove", &removed, token));
            }
        }
        Registration::Missing => {}
        // Sin saber si hay uno, no se pisa nada.
        Registration::Unknown(reason) => return Err(RegisterError::Failed(first_line(&redact(&reason, token)))),
    }
    let added = cli.run(&add_args(cac_url, token, binary)).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => RegisterError::NoClaude,
        _ => RegisterError::Failed(first_line(&redact(&error.to_string(), token))),
    })?;
    if !added.success {
        return Err(failed("claude mcp add", &added, token));
    }
    match look(cli).card(Some(binary)) {
        CardState::ThisBinary => Ok(()),
        _ => Err(RegisterError::Failed("claude mcp add finished, but cac does not point to this cac-mcp".into())),
    }
}

/// Un `claude` de mentira, para las pruebas de aquí y de la app nativa.
#[doc(hidden)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    pub const TOKEN: &str = "cac_pat_SECRET123";

    pub fn get_output(command: &str, scope: &str) -> Output {
        Output {
            success: true,
            stdout: format!(
                "cac:\n  Scope: User config (available in all your projects)\n  Status: ✔ Connected\n  Type: stdio\n  Command: {command}\n  Args:\n  Environment:\n    CAC_URL=https://cac.example\n    CAC_TOKEN={TOKEN}\n\nTo remove this server, run: claude mcp remove cac -s {scope}\n"
            ),
            stderr: String::new(),
        }
    }

    pub fn missing() -> Output {
        Output { success: false, stdout: String::new(), stderr: "No MCP server named \"cac\". Configured servers: canva".into() }
    }

    /// Un `claude` de mentira: contesta `get` con lo que tenga registrado,
    /// y apunta cada llamada.
    pub struct FakeClaude {
        pub registered: Mutex<Option<(String, String)>>,
        pub calls: Mutex<Vec<Vec<String>>>,
        pub add_fails_echoing: bool,
        /// Cuánto texto va delante de los argumentos que repite al fallar.
        pub echo_padding: usize,
        /// `add` sale bien pero deja registrado esto, no lo que se le pidió.
        pub add_registers: Option<String>,
        pub absent: bool,
    }

    impl FakeClaude {
        pub fn new(registered: Option<(&str, &str)>) -> Self {
            Self {
                registered: Mutex::new(registered.map(|(c, s)| (c.to_string(), s.to_string()))),
                calls: Mutex::new(vec![]),
                add_fails_echoing: false,
                echo_padding: 0,
                add_registers: None,
                absent: false,
            }
        }
        pub fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Cli for FakeClaude {
        fn run(&self, args: &[OsString]) -> std::io::Result<Output> {
            if self.absent {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "claude"));
            }
            let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
            self.calls.lock().unwrap().push(args.clone());
            let mut registered = self.registered.lock().unwrap();
            Ok(match args[1].as_str() {
                "get" => match registered.as_ref() {
                    Some((command, scope)) => get_output(command, scope),
                    None => missing(),
                },
                "remove" => {
                    *registered = None;
                    Output { success: true, ..Default::default() }
                }
                "add" if self.add_fails_echoing => {
                    Output { success: false, stdout: String::new(), stderr: format!("error: bad env {}{}", "x".repeat(self.echo_padding), args.join(" ")) }
                }
                "add" => {
                    if registered.is_some() {
                        return Ok(Output { success: false, stderr: "MCP server cac already exists".into(), ..Default::default() });
                    }
                    let command = self.add_registers.clone().unwrap_or_else(|| args.last().unwrap().clone());
                    *registered = Some((command, "user".into()));
                    Output { success: true, ..Default::default() }
                }
                _ => unreachable!(),
            })
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    use super::fake::{get_output, missing, FakeClaude};

    const TOKEN: &str = fake::TOKEN;

    /// Lo que contesta `claude mcp get cac` se lee: orden y alcance, y el
    /// «no hay» por lo que es.
    ///
    /// Roturas que atrapa: tomar el alcance del texto largo («User config»)
    /// y no de la orden de quitar; tomar un «no hay» por un fallo; quedarse
    /// con algo del bloque de variables.
    #[test]
    fn what_claude_says_about_cac_is_read() {
        assert_eq!(
            parse_get(&get_output("/home/ana/.local/bin/cac-mcp", "local")),
            Registration::Present { command: "/home/ana/.local/bin/cac-mcp".into(), scope: "local".into() }
        );
        assert_eq!(parse_get(&missing()), Registration::Missing);
        let odd = parse_get(&Output { success: false, stderr: "boom".into(), ..Default::default() });
        assert_eq!(odd, Registration::Unknown("boom".into()));
    }

    /// La tarjeta distingue este `cac-mcp`, otro programa, ninguno y sin
    /// `claude`.
    ///
    /// Roturas que atrapa: dar por «este» cualquier registro (el AppImage de
    /// Tauri pasaría por bueno); no ofrecer nada sin `claude`.
    #[test]
    fn the_card_says_this_binary_another_or_none() {
        let this = Path::new("/home/ana/.local/bin/cac-mcp");
        let present = |command: &str| Registration::Present { command: command.into(), scope: "user".into() };
        assert_eq!(present("/home/ana/.local/bin/cac-mcp").card(Some(this)), CardState::ThisBinary);
        assert_eq!(present("/opt/cac.AppImage").card(Some(this)), CardState::Other { command: "/opt/cac.AppImage".into() });
        assert_eq!(present("/home/ana/.local/bin/cac-mcp").card(None), CardState::Other { command: "/home/ana/.local/bin/cac-mcp".into() });
        assert_eq!(Registration::Missing.card(Some(this)), CardState::Missing);
        assert_eq!(Registration::NoClaude.card(Some(this)), CardState::NoClaude);
        let absent = FakeClaude { absent: true, ..FakeClaude::new(None) };
        assert_eq!(look(&absent), Registration::NoClaude);
    }

    /// Sin registro previo se registra para toda la cuenta, con la URL sin
    /// barra final, el token como variable y el binario tras `--`.
    ///
    /// Roturas que atrapa: registrar sin `-s user` (sólo valdría en una
    /// carpeta); el nombre detrás de los `-e` (se lo comería `--env`); un
    /// `remove` sin que hubiera nada; no comprobar que quedó apuntando aquí.
    #[test]
    fn without_a_previous_registration_it_adds_for_the_whole_account() {
        let claude = FakeClaude::new(None);
        let binary = Path::new("/home/ana/.local/bin/cac-mcp");
        register(&claude, "https://cac.example/", TOKEN, binary, false).unwrap();
        let calls = claude.calls();
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert_eq!(calls[0], ["mcp", "get", "cac"]);
        assert_eq!(
            calls[1],
            ["mcp", "add", "cac", "-s", "user", "-e", "CAC_URL=https://cac.example", "-e", &format!("CAC_TOKEN={TOKEN}"), "--", "/home/ana/.local/bin/cac-mcp"]
        );
        assert_eq!(calls[2], ["mcp", "get", "cac"]);
    }

    /// Con un registro previo, primero se pregunta: sin el sí no se quita
    /// nada; con el sí se quita en su alcance y se registra el nuevo.
    ///
    /// Roturas que atrapa: quitar el registro de la persona sin preguntar;
    /// quitarlo en otro alcance (`remove -s user` de uno `local` falla y el
    /// `add` choca); no registrar tras quitar.
    #[test]
    fn with_a_previous_registration_it_asks_before_removing_it() {
        let claude = FakeClaude::new(Some(("/opt/cac.AppImage", "local")));
        let binary = Path::new("/home/ana/.local/bin/cac-mcp");
        let asked = register(&claude, "https://cac.example", TOKEN, binary, false).unwrap_err();
        assert_eq!(asked, RegisterError::NeedsConfirmation { command: "/opt/cac.AppImage".into() });
        assert!(claude.calls().iter().all(|call| call[1] == "get"), "tocó algo sin el sí: {:?}", claude.calls());

        register(&claude, "https://cac.example", TOKEN, binary, true).unwrap();
        let calls = claude.calls();
        assert!(calls.contains(&vec!["mcp".into(), "remove".into(), "cac".into(), "-s".into(), "local".into()]), "{calls:?}");
        assert_eq!(look(&claude).card(Some(binary)), CardState::ThisBinary);
    }

    /// Un fallo de `claude` se cuenta sin el token, aunque `claude` lo
    /// repita en lo que contesta.
    ///
    /// Rotura que atrapa: pasar el texto de `claude` tal cual a la pantalla
    /// o al log, con `CAC_TOKEN=…` dentro.
    #[test]
    fn a_failure_never_carries_the_token() {
        let claude = FakeClaude { add_fails_echoing: true, ..FakeClaude::new(None) };
        let error = register(&claude, "https://cac.example", TOKEN, Path::new("/b/cac-mcp"), false).unwrap_err();
        assert!(matches!(error, RegisterError::Failed(_)));
        assert!(!error.to_string().contains(TOKEN), "{error}");
        assert!(!format!("{error:?}").contains(TOKEN), "{error:?}");
        assert!(error.to_string().contains("claude mcp add"), "{error}");
    }

    /// Un token de verdad: `cac_pat_` y 43 más, como `GeneratePAT`.
    const LONG_TOKEN: &str = "cac_pat_Q7vR2mXk9LpT4wZs8NcY1bHf6JdGa3UeKo5iVrW0qEt";

    fn no_piece_of(token: &str, text: &str) -> Result<(), String> {
        let chars: Vec<char> = token.chars().collect();
        for window in chars.windows(SHORTEST_PIECE) {
            let piece: String = window.iter().collect();
            if text.contains(&piece) {
                return Err(format!("«{piece}» del token está en: {text}"));
            }
        }
        Ok(())
    }

    /// `claude` repite sus argumentos en una línea larga y el corte de 200
    /// caracteres cae en mitad del token: no sale ningún trozo suyo, ni en
    /// lo que se pinta ni en lo que va al log (los dos son este texto).
    ///
    /// Rotura que atrapa: recortar primero y tapar después sólo el token
    /// entero (el trozo cortado ya no es «el token» y pasa).
    #[test]
    fn a_long_failure_cut_through_the_token_shows_no_piece_of_it() {
        assert_eq!(LONG_TOKEN.len(), 51);
        let binary = Path::new("/b/cac-mcp");
        // Lo que va delante del token en lo que se cuenta: «claude mcp add: »,
        // «error: bad env », el relleno y los argumentos hasta `CAC_TOKEN=`.
        let before_args = "claude mcp add: error: bad env ".chars().count();
        let up_to_token = add_args("https://cac.example", LONG_TOKEN, binary)
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
            .find("CAC_TOKEN=")
            .unwrap()
            + "CAC_TOKEN=".len();
        // El corte (200) deja 30 caracteres del token dentro.
        let padding = 200 - 30 - before_args - up_to_token;
        let claude = FakeClaude { add_fails_echoing: true, echo_padding: padding, ..FakeClaude::new(None) };
        let error = register(&claude, "https://cac.example", LONG_TOKEN, binary, false).unwrap_err();
        assert!(matches!(error, RegisterError::Failed(_)), "{error:?}");
        no_piece_of(LONG_TOKEN, &error.to_string()).unwrap();
        no_piece_of(LONG_TOKEN, &format!("{error:?}")).unwrap();
        assert!(error.to_string().contains("claude mcp add"), "{error}");
    }

    /// Lo que ya llega cortado (o es otro token) también se tapa: un trozo
    /// del token de 8 o más, cualquier `cac_pat_…` y cualquier `CAC_TOKEN=…`.
    ///
    /// Roturas que atrapa: tapar sólo el token entero; dejar pasar el token
    /// de otro registro (el que contesta `claude mcp get`).
    #[test]
    fn a_piece_of_the_token_or_another_token_is_hidden_too() {
        let piece = &LONG_TOKEN[12..40];
        let hidden = redact(&format!("could not parse {piece}"), LONG_TOKEN);
        no_piece_of(LONG_TOKEN, &hidden).unwrap();
        assert!(hidden.starts_with("could not parse "), "{hidden}");

        let other = "cac_pat_OTHERxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
        let hidden = redact(&format!("env CAC_TOKEN={other} and {other}, ok"), "");
        assert!(!hidden.contains("OTHER"), "{hidden}");
        assert!(hidden.ends_with(", ok"), "{hidden}");
        let hidden = redact("CAC_TOKEN=plainsecret rest", "");
        assert!(!hidden.contains("plainsecret"), "{hidden}");
        // Un texto sin token se queda igual.
        assert_eq!(redact("No MCP server named cac", LONG_TOKEN), "No MCP server named cac");
    }

    /// Después de añadir se vuelve a mirar, y lo que se ve decide: si `cac`
    /// sigue apuntando a otro programa, no se registró.
    ///
    /// Rotura que atrapa: dar por bueno cualquier cosa que conteste el `get`
    /// de después.
    #[test]
    fn an_add_that_leaves_another_program_is_a_failure() {
        let claude = FakeClaude { add_registers: Some("/opt/cac.AppImage".into()), ..FakeClaude::new(None) };
        let result = register(&claude, "https://cac.example", TOKEN, Path::new("/home/ana/.local/bin/cac-mcp"), false);
        assert!(matches!(result, Err(RegisterError::Failed(_))), "{result:?}");
        assert_eq!(claude.calls().len(), 3, "{:?}", claude.calls());
    }

    /// Sin `claude` se dice, para ofrecer el paso a mano.
    #[test]
    fn without_claude_it_says_so() {
        let claude = FakeClaude { absent: true, ..FakeClaude::new(None) };
        assert_eq!(register(&claude, "https://cac.example", TOKEN, Path::new("/b/cac-mcp"), false), Err(RegisterError::NoClaude));
    }
}
