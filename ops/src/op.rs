//! 1Password desde la máquina local: las bóvedas a las que se puede escribir y
//! crear un ítem con secretos que se enseñan una sola vez.
//!
//! Sale de la app de Tauri para que la app nativa use lo mismo. Todo `op` se
//! lanza con `cac_mcp::host::command`; los valores nunca van en el argv.

use cac_mcp::host;

use crate::ssh::ephemeral_suffix;

/// One place a secret can be filed: an account and one of its vaults.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OpVault {
    pub account: String, // the sign-in URL, which is what `--account` takes
    pub email: String,
    pub vault: String,
}

/// Lists every vault the signed-in accounts can write to.
///
/// Exists because the vault used to be a text box defaulting to "Private", and
/// that only exists in *some* accounts — a personal one has it, a business one
/// typically has "Employee" and named vaults instead. Typing the wrong name
/// failed at save time with a message from `op` that didn't say which of your
/// accounts it had even tried.
///
/// Returns an empty list rather than an error when 1Password isn't reachable:
/// the caller hides the feature instead of showing a scary message next to
/// secrets the user still needs to copy.
pub fn list_vaults() -> Vec<OpVault> {
    let accounts = match host::command("op")
        .args(["account", "list", "--format", "json"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return vec![],
    };
    let accounts: serde_json::Value = match serde_json::from_slice(&accounts) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut out = vec![];
    for a in accounts.as_array().unwrap_or(&vec![]) {
        let url = a
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let email = a
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if url.is_empty() {
            continue;
        }
        let vaults = match host::command("op")
            .args(["vault", "list", "--account", &url, "--format", "json"])
            .output()
        {
            Ok(o) if o.status.success() => o.stdout,
            // One unreachable account shouldn't hide the others.
            _ => continue,
        };
        let vaults: serde_json::Value = match serde_json::from_slice(&vaults) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for v in vaults.as_array().unwrap_or(&vec![]) {
            if let Some(name) = v.get("name").and_then(|x| x.as_str()) {
                out.push(OpVault {
                    account: url.clone(),
                    email: email.clone(),
                    vault: name.to_string(),
                });
            }
        }
    }
    out
}

/// Stores secrets that are shown exactly once as a new 1Password item, and
/// returns an `op://` reference per field.
///
/// The ingest key and the webhook secret are unrecoverable after the dialog
/// closes: cac keeps only a hash of one and never returns the other. Copying
/// them into a scratch file is what people actually do, so this offers the
/// alternative in the same breath — and hands back references a deploy config
/// can use directly instead of the literal values.
///
/// Values travel in a **0600 temp file passed with `--template`**, never in
/// argv: 1Password's own docs warn that command arguments land in shell history
/// and are readable by other processes. (Assignment statements and a template
/// on stdin were both tried; stdin silently discards the values.)
pub fn item_create(
    title: &str,
    account: &str,
    vault: &str,
    fields: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    use std::io::Write as _;

    if fields.is_empty() {
        return Err("Nothing to save".into());
    }
    if account.trim().is_empty() || vault.trim().is_empty() {
        return Err("Pick an account and a vault first".into());
    }

    let op_err = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => {
            "1Password CLI (op) not found. Install from https://developer.1password.com/docs/cli/"
                .to_string()
        }
        _ => format!("Failed to execute op: {e}"),
    };
    let friendly = |stderr: &str, status: std::process::ExitStatus| {
        let s = stderr.trim();
        if s.contains("not currently signed in") || s.contains("session expired") {
            "1Password session expired. Run `op signin` (or open the desktop app) and try again."
                .to_string()
        } else if s.is_empty() {
            format!("op exited with {status}")
        } else {
            s.to_string()
        }
    };

    // Start from the real category template so the item looks native in the app.
    // --account on every call: with more than one account signed in, letting op
    // pick means the item can land somewhere the user never chose.
    let tpl_out = host::command("op")
        .args([
            "item",
            "template",
            "get",
            "API Credential",
            "--account",
            account,
        ])
        .output()
        .map_err(op_err)?;
    if !tpl_out.status.success() {
        return Err(friendly(
            &String::from_utf8_lossy(&tpl_out.stderr),
            tpl_out.status,
        ));
    }
    let mut tpl: serde_json::Value = serde_json::from_slice(&tpl_out.stdout)
        .map_err(|e| format!("Unexpected op template: {e}"))?;

    let arr = tpl
        .get_mut("fields")
        .and_then(|f| f.as_array_mut())
        .ok_or("op template has no fields")?;
    for (name, value) in fields {
        arr.push(serde_json::json!({
            "id": name, "type": "CONCEALED", "label": name, "value": value
        }));
    }

    // 0600 before anything is written, and removed on every path out.
    let path = std::env::temp_dir().join(format!("cac-op-{}.json", ephemeral_suffix()));
    let write_template = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(tpl.to_string().as_bytes())
    };
    write_template().map_err(|e| format!("Could not stage the template: {e}"))?;

    // The category lives in the template; passing --category too is an error.
    let out = host::command("op")
        .args([
            "item",
            "create",
            "--account",
            account,
            "--vault",
            vault,
            "--title",
            title,
            "--format",
            "json",
        ])
        .arg("--template")
        .arg(&path)
        .output();
    let _ = std::fs::remove_file(&path);
    let out = out.map_err(op_err)?;
    if !out.status.success() {
        return Err(friendly(&String::from_utf8_lossy(&out.stderr), out.status));
    }

    // Reference by item id, not title: a title with spaces or a duplicate name
    // would make the reference ambiguous.
    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("Unexpected op output: {e}"))?;
    let id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if id.is_empty() {
        return Err("op created the item but returned no id".into());
    }
    Ok(fields
        .iter()
        .map(|(name, _)| (name.clone(), format!("op://{vault}/{id}/{name}")))
        .collect())
}

