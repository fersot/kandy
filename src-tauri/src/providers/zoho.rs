//! Zoho Calendar API v1.
//!
//! Particularidades:
//! - El host depende del centro de datos del usuario (`calendar.zoho.eu`, ...).
//! - El rango de consulta de eventos no puede superar 31 días.
//! - Con `byinstance=true` devuelve las ocurrencias de eventos recurrentes, pero
//!   con datos mínimos; por eso se combina con una segunda consulta con detalles.
//! - Crear/editar se hace con el parámetro `eventdata` (JSON en la query).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{url_with, Ctx};
use crate::error::{check, fail, Result};
use crate::models::*;
use crate::oauth::zoho_domains;

fn api_base(ctx: &Ctx<'_>) -> String {
    let loc = ctx.account.config.zoho_location.as_deref().unwrap_or("us");
    format!("{}/api/v1", zoho_domains(loc).1)
}

#[derive(Deserialize)]
struct CalList {
    #[serde(default)]
    calendars: Vec<ZCalendar>,
}

#[derive(Deserialize)]
struct ZCalendar {
    uid: String,
    name: Option<String>,
    color: Option<String>,
    privilege: Option<String>,
    #[serde(default)]
    isdefault: bool,
}

#[derive(Deserialize, Default)]
struct EvList {
    #[serde(default)]
    events: Vec<ZEvent>,
}

#[derive(Deserialize, Clone)]
struct DateAndTime {
    timezone: Option<String>,
    start: String,
    end: String,
}

#[derive(Deserialize, Clone)]
struct ZEvent {
    uid: String,
    title: Option<String>,
    #[serde(default)]
    isallday: bool,
    etag: Option<Value>,
    location: Option<String>,
    description: Option<String>,
    #[serde(rename = "viewEventURL")]
    view_url: Option<String>,
    dateandtime: Option<DateAndTime>,
    rrule: Option<String>,
    recurrenceid: Option<String>,
    /// Duración en milisegundos.
    duration: Option<Value>,
}

#[derive(Deserialize)]
pub struct ZohoUser {
    #[serde(rename = "Email")]
    pub email: Option<String>,
    #[serde(rename = "Display_Name")]
    pub display_name: Option<String>,
}

pub async fn user_info(
    http: &reqwest::Client,
    accounts_server: &str,
    token: &str,
) -> Result<ZohoUser> {
    let resp = http
        .get(format!(
            "{}/oauth/user/info",
            accounts_server.trim_end_matches('/')
        ))
        .header("Authorization", format!("Zoho-oauthtoken {token}"))
        .send()
        .await?;
    Ok(check(resp).await?.json().await?)
}

fn value_str(v: &Option<Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// Analiza las fechas de Zoho: `20240101`, `20240101T100000Z`,
/// `20240101T100000+0530` o `20240101T100000` (en la zona `tz`).
fn parse_zoho_time(s: &str, tz: Option<&str>) -> Option<(DateTime<Utc>, bool)> {
    let s = s.trim();
    if s.len() == 8 {
        let d = NaiveDate::parse_from_str(s, "%Y%m%d").ok()?;
        return Some((date_to_utc(d), true));
    }
    if let Some(stripped) = s.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some((n.and_utc(), false));
    }
    if s.len() > 15 {
        if let Ok(d) = DateTime::parse_from_str(s, "%Y%m%dT%H%M%S%z") {
            return Some((d.with_timezone(&Utc), false));
        }
    }
    let n = NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%S").ok()?;
    let tz: Tz = tz.and_then(|t| t.parse().ok()).unwrap_or(Tz::UTC);
    let d = n.and_local_timezone(tz).earliest()?;
    Some((d.with_timezone(&Utc), false))
}

fn fmt_utc(d: &DateTime<Utc>) -> String {
    d.format("%Y%m%dT%H%M%SZ").to_string()
}

fn to_remote(e: &ZEvent, recurring_uids: &HashSet<String>) -> Option<RemoteEvent> {
    let dt = e.dateandtime.as_ref()?;
    let tz = dt.timezone.as_deref();
    let (start, start_is_date) = parse_zoho_time(&dt.start, tz)?;
    let (mut end, _) = parse_zoho_time(&dt.end, tz)?;
    let all_day = e.isallday || start_is_date;
    let duration_ms = match &e.duration {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    };
    if all_day {
        // Si viene la duración, es la fuente más fiable del número de días.
        // Si no, se asume fin inclusivo (formato que muestran los ejemplos de Zoho).
        end = match duration_ms {
            Some(ms) if ms > 0 => {
                start + Duration::days(((ms as f64) / 86_400_000.0).round().max(1.0) as i64)
            }
            _ => end + Duration::days(1),
        };
        if end <= start {
            end = start + Duration::days(1);
        }
    } else if end < start {
        end = start;
    }
    let recurring =
        e.rrule.is_some() || e.recurrenceid.is_some() || recurring_uids.contains(&e.uid);
    let recurrence_id = if recurring {
        Some(e.recurrenceid.clone().unwrap_or_else(|| {
            if all_day {
                start.format("%Y%m%d").to_string()
            } else {
                fmt_utc(&start)
            }
        }))
    } else {
        None
    };
    Some(RemoteEvent {
        remote_id: match &recurrence_id {
            Some(r) => format!("{}#{r}", e.uid),
            None => e.uid.clone(),
        },
        title: e
            .title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "(Sin título)".into()),
        description: e.description.clone().filter(|d| !d.is_empty()),
        location: e.location.clone().filter(|l| !l.is_empty()),
        all_day,
        start,
        end,
        etag: value_str(&e.etag),
        url: e.view_url.clone(),
        recurring,
        extra: Some(json!({ "uid": e.uid, "recurrenceid": recurrence_id }).to_string()),
    })
}

async fn fetch_events(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    by_instance: bool,
) -> Result<Vec<ZEvent>> {
    let mut url = url_with(&api_base(ctx), &["calendars", &cal.remote_id, "events"])?;
    let range = json!({ "start": fmt_utc(&start), "end": fmt_utc(&end) }).to_string();
    url.query_pairs_mut().append_pair("range", &range);
    if by_instance {
        url.query_pairs_mut().append_pair("byinstance", "true");
    }
    let resp = ctx
        .req(Method::GET, url)
        // Necesario para que Zoho incluya la descripción.
        .header("Accept", "application/json+large")
        .send()
        .await?;
    let text = check(resp).await?.text().await?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let list: EvList = serde_json::from_str(&text).unwrap_or_default();
    Ok(list.events)
}

pub async fn list_calendars(ctx: &Ctx<'_>) -> Result<Vec<RemoteCalendar>> {
    let mut url = url_with(&api_base(ctx), &["calendars"])?;
    url.query_pairs_mut().append_pair("category", "all");
    let list: CalList = check(ctx.req(Method::GET, url).send().await?)
        .await?
        .json()
        .await?;
    Ok(list
        .calendars
        .into_iter()
        .map(|c| {
            let p = c.privilege.unwrap_or_default().to_ascii_lowercase();
            let writable = p.is_empty()
                || [
                    "owner", "write", "edit", "modify", "manage", "delegate", "admin",
                ]
                .iter()
                .any(|k| p.contains(k));
            RemoteCalendar {
                name: c.name.unwrap_or_else(|| "Calendario".into()),
                remote_id: c.uid,
                color: c.color,
                read_only: !writable,
                primary: c.isdefault,
            }
        })
        .collect())
}

pub async fn list_events(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<RemoteEvent>> {
    let mut out: HashMap<String, RemoteEvent> = HashMap::new();
    let mut chunk_start = start;
    while chunk_start < end {
        let chunk_end = (chunk_start + Duration::days(30)).min(end);
        // 1) Detalles completos (maestros de eventos recurrentes incluidos).
        let full = fetch_events(ctx, cal, chunk_start, chunk_end, false).await?;
        let recurring_uids: HashSet<String> = full
            .iter()
            .filter(|e| e.rrule.is_some())
            .map(|e| e.uid.clone())
            .collect();
        let details: HashMap<String, ZEvent> =
            full.into_iter().map(|e| (e.uid.clone(), e)).collect();
        // 2) Ocurrencias reales (recurrencias expandidas).
        let instances = fetch_events(ctx, cal, chunk_start, chunk_end, true).await?;
        for mut inst in instances {
            if let Some(d) = details.get(&inst.uid) {
                inst.location = inst.location.or_else(|| d.location.clone());
                inst.description = inst.description.or_else(|| d.description.clone());
                inst.view_url = inst.view_url.or_else(|| d.view_url.clone());
                inst.title = inst.title.or_else(|| d.title.clone());
            }
            if let Some(r) = to_remote(&inst, &recurring_uids) {
                out.insert(r.remote_id.clone(), r);
            }
        }
        chunk_start = chunk_end;
    }
    Ok(out.into_values().collect())
}

fn event_data(ev: &NewEvent) -> Value {
    let (start, end) = if ev.all_day {
        // Zoho usa fin inclusivo en eventos de día completo.
        let last = (ev.end - Duration::days(1)).max(ev.start);
        (
            ev.start.format("%Y%m%d").to_string(),
            last.format("%Y%m%d").to_string(),
        )
    } else {
        (fmt_utc(&ev.start), fmt_utc(&ev.end))
    };
    json!({
        "title": ev.title,
        "isallday": ev.all_day,
        "location": ev.location.clone().unwrap_or_default(),
        "description": ev.description.clone().unwrap_or_default(),
        "dateandtime": { "timezone": "UTC", "start": start, "end": end },
    })
}

fn etag_value(etag: &Option<String>) -> Value {
    match etag {
        Some(e) => e
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::from(e.clone())),
        None => Value::Null,
    }
}

struct Ids {
    uid: String,
    recurrence_id: Option<String>,
}

fn ids(old: &RemoteEvent) -> Ids {
    let extra: Value = old
        .extra
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    Ids {
        uid: extra["uid"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                old.remote_id
                    .split('#')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            }),
        recurrence_id: extra["recurrenceid"].as_str().map(str::to_string),
    }
}

fn first_event(text: &str) -> Option<ZEvent> {
    let list: EvList = serde_json::from_str(text).ok()?;
    list.events.into_iter().next()
}

pub async fn create_event(ctx: &Ctx<'_>, cal: &Calendar, ev: &NewEvent) -> Result<RemoteEvent> {
    let mut url = url_with(&api_base(ctx), &["calendars", &cal.remote_id, "events"])?;
    url.query_pairs_mut()
        .append_pair("eventdata", &event_data(ev).to_string());
    let text = check(ctx.req(Method::POST, url).send().await?)
        .await?
        .text()
        .await?;
    match first_event(&text).and_then(|e| to_remote(&e, &HashSet::new())) {
        Some(mut r) => {
            r.description = r.description.or_else(|| ev.description.clone());
            r.location = r.location.or_else(|| ev.location.clone());
            Ok(r)
        }
        None => fail(format!(
            "Respuesta inesperada de Zoho al crear: {}",
            text.chars().take(300).collect::<String>()
        )),
    }
}

pub async fn update_event(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    old: &RemoteEvent,
    ev: &NewEvent,
) -> Result<RemoteEvent> {
    let ids = ids(old);
    let mut data = event_data(ev);
    data["etag"] = etag_value(&old.etag);
    if let Some(rid) = &ids.recurrence_id {
        data["recurrenceid"] = Value::from(rid.clone());
        data["recurrence_edittype"] = Value::from("only");
    }
    let mut url = url_with(
        &api_base(ctx),
        &["calendars", &cal.remote_id, "events", &ids.uid],
    )?;
    url.query_pairs_mut()
        .append_pair("eventdata", &data.to_string());
    let text = check(ctx.req(Method::PUT, url).send().await?)
        .await?
        .text()
        .await?;
    let mut updated = RemoteEvent {
        title: ev.title.clone(),
        description: ev.description.clone(),
        location: ev.location.clone(),
        all_day: ev.all_day,
        start: ev.start,
        end: ev.end,
        ..old.clone()
    };
    if let Some(e) = first_event(&text) {
        updated.etag = value_str(&e.etag).or(updated.etag);
    }
    Ok(updated)
}

pub async fn delete_event(ctx: &Ctx<'_>, cal: &Calendar, old: &RemoteEvent) -> Result<()> {
    let ids = ids(old);
    let mut url = url_with(
        &api_base(ctx),
        &["calendars", &cal.remote_id, "events", &ids.uid],
    )?;
    if let Some(rid) = &ids.recurrence_id {
        let data = json!({
            "uid": ids.uid,
            "recurrenceid": rid,
            "recurrence_edittype": "only",
            "etag": etag_value(&old.etag),
        });
        url.query_pairs_mut()
            .append_pair("eventdata", &data.to_string());
    }
    let mut rb = ctx.req(Method::DELETE, url);
    if let Some(etag) = &old.etag {
        rb = rb.header("etag", etag);
    }
    let resp = rb.send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    check(resp).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zoho_formats() {
        let (d, is_date) = parse_zoho_time("20170718T205000+0530", None).unwrap();
        assert!(!is_date);
        assert_eq!(d.to_rfc3339(), "2017-07-18T15:20:00+00:00");
        let (d, _) = parse_zoho_time("20221130T180000Z", None).unwrap();
        assert_eq!(d.to_rfc3339(), "2022-11-30T18:00:00+00:00");
        let (_, is_date) = parse_zoho_time("20241028", None).unwrap();
        assert!(is_date);
        let (d, _) = parse_zoho_time("20170718T205000", Some("Asia/Calcutta")).unwrap();
        assert_eq!(d.to_rfc3339(), "2017-07-18T15:20:00+00:00");
    }

    #[test]
    fn all_day_uses_duration() {
        let e = ZEvent {
            uid: "u".into(),
            title: Some("t".into()),
            isallday: true,
            etag: Some(json!(123)),
            location: None,
            description: None,
            view_url: None,
            dateandtime: Some(DateAndTime {
                timezone: None,
                start: "20241028".into(),
                end: "20241029".into(),
            }),
            rrule: None,
            recurrenceid: None,
            duration: Some(json!(172_800_000)),
        };
        let r = to_remote(&e, &HashSet::new()).unwrap();
        assert_eq!(r.end - r.start, Duration::days(2));
        assert_eq!(r.etag.as_deref(), Some("123"));
        assert_eq!(r.remote_id, "u");
    }
}
