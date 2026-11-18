//! Leer un secreto de 1Password con `op read`.
//!
//! Vive aquí, y no en la app de escritorio, porque lo usa Ansible
//! (`ansible::run_playbook`) y Ansible lo corre también el binario `cac-mcp`
//! solo, sin la app. La app lo usa desde aquí para lo suyo (los secretos de un
//! servidor, el token de GitHub).

/// Si el ejecutable de esta app ya no está en disco (el actualizador lo
/// cambió con la app abierta). En Linux se ve en `/proc/self/exe`.
pub fn running_binary_replaced() -> bool {
    exe_link_says_deleted(std::fs::read_link("/proc/self/exe").ok().as_deref())
}


fn exe_link_says_deleted(link: Option<&std::path::Path>) -> bool {
    link.map(|p| p.to_string_lossy().ends_with(" (deleted)"))
        .unwrap_or(false)
}

/// Lee una referencia `op://` en la cuenta de 1Password que la tenga.
///
/// Una referencia no dice de qué cuenta es, y con varias abiertas (la
/// personal y la del trabajo) `op` busca sólo en la de por defecto: el ítem
/// `op://Private/…` de la personal no se encontraba si la de por defecto era
/// la del trabajo, que no tiene «Private» (jose, 2-nov-2026, el ensayo de
/// gstd). Si la de por defecto contesta que no está, se prueba en las demás, y
/// la que acierta se recuerda para esa bóveda.
pub fn read(reference: &str) -> Result<String, String> {
    retry_while_refused(
        || op_read_across(reference, op_vault_accounts(), op_read_in, op_accounts),
        OP_REFUSED_ATTEMPTS,
        std::time::Duration::from_secs(2),
    )
}

/// Cuántas veces se insiste cuando 1Password rechaza la conexión.
const OP_REFUSED_ATTEMPTS: u32 = 3;

/// 1Password rechaza a veces la primera conexión (la app aún no ha abierto
/// el aviso de aprobación, o acaba de reiniciarse) y a la siguiente contesta:
/// el 3-nov-2026 una ejecución aprobada murió a los 0,2 s por eso y hubo que
/// rehacer ensayo y aprobación. Se insiste unas veces, con una pausa, antes
/// de rendirse; cualquier otro error (sesión caducada, ítem que no existe,
/// `op` sin instalar) se devuelve a la primera, que insistir no lo arregla.
fn retry_while_refused(
    mut read: impl FnMut() -> Result<String, String>,
    attempts: u32,
    wait: std::time::Duration,
) -> Result<String, String> {
    let mut last = String::new();
    for i in 0..attempts.max(1) {
        match read() {
            Ok(v) => return Ok(v),
            Err(e) if e.contains("refused the connection") && i + 1 < attempts => {
                last = e;
                std::thread::sleep(wait);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// El cuerpo de `op_read`, con quién lee y quién lista las cuentas por fuera
/// para probarlo sin 1Password.
fn op_read_across(
    reference: &str,
    learned: &std::sync::Mutex<std::collections::HashMap<String, String>>,
    read: impl Fn(&str, Option<&str>) -> Result<String, String>,
    accounts: impl FnOnce() -> Vec<String>,
) -> Result<String, String> {
    let vault = reference.trim_start_matches("op://").split('/').next().unwrap_or("").to_string();
    let remembered = learned.lock().ok().and_then(|m| m.get(&vault).cloned());
    let first_err = match read(reference, remembered.as_deref()) {
        Ok(v) => return Ok(v),
        Err(e) => e,
    };
    if !op_not_in_this_account(&first_err) {
        return Err(first_err);
    }
    for account in accounts().into_iter().filter(|a| Some(a.as_str()) != remembered.as_deref()) {
        if let Ok(v) = read(reference, Some(&account)) {
            if let Ok(mut m) = learned.lock() {
                m.insert(vault, account);
            }
            return Ok(v);
        }
    }
    Err(format!("{first_err} (not found in any signed-in 1Password account either)"))
}

/// Lo que contesta `op` cuando la referencia no está en la cuenta que miró:
/// otra cuenta puede tenerla. Lo demás (sesión, permiso, app cerrada) no se
/// arregla probando otra cuenta.
fn op_not_in_this_account(err: &str) -> bool {
    ["isn't an item", "isn't a vault", "isn't a field", "could not find"].iter().any(|m| err.contains(m))
}

/// De qué cuenta es cada bóveda, aprendido al leer.
fn op_vault_accounts() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static M: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Las cuentas abiertas en `op`, por su dirección (lo que toma `--account`).
fn op_accounts() -> Vec<String> {
    let Ok(out) = crate::host::command("op").args(["account", "list", "--format", "json"]).output() else {
        return Vec::new();
    };
    op_account_urls(&String::from_utf8_lossy(&out.stdout))
}

fn op_account_urls(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<serde_json::Value>>(json)
        .unwrap_or_default()
        .iter()
        .filter_map(|a| a.get("url").and_then(|u| u.as_str()).map(str::to_string))
        .filter(|u| !u.is_empty())
        .collect()
}

fn op_read_in(reference: &str, account: Option<&str>) -> Result<String, String> {
    let mut args = vec!["read", "--no-newline", reference];
    if let Some(a) = account {
        args.extend(["--account", a]);
    }
    let output = crate::host::command("op")
        .args(&args)
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "1Password CLI (op) not found. Install from https://developer.1password.com/docs/cli/".to_string(),
            _ => format!("Failed to execute op: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let msg = if stderr.contains("not currently signed in")
            || stderr.contains("session expired")
        {
            "1Password session expired. Run `op signin` (or open the 1Password desktop app) and try again.".to_string()
        } else if stderr.contains("connecting to desktop app") && running_binary_replaced() {
            // 1Password comprueba qué app le llama leyendo su ejecutable, y el de
            // esta ya no existe: el actualizador lo cambió con la app abierta.
            // Lo rechaza al instante (`InvalidClientInfo` en su log), y ni
            // `op signin` ni desbloquear lo arreglan; reabrir la app, sí.
            "cac was updated while it was open, and 1Password won't talk to the old copy. Quit cac completely, open it again, and retry.".to_string()
        } else if stderr.contains("connecting to desktop app") {
            "1Password refused the connection. Unlock the 1Password app and approve the request from cac (on a tiling window manager it waits in the tray icon), then try again.".to_string()
        } else if stderr.is_empty() {
            format!("op read exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(msg);
    }

    let token = String::from_utf8(output.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_string();

    if token.is_empty() {
        return Err("1Password returned an empty value for that reference".into());
    }
    Ok(token)
}

#[cfg(test)]
mod op_retry_tests {
    use super::*;

    // Roturas que atrapa: no insistir cuando 1Password rechaza la conexión;
    // insistir con cualquier otro error (una sesión caducada no se arregla
    // esperando); insistir sin tope.
    #[test]
    fn a_refused_1password_is_asked_again_but_nothing_else_is() {
        let wait = std::time::Duration::ZERO;
        let mut calls = 0;
        let out = retry_while_refused(
            || {
                calls += 1;
                if calls < 3 {
                    Err("cloudflare_dns_token: 1Password refused the connection. Unlock…".to_string())
                } else {
                    Ok("tok".to_string())
                }
            },
            3,
            wait,
        );
        assert_eq!((out.as_deref(), calls), (Ok("tok"), 3), "a la tercera contesta y se toma");

        let mut calls = 0;
        let out = retry_while_refused(
            || {
                calls += 1;
                Err("1Password session expired. Run `op signin`".to_string())
            },
            3,
            wait,
        );
        assert!(out.is_err() && calls == 1, "otro error no se reintenta ({calls} intentos)");

        let mut calls = 0;
        let out = retry_while_refused(
            || {
                calls += 1;
                Err("1Password refused the connection".to_string())
            },
            3,
            wait,
        );
        assert!(out.unwrap_err().contains("refused") && calls == 3, "se rinde tras los intentos ({calls})");
    }
}

#[cfg(test)]
mod replaced_binary_tests {
    use super::exe_link_says_deleted;
    use std::path::Path;

    // Lo que deja el actualizador: el ejecutable de la app abierta ya no está.
    #[test]
    fn a_replaced_appimage_is_noticed() {
        assert!(exe_link_says_deleted(Some(Path::new(
            "/home/rv/.cache/tauri_current_app8SWmRl/current_app.AppImage (deleted)"
        ))));
        assert!(!exe_link_says_deleted(Some(Path::new(
            "/home/rv/cac.AppImage"
        ))));
        assert!(!exe_link_says_deleted(None));
    }
}

#[cfg(test)]
mod op_account_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const NOT_HERE: &str = "[ERROR] could not read secret 'op://Private/gstd_infra_devops/password': could not get item Private/gstd_infra_devops: \"gstd_infra_devops\" isn't an item in the \"Private\" vault.";

    // Con dos cuentas abiertas, la referencia de la personal se buscaba sólo
    // en la del trabajo (el ensayo de gstd, 2-nov-2026). Roturas que atrapa:
    // no probar las demás cuentas; probarlas ante un error que no es «no
    // está» (una sesión caducada pediría aprobación a cada cuenta); no
    // recordar la cuenta, y que la siguiente lectura vuelva a dar la vuelta.
    #[test]
    fn a_reference_is_read_in_whichever_account_has_it() {
        let learned = Mutex::new(HashMap::new());
        let calls = RefCell::new(Vec::new());
        let read = |r: &str, a: Option<&str>| {
            calls.borrow_mut().push(a.map(str::to_string));
            match a {
                Some("my.1password.com") => Ok(format!("value of {r}")),
                _ => Err(NOT_HERE.to_string()),
            }
        };
        let accounts = || vec!["dwitmexicosadecv.1password.com".to_string(), "my.1password.com".to_string()];
        let r = "op://Private/gstd_infra_devops/password";
        assert_eq!(op_read_across(r, &learned, read, accounts).unwrap(), format!("value of {r}"));
        assert_eq!(learned.lock().unwrap().get("Private").map(String::as_str), Some("my.1password.com"));

        // La segunda vez va directa a la que la tenía.
        calls.borrow_mut().clear();
        op_read_across(r, &learned, read, || panic!("no hacía falta listar cuentas")).unwrap();
        assert_eq!(*calls.borrow(), vec![Some("my.1password.com".to_string())]);

        // Si no está en ninguna, se dice el error de la primera y que se buscó.
        let learned = Mutex::new(HashMap::new());
        let e = op_read_across(r, &learned, |_, _| Err(NOT_HERE.to_string()), accounts).unwrap_err();
        assert!(e.contains("isn't an item") && e.contains("any signed-in"), "{e}");

        // Otro error no da la vuelta por las cuentas.
        let e = op_read_across(r, &learned, |_, _| Err("1Password session expired".into()), || panic!("no se prueban otras cuentas")).unwrap_err();
        assert_eq!(e, "1Password session expired");
    }

    #[test]
    fn the_signed_in_accounts_are_read_by_their_address() {
        let json = r#"[{"url":"my.1password.com","email":"a@b","user_uuid":"U1","account_uuid":"A1"},{"url":"dwitmexicosadecv.1password.com","email":"c@d"}]"#;
        assert_eq!(op_account_urls(json), vec!["my.1password.com", "dwitmexicosadecv.1password.com"]);
        assert!(op_account_urls("no es json").is_empty());
    }
}
