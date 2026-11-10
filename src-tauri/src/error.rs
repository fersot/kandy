use serde::{Serialize, Serializer};

/// Error único de la aplicación. Se serializa como texto para que el
/// frontend reciba un mensaje legible en el `catch` de `invoke`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Msg(String),
    #[error("Error de red: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Error de base de datos: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("Datos inválidos: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("La sesión de la cuenta ha caducado o fue revocada. Vuelve a conectarla.")]
    AuthExpired,
    #[error("Operación cancelada")]
    Cancelled,
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::Msg(e.to_string())
    }
}

impl From<url::ParseError> for AppError {
    fn from(e: url::ParseError) -> Self {
        AppError::Msg(format!("URL inválida: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// Atajo para devolver un error con mensaje.
pub fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(AppError::Msg(msg.into()))
}

/// Comprueba el estado HTTP y convierte las respuestas de error en `AppError`
/// con un fragmento del cuerpo (útil para diagnosticar problemas de API).
pub async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let url = resp.url().clone();
    let body = resp.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        log::warn!("401 en {url}: {body}");
        return Err(AppError::AuthExpired);
    }
    let snippet: String = body.chars().take(400).collect();
    Err(AppError::Msg(format!(
        "El servidor respondió {status} ({}): {snippet}",
        url.host_str().unwrap_or("?")
    )))
}
