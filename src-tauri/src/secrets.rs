//! Almacenamiento de tokens y contraseñas.
//!
//! Usa `oo7`: dentro de Flatpak guarda los secretos en un llavero cifrado
//! propio de la app con la clave que entrega el portal `org.freedesktop.portal.Secret`
//! (GNOME Keyring o KWallet); fuera del sandbox habla con el Secret Service por D-Bus.
//! Si no hay ningún servicio de secretos disponible, recurre a un archivo con
//! permisos 0600 en el directorio de datos de la app (y lo registra en el log).

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::{Mutex, OnceCell};

use crate::error::{AppError, Result};

const APP_ATTR: &str = "io.github.fersot.Kandy";

pub struct Secrets {
    keyring: OnceCell<Option<oo7::Keyring>>,
    fallback_path: PathBuf,
    fallback_lock: Mutex<()>,
}

impl Secrets {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            keyring: OnceCell::new(),
            fallback_path: data_dir.join("secrets.json"),
            fallback_lock: Mutex::new(()),
        }
    }

    async fn keyring(&self) -> Option<&oo7::Keyring> {
        self.keyring
            .get_or_init(|| async {
                match oo7::Keyring::new().await {
                    Ok(k) => {
                        if let Err(e) = k.unlock().await {
                            log::warn!("No se pudo desbloquear el llavero: {e}");
                        }
                        Some(k)
                    }
                    Err(e) => {
                        log::warn!(
                            "Llavero del sistema no disponible ({e}); se usará {}",
                            self.fallback_path.display()
                        );
                        None
                    }
                }
            })
            .await
            .as_ref()
    }

    fn attrs(key: &str) -> [(&'static str, String); 2] {
        [
            ("application", APP_ATTR.to_string()),
            ("account", key.to_string()),
        ]
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        if let Some(k) = self.keyring().await {
            match k
                .create_item(&format!("Kandy: {key}"), &Self::attrs(key), value, true)
                .await
            {
                Ok(()) => {
                    // Si existía una copia en el archivo de respaldo, se elimina.
                    let _ = self.file_remove(key).await;
                    return Ok(());
                }
                Err(e) => log::warn!("No se pudo guardar en el llavero: {e}"),
            }
        }
        self.file_set(key, value).await
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        if let Some(k) = self.keyring().await {
            match k.search_items(&Self::attrs(key)).await {
                Ok(items) => {
                    if let Some(item) = items.first() {
                        let secret = item
                            .secret()
                            .await
                            .map_err(|e| AppError::Msg(format!("Llavero: {e}")))?;
                        return Ok(Some(
                            String::from_utf8_lossy(secret.as_bytes()).into_owned(),
                        ));
                    }
                }
                Err(e) => log::warn!("No se pudo leer del llavero: {e}"),
            }
        }
        self.file_get(key).await
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        if let Some(k) = self.keyring().await {
            if let Err(e) = k.delete(&Self::attrs(key)).await {
                log::warn!("No se pudo borrar del llavero: {e}");
            }
        }
        self.file_remove(key).await
    }

    // ---------- Respaldo en archivo ----------

    async fn file_load(&self) -> HashMap<String, String> {
        match tokio::fs::read_to_string(&self.fallback_path).await {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    async fn file_save(&self, map: &HashMap<String, String>) -> Result<()> {
        let data = serde_json::to_vec(map)?;
        tokio::fs::write(&self.fallback_path, data).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            tokio::fs::set_permissions(&self.fallback_path, perms).await?;
        }
        Ok(())
    }

    async fn file_set(&self, key: &str, value: &str) -> Result<()> {
        let _g = self.fallback_lock.lock().await;
        let mut map = self.file_load().await;
        map.insert(key.to_string(), value.to_string());
        self.file_save(&map).await
    }

    async fn file_get(&self, key: &str) -> Result<Option<String>> {
        let _g = self.fallback_lock.lock().await;
        Ok(self.file_load().await.remove(key))
    }

    async fn file_remove(&self, key: &str) -> Result<()> {
        let _g = self.fallback_lock.lock().await;
        let mut map = self.file_load().await;
        if map.remove(key).is_some() {
            self.file_save(&map).await?;
        }
        Ok(())
    }
}
