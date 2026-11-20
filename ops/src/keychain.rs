//! El llavero del sistema: lo que se guarda de cada servidor en la máquina.
//!
//! Tres cosas por servidor, bajo el servicio `cac-vps`: el token de GitHub de
//! su stack, la referencia de 1Password de la que salió (para refrescarlo), y
//! la llave pública que se le ofrece por SSH. Los nombres de las cuentas son
//! los que ya hay guardados en las máquinas: cambiarlos deja huérfano todo lo
//! guardado (guardián: `tests::the_stored_names_do_not_move`).
//!
//! Los errores salen como texto, nunca con el valor dentro.

/// El servicio bajo el que va todo en el llavero.
pub const SERVICE: &str = "cac-vps";

fn token_account(server_id: &str) -> String {
    format!("github-token:{server_id}")
}

fn ref_account(server_id: &str) -> String {
    format!("op-reference:{server_id}")
}

fn ssh_key_account(server_id: &str) -> String {
    format!("ssh-key-ref:{server_id}")
}

fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())
}

// ─── El token de GitHub ──────────────────────────────────────────────────────

pub fn github_token(server_id: &str) -> Result<String, String> {
    entry(&token_account(server_id))?
        .get_password()
        .map_err(|e| e.to_string())
}

pub fn set_github_token(server_id: &str, token: &str) -> Result<(), String> {
    entry(&token_account(server_id))?
        .set_password(token)
        .map_err(|e| e.to_string())
}

pub fn delete_github_token(server_id: &str) -> Result<(), String> {
    entry(&token_account(server_id))?
        .delete_credential()
        .map_err(|e| e.to_string())
}

// ─── La referencia de 1Password del token ────────────────────────────────────

pub fn op_reference(server_id: &str) -> Result<String, String> {
    entry(&ref_account(server_id))?
        .get_password()
        .map_err(|e| e.to_string())
}

pub fn set_op_reference(server_id: &str, reference: &str) -> Result<(), String> {
    entry(&ref_account(server_id))?
        .set_password(reference)
        .map_err(|e| e.to_string())
}

pub fn delete_op_reference(server_id: &str) -> Result<(), String> {
    entry(&ref_account(server_id))?
        .delete_credential()
        .map_err(|e| e.to_string())
}

// ─── La llave SSH del servidor ───────────────────────────────────────────────

/// Guarda la llave pública que se le ofrece a este servidor. En blanco, la
/// quita (y no tenerla ya no es un error).
pub fn set_ssh_key(server_id: &str, public_key: &str) -> Result<(), String> {
    if public_key.trim().is_empty() {
        return entry(&ssh_key_account(server_id))?
            .delete_credential()
            .or(Ok(()));
    }
    entry(&ssh_key_account(server_id))?
        .set_password(public_key.trim())
        .map_err(|e| e.to_string())
}

/// La llave pública guardada para este servidor, si hay.
pub fn ssh_key(server_id: &str) -> Result<Option<String>, String> {
    match entry(&ssh_key_account(server_id))?.get_password() {
        Ok(v) => Ok(Some(v)),
        // Sólo «no hay» es no tener llave. Un llavero bloqueado o caído se
        // dice: tragárselo era conectar ofreciendo todas las llaves y acabar
        // en «Too many authentication failures», que apunta a otro sitio.
        Err(e) => no_entry_is_none(e),
    }
}

/// Lo de arriba, sin el llavero: «no hay» es `None`, cualquier otro fallo es
/// un error. Aparte para probarlo sin tocar un llavero.
fn no_entry_is_none(e: keyring::Error) -> Result<Option<String>, String> {
    match e {
        keyring::Error::NoEntry => Ok(None),
        e => Err(format!("keychain: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lo que se guarda en el llavero va a un llavero de verdad.
    ///
    /// Sin features, keyring 3 usa su almacén de pruebas en memoria y todo
    /// «se guarda» sin guardarse (ver el comentario en `Cargo.toml`). No toca
    /// el llavero de la máquina: sólo mira qué clase de credencial construye.
    /// Rotura que detecta: quitar las features de plataforma de `keyring`.
    #[test]
    fn the_keychain_is_a_real_one() {
        let entry = keyring::Entry::new(SERVICE, &ssh_key_account("probe")).unwrap();
        assert!(
            entry
                .get_credential()
                .downcast_ref::<keyring::mock::MockCredential>()
                .is_none(),
            "keyring está usando su almacén de mentira: faltan las features de plataforma"
        );
    }

    /// El servicio y las cuentas son los que ya hay guardados en las máquinas
    /// de quien usa la app. Rotura que detecta: renombrar cualquiera de los
    /// cuatro, que no rompe nada al compilar y deja a todos sin su token, su
    /// referencia y su llave (la app diría «no configurado» como si nunca se
    /// hubiera guardado). Y que dos cosas distintas compartan cuenta.
    #[test]
    fn the_stored_names_do_not_move() {
        assert_eq!(SERVICE, "cac-vps");
        assert_eq!(token_account("s1"), "github-token:s1");
        assert_eq!(ref_account("s1"), "op-reference:s1");
        assert_eq!(ssh_key_account("s1"), "ssh-key-ref:s1");
    }

    /// Sólo «no hay entrada» es no tener llave. Rotura que detecta: tragarse
    /// un llavero bloqueado como «sin llave», que conecta ofreciendo todas las
    /// llaves del agente y acaba en «Too many authentication failures».
    #[test]
    fn only_a_missing_entry_means_no_key() {
        assert_eq!(no_entry_is_none(keyring::Error::NoEntry), Ok(None));
        let locked = no_entry_is_none(keyring::Error::NoStorageAccess("locked".into()));
        assert!(matches!(&locked, Err(m) if m.starts_with("keychain: ")), "{locked:?}");
    }
}
