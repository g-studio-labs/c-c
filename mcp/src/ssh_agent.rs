//! Encontrar el agente SSH de la máquina.
//!
//! Un proceso que no nace de una shell —la app lanzada desde el escritorio, o
//! este servidor lanzado por un cliente MCP— no siempre hereda
//! `SSH_AUTH_SOCK`, y entonces `ssh` y `ssh-add` fallan con «Could not open a
//! connection to your authentication agent» aunque en una terminal funcionen.
//! Así que el socket se busca aquí y se le pasa al proceso hijo a mano.
//!
//! Vive en este crate porque lo necesita Ansible (`ansible::run_playbook`), que
//! también corre el binario `cac-mcp` sin la app; la app lo usa desde aquí para
//! su terminal y su lista de llaves.

/// Outcome of asking one agent for its identities.
pub enum AgentProbe {
    Keys(usize),
    /// The socket exists but nothing answered — the classic "1Password isn't
    /// running (or its SSH agent is off)" case, worth reporting differently from
    /// a socket that simply isn't there.
    Refused,
    Missing,
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// `IdentityAgent` from ~/.ssh/config — the line 1Password's own setup guide
/// tells people to add, so it's the most reliable hint when it's there.
fn identity_agent_from_ssh_config() -> Option<String> {
    let cfg = home_dir()?.join(".ssh").join("config");
    let text = std::fs::read_to_string(cfg).ok()?;
    parse_identity_agent(&text).map(|v| expand_tilde(&v))
}

/// First uncommented `IdentityAgent` value in an ssh_config. Split out from file
/// access so it can be tested: real configs are full of commented-out variants
/// (`# IdentityAgent ~/.bitwarden-ssh-agent.sock`) and picking one of those up
/// would send us at a socket the user deliberately disabled.
fn parse_identity_agent(text: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        let lower = l.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("identityagent") else {
            continue;
        };
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '=') {
            continue;
        }
        let value = l[l.len() - rest.len()..]
            .trim_start_matches(['=', ' ', '\t'])
            .trim()
            .trim_matches('"');
        if value.is_empty() || value.eq_ignore_ascii_case("none") {
            continue;
        }
        return Some(value.to_string());
    }
    None
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

/// Every socket worth trying, most specific first. Existence is checked by the
/// caller, which also probes each one for keys.
pub fn agent_candidates() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    if let Some(sock) = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
    {
        out.push((sock, "Inherited from the environment".into()));
    }
    if let Some(sock) = identity_agent_from_ssh_config() {
        out.push((sock, "IdentityAgent in ~/.ssh/config".into()));
    }
    if let Some(home) = home_dir() {
        #[cfg(target_os = "macos")]
        out.push((
            home.join("Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock")
                .to_string_lossy()
                .to_string(),
            "1Password".into(),
        ));
        out.push((
            home.join(".1password/agent.sock")
                .to_string_lossy()
                .to_string(),
            "1Password".into(),
        ));
    }
    #[cfg(target_os = "windows")]
    out.push((
        r"\\.\pipe\openssh-ssh-agent".into(),
        "Windows OpenSSH agent".into(),
    ));
    if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR") {
        let run = std::path::PathBuf::from(run);
        out.push((
            run.join("ssh-agent.socket").to_string_lossy().to_string(),
            "systemd ssh-agent".into(),
        ));
        out.push((
            run.join("keyring/ssh").to_string_lossy().to_string(),
            "GNOME Keyring".into(),
        ));
        out.push((
            run.join("gcr/ssh").to_string_lossy().to_string(),
            "GNOME Keyring".into(),
        ));
    }

    // Keep the first mention of each socket (earlier = more specific label).
    let mut seen = std::collections::HashSet::new();
    out.retain(|(sock, _)| seen.insert(sock.clone()));
    out
}

/// The socket to use when the caller didn't pick one: the first candidate that
/// actually answers with at least one key, else the first that answers at all.
pub fn resolve_agent_socket() -> Option<String> {
    let mut answering: Option<String> = None;
    for (sock, _) in agent_candidates() {
        match probe_agent(&sock) {
            AgentProbe::Keys(n) if n > 0 => return Some(sock),
            AgentProbe::Keys(_) => {
                answering.get_or_insert(sock);
            }
            _ => continue,
        }
    }
    answering
}

pub fn probe_agent(socket: &str) -> AgentProbe {
    let exists = std::path::Path::new(socket).exists() || socket.starts_with(r"\\.\pipe\");
    let Ok(out) = crate::host::command("ssh-add")
        .arg("-l")
        .env("SSH_AUTH_SOCK", socket)
        .output()
    else {
        return AgentProbe::Missing;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stdout.contains("no identities") || stderr.contains("no identities") {
        return AgentProbe::Keys(0);
    }
    if !out.status.success() {
        return if exists {
            AgentProbe::Refused
        } else {
            AgentProbe::Missing
        };
    }
    AgentProbe::Keys(stdout.lines().filter(|l| !l.trim().is_empty()).count())
}

#[cfg(test)]
mod tests {
    use super::parse_identity_agent;

    #[test]
    fn picks_the_active_line_and_ignores_commented_ones() {
        // Shape taken from a real config: two disabled variants around the live one.
        let cfg = "\
Host *
  #  IdentityAgent ~/.bitwarden-ssh-agent.sock
  IdentityAgent /home/rv/.1password/agent.sock
  #IdentityAgent none
";
        assert_eq!(
            parse_identity_agent(cfg).as_deref(),
            Some("/home/rv/.1password/agent.sock")
        );
    }

    #[test]
    fn handles_equals_quotes_and_case() {
        let cfg = "IdentityAgent=\"~/.1password/agent.sock\"\n";
        assert_eq!(
            parse_identity_agent(cfg).as_deref(),
            Some("~/.1password/agent.sock")
        );
        let cfg = "  identityagent   /tmp/a.sock\n";
        assert_eq!(parse_identity_agent(cfg).as_deref(), Some("/tmp/a.sock"));
    }

    #[test]
    fn none_and_lookalikes_are_not_sockets() {
        // `IdentityAgent none` disables the agent — following it would be wrong.
        assert_eq!(parse_identity_agent("IdentityAgent none\n"), None);
        // A different keyword that merely starts the same way.
        assert_eq!(parse_identity_agent("IdentityAgentFoo /x\n"), None);
        assert_eq!(
            parse_identity_agent("IdentityFile ~/.ssh/id_ed25519\n"),
            None
        );
    }
}

