//! Hablar por SSH desde la máquina local: los agentes que hay, las llaves que
//! tienen, y correr un comando en un servidor.
//!
//! La autenticación la pone el agente SSH del sistema (`SSH_AUTH_SOCK`, p. ej.
//! 1Password); aquí nunca se toca una llave privada. Lo que se escribe a disco
//! es sólo la mitad **pública**, y sólo mientras dura una llamada
//! (`stage_public_key`).

use cac_mcp::host;
use cac_mcp::ssh_agent::{agent_candidates, probe_agent, resolve_agent_socket, AgentProbe};
use serde::Serialize;

// ─── SSH agent discovery ─────────────────────────────────────────────────────
//
// A desktop-launched app does NOT inherit the shell environment, so
// `SSH_AUTH_SOCK` is usually missing and every `ssh`/`ssh-add` call fails with
// "Could not open a connection to your authentication agent" — even though the
// same commands work fine in a terminal. So we locate the socket ourselves and
// hand it to the child process explicitly.

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SshAgent {
    /// Socket path (or Windows named pipe) to export as SSH_AUTH_SOCK.
    pub socket: String,
    /// Human label: which agent this is and where we found it.
    pub label: String,
    /// How many identities it holds right now.
    pub key_count: usize,
    /// "ok" (holds keys) · "empty" (answers, no keys — usually locked) ·
    /// "refused" (socket file is there but nothing is listening).
    pub status: String,
}

/// Agents the machine actually has, so the UI can let the user choose (a laptop
/// with both 1Password and GNOME Keyring running has two, holding different
/// keys). Only reachable ones are returned.
pub fn list_ssh_agents() -> Vec<SshAgent> {
    agent_candidates()
        .into_iter()
        .filter_map(|(socket, label)| match probe_agent(&socket) {
            AgentProbe::Keys(n) => Some(SshAgent {
                socket,
                label,
                key_count: n,
                status: if n > 0 { "ok".into() } else { "empty".into() },
            }),
            // Surfaced on purpose: "1Password is installed but not answering" is
            // something the user can fix, unlike a path that was never there.
            AgentProbe::Refused => Some(SshAgent {
                socket,
                label,
                key_count: 0,
                status: "refused".into(),
            }),
            AgentProbe::Missing => None,
        })
        .collect()
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyItem {
    /// Full public key line, e.g. "ssh-ed25519 AAAA… title".
    pub public_key: String,
    /// The agent's comment — for 1Password this is the item title.
    pub title: String,
    pub fingerprint: String,
    pub key_type: String,
}

fn run_ssh_add(flag: &str, socket: Option<&str>) -> Result<String, String> {
    let mut cmd = host::command("ssh-add");
    cmd.arg(flag);
    // Explicit socket: the inherited environment can't be trusted in a
    // desktop-launched app (see agent discovery above).
    if let Some(sock) = socket {
        cmd.env("SSH_AUTH_SOCK", sock);
    }
    let output = cmd.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "`ssh-add` not found on PATH.".to_string(),
        _ => format!("Failed to execute ssh-add: {e}"),
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // ssh-add exits non-zero when the agent holds nothing or is unreachable.
        if stdout.contains("no identities") || stderr.contains("no identities") {
            return Ok(String::new());
        }
        return Err(if stderr.is_empty() {
            no_agent_message()
        } else if stderr.contains("Could not open a connection") {
            // The generic ssh-add wording is a dead end for the user: say which
            // sockets were tried and what to do about it.
            format!("{stderr}\n\n{}", no_agent_message())
        } else {
            stderr
        });
    }
    Ok(stdout)
}

/// What to tell the user when no agent answers. Lists the sockets we tried,
/// because "could not open a connection" alone gives them nothing to act on.
fn no_agent_message() -> String {
    let mut refused = Vec::new();
    let mut tried = Vec::new();
    for (sock, label) in agent_candidates() {
        match probe_agent(&sock) {
            AgentProbe::Refused => refused.push(format!("{label} ({sock})")),
            _ => tried.push(format!("  • {sock}  ({label})")),
        }
    }
    if !refused.is_empty() {
        return format!(
            "Found an agent socket but nothing is listening on it:\n  • {}\n\n\
             That usually means the app that owns it isn't running — for 1Password, \
             open it, unlock it, and check Settings → Developer → \"Use the SSH agent\".",
            refused.join("\n  • ")
        );
    }
    format!(
        "No SSH agent answered. Tried:\n{}\n\nIf you use 1Password, turn on \
         Settings → Developer → \"Use the SSH agent\", then reopen this dialog.",
        tried.join("\n")
    )
}

/// Lists the keys the SSH agent currently holds — for a 1Password agent, that's
/// the keys from the enabled vaults, with the item title as the comment.
///
/// This deliberately does NOT shell out to `op`: the 1Password CLI's desktop-app
/// integration validates the process that invokes it and refuses when launched
/// from a GUI app ("connecting to desktop app: connection reset"), while the
/// agent socket works fine from the same context — it's what ssh already uses.
pub fn list_agent_ssh_keys(socket: Option<String>) -> Result<Vec<SshKeyItem>, String> {
    let sock = socket
        .filter(|s| !s.is_empty())
        .or_else(resolve_agent_socket)
        .ok_or_else(no_agent_message)?;
    let listing = run_ssh_add("-L", Some(&sock))?; // public key material + comment
    let prints = run_ssh_add("-l", Some(&sock)).unwrap_or_default(); // same order
    Ok(parse_agent_keys(&listing, &prints))
}

/// Junta la salida de `ssh-add -L` (la llave pública y su comentario) con la de
/// `ssh-add -l` (la huella), que el agente da **en el mismo orden**. Aparte de
/// lanzar `ssh-add` para poder probarla sin un agente de verdad.
fn parse_agent_keys(listing: &str, prints: &str) -> Vec<SshKeyItem> {
    // "256 SHA256:… comment (ED25519)" → fingerprint by position.
    let fingerprints: Vec<String> = prints
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split_whitespace().nth(1).unwrap_or_default().to_string())
        .collect();

    listing
        .lines()
        .filter(|l| l.starts_with("ssh-") || l.starts_with("ecdsa-"))
        .enumerate()
        .map(|(i, line)| {
            let mut parts = line.splitn(3, ' ');
            let key_type = parts.next().unwrap_or_default().to_string();
            let _material = parts.next().unwrap_or_default();
            let title = parts.next().unwrap_or("(no title)").trim().to_string();
            SshKeyItem {
                public_key: line.to_string(),
                title: if title.is_empty() {
                    "(no title)".into()
                } else {
                    title
                },
                fingerprint: fingerprints.get(i).cloned().unwrap_or_default(),
                key_type,
            }
        })
        .collect()
}

// ─── Identidad efímera ───────────────────────────────────────────────────────

/// A public key materialized on disk only for the lifetime of one ssh call.
/// ssh needs an `IdentityFile` path; the file holds the PUBLIC half only (the
/// private key never leaves the agent, which does the signing) and is removed as
/// soon as the command returns.
pub struct EphemeralIdentity {
    pub path: std::path::PathBuf,
}

impl Drop for EphemeralIdentity {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Escribe la llave pública en un fichero temporal para pasársela a `ssh -i`.
/// Lo que no sea una llave pública se rechaza: este fichero no puede llegar a
/// tener nunca la mitad privada.
pub fn stage_public_key(public_key: &str) -> Result<EphemeralIdentity, String> {
    stage_public_key_in(&std::env::temp_dir(), public_key)
}

/// `stage_public_key` en una carpeta dada: las pruebas la usan con una suya,
/// nunca con `~/.ssh`.
fn stage_public_key_in(dir: &std::path::Path, public_key: &str) -> Result<EphemeralIdentity, String> {
    let key = public_key.trim();
    if !key.starts_with("ssh-") && !key.starts_with("ecdsa-") {
        return Err("Stored SSH identity is not a public key".into());
    }

    let path = dir.join(format!("cac-ssh-{}.pub", ephemeral_suffix()));
    std::fs::write(&path, format!("{key}\n"))
        .map_err(|e| format!("Could not stage the public key: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(EphemeralIdentity { path })
}

/// Un sufijo que no se repite entre llamadas ni entre procesos, para ficheros
/// temporales (la identidad de arriba; las apps lo usan también para los suyos).
pub fn ephemeral_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{}", std::process::id())
}

// ─── SSH agent operations ─────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SshOutput {
    pub stdout: String,
    pub stderr: String,
}

// Authentication is provided by the OS SSH agent (`SSH_AUTH_SOCK`, e.g. 1Password).
// StrictHostKeyChecking=accept-new pins unknown hosts on first connect.
//
/// Los argumentos de `ssh`, compartidos por `ssh_run` y su variante con stdin.
///
/// El comando remoto es lo último y es **lo único** que viaja como argumento
/// al otro lado: por eso lo que es secreto no va aquí sino por stdin
/// (`ssh_run_input`), donde no lo ve `ps` de ninguna de las dos máquinas.
pub fn ssh_base_args(
    host: &str,
    port: u16,
    user: &str,
    remote_cmd: &str,
    identity: Option<&str>,
) -> Vec<String> {
    let target = format!("{user}@{host}");
    let port_str = port.to_string();

    let mut args: Vec<String> = vec![
        "-p".into(),
        port_str,
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
    ];

    // Pin a single key. An agent holding many keys (1Password vaults often do)
    // makes ssh offer them one by one until the server's MaxAuthTries (6 by
    // default) trips "Too many authentication failures" — before it ever reaches
    // the right one. With the 1Password agent the identity is the **public** key
    // file: ssh then asks the agent for just that one.
    if let Some(id) = identity.filter(|s| !s.trim().is_empty()) {
        args.push("-i".into());
        args.push(id.trim().to_string());
        args.push("-o".into());
        args.push("IdentitiesOnly=yes".into());
    }

    args.push(target);
    args.push(remote_cmd.to_string());
    args
}

/// `ssh_run` con algo que escribir en el stdin del comando remoto.
///
/// Es el canal para los secretos: un argumento sale en `ps` de cualquier
/// usuario de la máquina remota y queda en el historial; stdin no toca ni el
/// disco ni la tabla de procesos. El mismo contrato que `rotate-secrets` en los
/// servidores de RRHH.
pub fn ssh_run_input(
    host: &str,
    port: u16,
    user: &str,
    remote_cmd: &str,
    identity: Option<&str>,
    input: Option<&str>,
) -> Result<SshOutput, String> {
    use std::io::Write;
    use std::process::Stdio;

    let args = ssh_base_args(host, port, user, remote_cmd, identity);
    let mut cmd = host::command("ssh");
    cmd.args(&args);
    // Same reason as ssh-add: without this the desktop-launched app has no agent
    // and every connection fails with "Permission denied (publickey)".
    if let Some(sock) = resolve_agent_socket() {
        cmd.env("SSH_AUTH_SOCK", sock);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "`ssh` binary not found on PATH.".to_string(),
        _ => format!("Failed to execute ssh: {e}"),
    })?;
    if let Some(text) = input {
        // Se escribe y se cierra: el otro lado lee hasta EOF.
        let mut stdin = child.stdin.take().ok_or("ssh sin stdin")?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("no se pudo escribir en ssh: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to execute ssh: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        let trimmed = stderr.trim();
        return Err(format!(
            "ssh exited with status {}{}\n{}",
            output.status,
            failure_hint(trimmed),
            trimmed
        ));
    }

    Ok(SshOutput { stdout, stderr })
}

/// Qué decirle a quien ve fallar un `ssh`, según lo que dijo en stderr. Aparte
/// para poder probarlo sin lanzar nada.
fn failure_hint(stderr: &str) -> &'static str {
    if stderr.contains("Too many authentication failures") {
        " (your SSH agent offered more keys than the server allows before the right one. Pick this server's key with the key button in the servers table so cac offers only that one)"
    } else if stderr.contains("Permission denied") {
        " (is your 1Password SSH agent unlocked and the key authorized on this server?)"
    } else if stderr.contains("Connection refused") || stderr.contains("Connection timed out") {
        " (host unreachable or SSH port closed)"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(args: &[String], name: &str) -> bool {
        args.windows(2).any(|w| w[0] == "-o" && w[1] == name)
    }

    /// El puerto va tras `-p`, y el destino es `usuario@host` justo antes del
    /// comando remoto, que es lo último. Roturas que detecta: el puerto por
    /// defecto o en otro sitio, usuario y host al revés, el comando remoto
    /// fuera del final (ssh lo tomaría por una opción o un destino).
    #[test]
    fn port_user_and_command_go_where_ssh_reads_them() {
        let args = ssh_base_args("10.0.0.1", 2222, "deploy", "uptime", None);
        let p = args.iter().position(|a| a == "-p").expect("sin -p");
        assert_eq!(args[p + 1], "2222");
        let n = args.len();
        assert_eq!(args[n - 2], "deploy@10.0.0.1");
        assert_eq!(args[n - 1], "uptime");
    }

    /// Las opciones que hacen que un fallo sea un fallo y no una espera:
    /// `BatchMode` (sin pedir contraseña), `accept-new` (fijar el host la
    /// primera vez, y negarse si cambia) y un tope de conexión. Roturas que
    /// detecta: quitar cualquiera, o relajar el host a `no`.
    #[test]
    fn a_connection_never_waits_for_a_human() {
        let args = ssh_base_args("h", 22, "root", "true", None);
        assert!(opt(&args, "BatchMode=yes"), "{args:?}");
        assert!(opt(&args, "StrictHostKeyChecking=accept-new"), "{args:?}");
        assert!(opt(&args, "ConnectTimeout=15"), "{args:?}");
    }

    /// Con llave elegida, ssh ofrece sólo ésa; sin ella (o en blanco), no se
    /// pinta ni `-i` ni `IdentitiesOnly`, que dejaría al agente sin llave que
    /// ofrecer. Roturas que detecta: olvidar `IdentitiesOnly` (vuelve «Too many
    /// authentication failures»), no recortar la ruta, o pasar un `-i` vacío.
    #[test]
    fn a_chosen_key_is_the_only_one_offered() {
        let args = ssh_base_args("h", 22, "root", "true", Some("  /tmp/k.pub "));
        let i = args.iter().position(|a| a == "-i").expect("sin -i");
        assert_eq!(args[i + 1], "/tmp/k.pub");
        assert!(opt(&args, "IdentitiesOnly=yes"), "{args:?}");
        // El destino y el comando siguen siendo lo último.
        assert_eq!(&args[args.len() - 2..], ["root@h", "true"]);

        for none in [None, Some(""), Some("   ")] {
            let args = ssh_base_args("h", 22, "root", "true", none);
            assert!(!args.iter().any(|a| a == "-i"), "{none:?}: {args:?}");
            assert!(!opt(&args, "IdentitiesOnly=yes"), "{none:?}: {args:?}");
        }
    }

    /// Lo que sale de la llave elegida es la pública, y se va al soltarla.
    /// Roturas que detecta: aceptar algo que no es una llave pública (p. ej.
    /// una privada pegada por error), dejar el fichero tras la llamada, o
    /// dejarlo legible por otros. Usa una carpeta propia, nunca `~/.ssh`.
    #[test]
    fn a_staged_key_is_public_private_to_us_and_gone_after() {
        let dir = std::env::temp_dir().join(format!("cac-ops-test-{}", ephemeral_suffix()));
        std::fs::create_dir_all(&dir).unwrap();

        let staged = stage_public_key_in(&dir, "  ssh-ed25519 AAAAC3Nza test@cac \n").unwrap();
        let path = staged.path.clone();
        assert!(path.starts_with(&dir));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ssh-ed25519 AAAAC3Nza test@cac\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        drop(staged);
        assert!(!path.exists(), "la identidad sigue en disco tras soltarla");

        assert!(stage_public_key_in(&dir, "ecdsa-sha2-nistp256 AAAA x").is_ok());
        for not_public in ["-----BEGIN OPENSSH PRIVATE KEY-----\nabc", "", "AAAAC3Nza"] {
            assert!(stage_public_key_in(&dir, not_public).is_err(), "{not_public:?}");
        }
        // Lo rechazado no deja nada escrito.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dos llamadas seguidas no comparten fichero. Rotura que detecta: un
    /// sufijo fijo, con el que dos llamadas a la vez se pisarían la llave.
    #[test]
    fn two_suffixes_are_never_the_same() {
        assert_ne!(ephemeral_suffix(), ephemeral_suffix());
    }

    /// Cada llave de `ssh-add -L` con su huella de `ssh-add -l`, por orden; lo
    /// que no es una llave (líneas en blanco, avisos) no cuenta. Roturas que
    /// detecta: casar las huellas fuera de orden, contar las líneas que no son
    /// llaves (corre las huellas una posición), o perder el título con
    /// espacios de 1Password.
    #[test]
    fn each_key_gets_its_own_fingerprint() {
        let listing = "ssh-ed25519 AAAA1 GitHub personal\n\
                       ecdsa-sha2-nistp256 AAAA2 contabo production k8s\n\
                       ssh-rsa AAAA3\n";
        let prints = "256 SHA256:uno GitHub personal (ED25519)\n\n\
                      256 SHA256:dos contabo production k8s (ECDSA)\n\
                      3072 SHA256:tres (RSA)\n";
        let keys = parse_agent_keys(listing, prints);
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0].key_type, "ssh-ed25519");
        assert_eq!(keys[0].title, "GitHub personal");
        assert_eq!(keys[0].fingerprint, "SHA256:uno");
        assert_eq!(keys[0].public_key, "ssh-ed25519 AAAA1 GitHub personal");
        assert_eq!(keys[1].title, "contabo production k8s");
        assert_eq!(keys[1].fingerprint, "SHA256:dos");
        assert_eq!(keys[2].title, "(no title)");
        assert_eq!(keys[2].fingerprint, "SHA256:tres");

        assert!(parse_agent_keys("The agent has no identities.\n", "").is_empty());
    }

    /// Cada fallo conocido de ssh dice qué hacer. Roturas que detecta: que el
    /// de «demasiadas llaves» deje de apuntar al botón de la llave (es el que
    /// más confunde), o que un fallo desconocido se invente una pista.
    #[test]
    fn a_known_ssh_failure_says_what_to_do() {
        assert!(failure_hint("Received disconnect: Too many authentication failures").contains("key button"));
        assert!(failure_hint("root@h: Permission denied (publickey).").contains("1Password"));
        assert!(failure_hint("connect to host h port 22: Connection refused").contains("unreachable"));
        assert!(failure_hint("connect to host h port 22: Connection timed out").contains("unreachable"));
        assert_eq!(failure_hint("Host key verification failed."), "");
    }
}
