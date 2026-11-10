use serde::{Deserialize, Serialize};

/// Ajustes de la aplicación. Las credenciales OAuth pueden venir:
///  1. de lo que el usuario escriba en Ajustes (se guarda en la BD local), o
///  2. de variables de entorno en tiempo de compilación
///     (`KANDY_GOOGLE_CLIENT_ID`, `KANDY_MICROSOFT_CLIENT_ID`, ...),
///     pensado para distribuir la app ya configurada.
///
/// Para apps de escritorio (clientes públicos) los "client secret" de Google y
/// Zoho no son realmente secretos: ambos proveedores los exigen igualmente.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub google_client_id: String,
    pub google_client_secret: String,
    pub microsoft_client_id: String,
    pub microsoft_tenant: String,
    pub zoho_client_id: String,
    pub zoho_client_secret: String,
    pub zoho_redirect_port: u16,
    pub sync_interval_minutes: u32,
    pub reminder_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            google_client_id: String::new(),
            google_client_secret: String::new(),
            microsoft_client_id: String::new(),
            microsoft_tenant: "common".into(),
            zoho_client_id: String::new(),
            zoho_client_secret: String::new(),
            zoho_redirect_port: 8977,
            sync_interval_minutes: 15,
            reminder_minutes: 10,
        }
    }
}

fn pick(user: &str, built_in: Option<&'static str>) -> String {
    let user = user.trim();
    if !user.is_empty() {
        user.to_string()
    } else {
        built_in.unwrap_or_default().trim().to_string()
    }
}

impl Settings {
    pub fn google_client_id(&self) -> String {
        pick(
            &self.google_client_id,
            option_env!("KANDY_GOOGLE_CLIENT_ID"),
        )
    }
    pub fn google_client_secret(&self) -> String {
        pick(
            &self.google_client_secret,
            option_env!("KANDY_GOOGLE_CLIENT_SECRET"),
        )
    }
    pub fn microsoft_client_id(&self) -> String {
        pick(
            &self.microsoft_client_id,
            option_env!("KANDY_MICROSOFT_CLIENT_ID"),
        )
    }
    pub fn microsoft_tenant(&self) -> String {
        let t = pick(
            &self.microsoft_tenant,
            option_env!("KANDY_MICROSOFT_TENANT"),
        );
        if t.is_empty() {
            "common".into()
        } else {
            t
        }
    }
    pub fn zoho_client_id(&self) -> String {
        pick(&self.zoho_client_id, option_env!("KANDY_ZOHO_CLIENT_ID"))
    }
    pub fn zoho_client_secret(&self) -> String {
        pick(
            &self.zoho_client_secret,
            option_env!("KANDY_ZOHO_CLIENT_SECRET"),
        )
    }
    pub fn zoho_port(&self) -> u16 {
        if self.zoho_redirect_port == 0 {
            8977
        } else {
            self.zoho_redirect_port
        }
    }

    /// Indica qué proveedores OAuth tienen credenciales incorporadas en el binario,
    /// para que la UI no obligue a configurarlas.
    pub fn built_in() -> BuiltIn {
        BuiltIn {
            google: option_env!("KANDY_GOOGLE_CLIENT_ID").is_some_and(|s| !s.is_empty()),
            microsoft: option_env!("KANDY_MICROSOFT_CLIENT_ID").is_some_and(|s| !s.is_empty()),
            zoho: option_env!("KANDY_ZOHO_CLIENT_ID").is_some_and(|s| !s.is_empty()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BuiltIn {
    pub google: bool,
    pub microsoft: bool,
    pub zoho: bool,
}
