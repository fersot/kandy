//! Microsoft Graph (Outlook.com y Microsoft 365).

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{url_with, Ctx};
use crate::error::{check, Result};
use crate::models::*;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// Pide a Graph las horas en UTC y las descripciones en texto plano.
fn prefer(rb: RequestBuilder) -> RequestBuilder {
    rb.header(
        "Prefer",
        "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"",
    )
}

#[derive(Deserialize)]
struct Page<T> {
    value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsCalendar {
    id: String,
    name: Option<String>,
    hex_color: Option<String>,
    color: Option<String>,
    #[serde(default = "yes")]
    can_edit: bool,
    #[serde(default)]
    is_default_calendar: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsTime {
    date_time: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsBody {
    content: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsLocation {
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsEvent {
    id: String,
    subject: Option<String>,
    body: Option<MsBody>,
    location: Option<MsLocation>,
    start: MsTime,
    end: MsTime,
    #[serde(default)]
    is_all_day: bool,
    #[serde(default)]
    is_cancelled: bool,
    web_link: Option<String>,
    #[serde(rename = "@odata.etag")]
    etag: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}

pub async fn me(http: &reqwest::Client, token: &str) -> Result<Me> {
    let resp = http
        .get(format!(
            "{GRAPH}/me?$select=displayName,mail,userPrincipalName"
        ))
        .bearer_auth(token)
        .send()
        .await?;
    Ok(check(resp).await?.json().await?)
}

fn named_color(c: &str) -> Option<String> {
    Some(
        match c {
            "lightBlue" => "#4a90d9",
            "lightGreen" => "#5cb85c",
            "lightOrange" => "#f0ad4e",
            "lightGray" => "#9e9e9e",
            "lightYellow" => "#e6c229",
            "lightTeal" => "#20b2aa",
            "lightPink" => "#e57399",
            "lightBrown" => "#a0785a",
            "lightRed" => "#d9534f",
            _ => return None,
        }
        .to_string(),
    )
}

fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    // Graph devuelve "2024-05-01T10:00:00.0000000" (sin offset; UTC por el Prefer).
    let trimmed = s.trim_end_matches('Z');
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc())
}

fn to_remote(e: MsEvent) -> Option<RemoteEvent> {
    if e.is_cancelled {
        return None;
    }
    let mut start = parse_utc(&e.start.date_time)?;
    let mut end = parse_utc(&e.end.date_time)?;
    if e.is_all_day {
        start = date_to_utc(start.date_naive());
        end = date_to_utc(end.date_naive());
    }
    Some(RemoteEvent {
        remote_id: e.id,
        title: e
            .subject
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(Sin título)".into()),
        description: e
            .body
            .and_then(|b| b.content)
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty()),
        location: e
            .location
            .and_then(|l| l.display_name)
            .filter(|l| !l.is_empty()),
        all_day: e.is_all_day,
        start,
        end,
        etag: e.etag,
        url: e.web_link,
        recurring: matches!(e.kind.as_deref(), Some("occurrence") | Some("exception")),
        extra: None,
    })
}

fn body(ev: &NewEvent) -> Value {
    let time = |d: &DateTime<Utc>| {
        let s = if ev.all_day {
            d.format("%Y-%m-%dT00:00:00").to_string()
        } else {
            d.format("%Y-%m-%dT%H:%M:%S").to_string()
        };
        json!({ "dateTime": s, "timeZone": "UTC" })
    };
    json!({
        "subject": ev.title,
        "body": { "contentType": "text", "content": ev.description.clone().unwrap_or_default() },
        "location": { "displayName": ev.location.clone().unwrap_or_default() },
        "isAllDay": ev.all_day,
        "start": time(&ev.start),
        "end": time(&ev.end),
    })
}

pub async fn list_calendars(ctx: &Ctx<'_>) -> Result<Vec<RemoteCalendar>> {
    let mut out = Vec::new();
    let mut next = Some(format!("{GRAPH}/me/calendars?$top=100"));
    while let Some(url) = next {
        let page: Page<MsCalendar> = check(ctx.req(Method::GET, &url).send().await?)
            .await?
            .json()
            .await?;
        for c in page.value {
            let color = c
                .hex_color
                .filter(|h| !h.is_empty())
                .or_else(|| c.color.as_deref().and_then(named_color));
            out.push(RemoteCalendar {
                name: c.name.unwrap_or_else(|| "Calendario".into()),
                remote_id: c.id,
                color,
                read_only: !c.can_edit,
                primary: c.is_default_calendar,
            });
        }
        next = page.next_link;
    }
    Ok(out)
}

pub async fn list_events(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<RemoteEvent>> {
    let mut url = url_with(GRAPH, &["me", "calendars", &cal.remote_id, "calendarView"])?;
    url.query_pairs_mut()
        .append_pair(
            "startDateTime",
            &start.to_rfc3339_opts(SecondsFormat::Secs, true),
        )
        .append_pair(
            "endDateTime",
            &end.to_rfc3339_opts(SecondsFormat::Secs, true),
        )
        .append_pair("$top", "250")
        .append_pair(
            "$select",
            "id,subject,body,location,start,end,isAllDay,isCancelled,webLink,type",
        );
    let mut out = Vec::new();
    let mut next = Some(url.to_string());
    while let Some(u) = next {
        let page: Page<MsEvent> = check(prefer(ctx.req(Method::GET, &u)).send().await?)
            .await?
            .json()
            .await?;
        out.extend(page.value.into_iter().filter_map(to_remote));
        next = page.next_link;
    }
    Ok(out)
}

fn fallback(ev: &NewEvent, id: String) -> RemoteEvent {
    RemoteEvent {
        remote_id: id,
        title: ev.title.clone(),
        description: ev.description.clone(),
        location: ev.location.clone(),
        all_day: ev.all_day,
        start: ev.start,
        end: ev.end,
        etag: None,
        url: None,
        recurring: false,
        extra: None,
    }
}

pub async fn create_event(ctx: &Ctx<'_>, cal: &Calendar, ev: &NewEvent) -> Result<RemoteEvent> {
    let url = url_with(GRAPH, &["me", "calendars", &cal.remote_id, "events"])?;
    let resp = prefer(ctx.req(Method::POST, url))
        .json(&body(ev))
        .send()
        .await?;
    let e: MsEvent = check(resp).await?.json().await?;
    let id = e.id.clone();
    Ok(to_remote(e).unwrap_or_else(|| fallback(ev, id)))
}

pub async fn update_event(ctx: &Ctx<'_>, old: &RemoteEvent, ev: &NewEvent) -> Result<RemoteEvent> {
    let url = url_with(GRAPH, &["me", "events", &old.remote_id])?;
    let resp = prefer(ctx.req(Method::PATCH, url))
        .json(&body(ev))
        .send()
        .await?;
    let e: MsEvent = check(resp).await?.json().await?;
    let mut r = to_remote(e).unwrap_or_else(|| fallback(ev, old.remote_id.clone()));
    r.recurring = old.recurring;
    Ok(r)
}

pub async fn delete_event(ctx: &Ctx<'_>, old: &RemoteEvent) -> Result<()> {
    let url = url_with(GRAPH, &["me", "events", &old.remote_id])?;
    let resp = ctx.req(Method::DELETE, url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    check(resp).await?;
    Ok(())
}
