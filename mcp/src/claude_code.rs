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
        return Registration::Unknown(first_line(&said));
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

/// Tapa el token en un texto que viene de `claude`.
fn scrub(text: &str, token: &str) -> String {
    if token.is_empty() {
        return text.to_string();
    }
    text.replace(token, "•••")
}

fn failed(what: &str, output: &Output, token: &str) -> RegisterError {
    let said = first_line(&format!("{}\n{}", output.stderr, output.stdout));
    RegisterError::Failed(scrub(&format!("{what}: {said}"), token))
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
        Registration::Unknown(reason) => return Err(RegisterError::Failed(scrub(&reason, token))),
    }
    let added = cli.run(&add_args(cac_url, token, binary)).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => RegisterError::NoClaude,
        _ => RegisterError::Failed(scrub(&error.to_string(), token)),
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
        pub absent: bool,
    }

    impl FakeClaude {
        pub fn new(registered: Option<(&str, &str)>) -> Self {
            Self {
                registered: Mutex::new(registered.map(|(c, s)| (c.to_string(), s.to_string()))),
                calls: Mutex::new(vec![]),
                add_fails_echoing: false,
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
                    Output { success: false, stdout: String::new(), stderr: format!("error: bad env {}", args.join(" ")) }
                }
                "add" => {
                    if registered.is_some() {
                        return Ok(Output { success: false, stderr: "MCP server cac already exists".into(), ..Default::default() });
                    }
                    *registered = Some((args.last().unwrap().clone(), "user".into()));
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

    /// Sin `claude` se dice, para ofrecer el paso a mano.
    #[test]
    fn without_claude_it_says_so() {
        let claude = FakeClaude { absent: true, ..FakeClaude::new(None) };
        assert_eq!(register(&claude, "https://cac.example", TOKEN, Path::new("/b/cac-mcp"), false), Err(RegisterError::NoClaude));
    }
}
