//! Suscripciones iCalendar de solo lectura (webcal:// o https://), p. ej.
//! festivos, calendarios públicos o el enlace "ICS secreto" de cualquier servicio.

use chrono::{DateTime, Utc};

use super::Ctx;
use crate::error::{check, fail, AppError, Result};
use crate::ical;
use crate::models::*;

pub fn normalize_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let url = if let Some(rest) = raw.strip_prefix("webcal://") {
        format!("https://{rest}")
    } else if let Some(rest) = raw.strip_prefix("webcals://") {
        format!("https://{rest}")
    } else {
        raw.to_string()
    };
    let parsed = url::Url::parse(&url)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return fail("La URL debe empezar por https://, http:// o webcal://");
    }
    Ok(parsed.to_string())
}

pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<String> {
    let resp = http
        .get(url)
        .header("Accept", "text/calendar, */*;q=0.5")
        .send()
        .await?;
    let text = check(resp).await?.text().await?;
    if !text.contains("BEGIN:VCALENDAR") {
        return fail("La URL no devolvió un calendario iCalendar válido");
    }
    Ok(text)
}

pub async fn list_calendars(ctx: &Ctx<'_>) -> Result<Vec<RemoteCalendar>> {
    Ok(vec![RemoteCalendar {
        remote_id: "ics".into(),
        name: ctx.account.name.clone(),
        color: ctx.account.config.color.clone(),
        read_only: true,
        primary: true,
    }])
}

pub async fn list_events(
    ctx: &Ctx<'_>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<RemoteEvent>> {
    let url = ctx
        .account
        .config
        .url
        .clone()
        .ok_or_else(|| AppError::Msg("Falta la URL de la suscripción".into()))?;
    // Tolera URLs webcal:// guardadas sin normalizar (p. ej. importadas).
    let text = fetch(ctx.http, &normalize_url(&url)?).await?;
    let events = ical::vevents(&text);
    Ok(ical::expand(&events, start, end, None))
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn normalizes_webcal() {
        assert_eq!(
            normalize_url("webcal://example.com/a%23b.ics").unwrap(),
            "https://example.com/a%23b.ics"
        );
        assert!(normalize_url("ftp://example.com/x.ics").is_err());
    }
}
