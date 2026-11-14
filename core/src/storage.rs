//! Dónde guarda el core lo que tiene que sobrevivir a cerrar la app.
//!
//! Cada plataforma tiene su sitio —un fichero en el escritorio, el Keystore
//! en Android, el llavero en iOS— y el core no elige: pide por este trait y
//! quien lo usa lo implementa.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Un almacén pequeño de clave y valor.
///
/// Síncrono a propósito: lo que guarda es poco, y así lo puede implementar
/// Kotlin o Swift sin corrutinas de por medio.
pub trait Storage: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn remove(&self, key: &str);
}

/// El del escritorio: un fichero por clave, sólo legible por su dueño.
///
/// No es el llavero del sistema, y es una decisión: sin Secret Service en
/// marcha (un escritorio sin GNOME ni KDE) el llavero no guarda nada, y perder
/// la sesión en cada arranque es peor que lo que había, que era `localStorage`
/// —también texto en disco—.
pub struct FileStorage {
    dir: PathBuf,
}

impl FileStorage {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }
}

impl Storage for FileStorage {
    fn get(&self, key: &str) -> Option<String> {
        fs::read_to_string(self.path(key)).ok()
    }

    /// Escribe en un temporal y lo renombra encima.
    ///
    /// Un `rename` dentro de la misma carpeta es atómico: quien lea ve el
    /// valor viejo o el nuevo, nunca medio fichero. Importa porque lo que va
    /// aquí es un refresh recién rotado: si la app muere a media escritura y
    /// arranca con el anterior, el servidor lo toma por robado y cierra la
    /// sesión en todos los dispositivos.
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let target = self.path(key);
        let temporary = self.dir.join(format!("{key}.json.tmp"));

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(value.as_bytes()).map_err(|e| e.to_string())?;
        // Al disco antes del rename: sin esto, un corte de luz puede dejar el
        // nombre nuevo apuntando a un fichero vacío.
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);

        fs::rename(&temporary, &target).map_err(|e| e.to_string())
    }

    fn remove(&self, key: &str) {
        let _ = fs::remove_file(self.path(key));
    }
}
