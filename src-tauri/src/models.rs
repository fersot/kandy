use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{fail, Result};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Google,
    Microsoft,
    Zoho,
    Caldav,
    Ics,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Zoho => "zoho",
            Self::Caldav => "caldav",
            Self::Ics => "ics",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "google" => Self::Google,
            "microsoft" => Self::Microsoft,
            "zoho" => Self::Zoho,
            "caldav" => Self::Caldav,
            "ics" => Self::Ics,
            other => return fail(format!("Proveedor desconocido: {other}")),
        })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::Microsoft => "Microsoft",
            Self::Zoho => "Zoho",
            Self::Caldav => "CalDAV",
            Self::Ics => "Suscripción ICS",
        }
    }
}

/// Configuración no secreta específica de cada proveedor.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountConfig {
    /// CalDAV: URL base del servidor. ICS: URL del feed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// CalDAV: usuario.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Zoho: dominio del centro de datos (`com`, `eu`, `in`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoho_location: Option<String>,
    /// Color elegido para suscripciones ICS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Account {
    pub id: String,
    pub provider: ProviderKind,
    pub name: String,
    pub email: Option<String>,
    pub config: AccountConfig,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Calendar {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub name: String,
    pub color: String,
    pub read_only: bool,
    pub visible: bool,
}

/// Calendario tal y como lo devuelve un proveedor remoto.
#[derive(Clone, Debug)]
pub struct RemoteCalendar {
    pub remote_id: String,
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
    pub primary: bool,
}

/// Evento normalizado: horas en UTC. En eventos de día completo `start` y
/// `end` son medianoche UTC de la fecha y `end` es exclusivo.
#[derive(Clone, Debug)]
pub struct RemoteEvent {
    pub remote_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub all_day: bool,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub etag: Option<String>,
    pub url: Option<String>,
    pub recurring: bool,
    /// Datos propios del proveedor (p. ej. href CalDAV), en JSON.
    pub extra: Option<String>,
}

/// Evento guardado en la caché local.
#[derive(Clone, Debug)]
pub struct StoredEvent {
    pub id: String,
    pub calendar_id: String,
    pub ev: RemoteEvent,
}

/// Datos que envía el formulario de evento del frontend.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDraft {
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub all_day: bool,
    /// ISO-8601 en UTC (`2024-05-01T10:00:00.000Z`) o fecha (`2024-05-01`).
    pub start: String,
    pub end: String,
}

/// Borrador ya validado y normalizado a UTC.
#[derive(Clone, Debug)]
pub struct NewEvent {
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub all_day: bool,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn date_to_utc(d: NaiveDate) -> DateTime<Utc> {
    d.and_hms_opt(0, 0, 0).unwrap().and_utc()
}

pub fn parse_any_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d")
        .ok()
        .map(date_to_utc)
}

impl EventDraft {
    pub fn normalize(&self) -> Result<NewEvent> {
        let title = self.title.trim();
        let title = if title.is_empty() {
            "(Sin título)"
        } else {
            title
        }
        .to_string();
        let (start, mut end) = if self.all_day {
            let parse = |s: &str| {
                s.get(..10)
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .map(date_to_utc)
            };
            match (parse(&self.start), parse(&self.end)) {
                (Some(s), Some(e)) => (s, e),
                _ => return fail("Fechas inválidas"),
            }
        } else {
            match (
                parse_any_datetime(&self.start),
                parse_any_datetime(&self.end),
            ) {
                (Some(s), Some(e)) => (s, e),
                _ => return fail("Fechas inválidas"),
            }
        };
        if end <= start {
            end = start
                + if self.all_day {
                    Duration::days(1)
                } else {
                    Duration::hours(1)
                };
        }
        Ok(NewEvent {
            title,
            description: non_empty(&self.description),
            location: non_empty(&self.location),
            all_day: self.all_day,
            start,
            end,
        })
    }
}

// ---------- Vistas que se envían al frontend ----------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarView {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub color: String,
    pub read_only: bool,
    pub visible: bool,
}

impl From<&Calendar> for CalendarView {
    fn from(c: &Calendar) -> Self {
        Self {
            id: c.id.clone(),
            account_id: c.account_id.clone(),
            name: c.name.clone(),
            color: c.color.clone(),
            read_only: c.read_only,
            visible: c.visible,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub provider: ProviderKind,
    pub name: String,
    pub email: Option<String>,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
    pub calendars: Vec<CalendarView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventView {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub url: Option<String>,
    pub read_only: bool,
    pub recurring: bool,
    pub color: String,
}

pub fn fmt_time(dt: &DateTime<Utc>, all_day: bool) -> String {
    if all_day {
        dt.format("%Y-%m-%d").to_string()
    } else {
        dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }
}

impl EventView {
    pub fn build(stored: &StoredEvent, cal: &Calendar, provider: ProviderKind) -> Self {
        let ev = &stored.ev;
        // Las instancias recurrentes de CalDAV/ICS se gestionan en el servidor original.
        let read_only = cal.read_only
            || provider == ProviderKind::Ics
            || (provider == ProviderKind::Caldav && ev.recurring);
        Self {
            id: stored.id.clone(),
            calendar_id: stored.calendar_id.clone(),
            title: ev.title.clone(),
            description: ev.description.clone(),
            location: ev.location.clone(),
            start: fmt_time(&ev.start, ev.all_day),
            end: fmt_time(&ev.end, ev.all_day),
            all_day: ev.all_day,
            url: ev.url.clone(),
            read_only,
            recurring: ev.recurring,
            color: cal.color.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub account_id: String,
    pub status: &'static str,
    pub message: Option<String>,
}

/// Paleta por defecto para calendarios sin color propio.
pub const PALETTE: [&str; 10] = [
    "#3b82f6", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6", "#ec4899", "#14b8a6", "#f97316",
    "#6366f1", "#84cc16",
];

/// Normaliza colores como `#RRGGBBAA`, `RRGGBB` o `#RGB` a `#rrggbb`.
pub fn normalize_color(c: &str) -> Option<String> {
    let hex = c.trim().trim_start_matches('#');
    let hex = match hex.len() {
        3 => hex.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
        6 => hex.to_string(),
        8 => hex[..6].to_string(),
        _ => return None,
    };
    hex.chars()
        .all(|ch| ch.is_ascii_hexdigit())
        .then(|| format!("#{}", hex.to_ascii_lowercase()))
}
