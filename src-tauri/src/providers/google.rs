//! Google Calendar API v3.

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{url_with, Ctx};
use crate::error::{check, fail, Result};
use crate::models::*;

const API: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarList {
    #[serde(default)]
    items: Vec<CalendarEntry>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarEntry {
    id: String,
    summary: Option<String>,
    summary_override: Option<String>,
    background_color: Option<String>,
    access_role: Option<String>,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventList {
    #[serde(default)]
    items: Vec<GEvent>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GTime {
    date: Option<String>,
    date_time: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GEvent {
    id: String,
    status: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    html_link: Option<String>,
    etag: Option<String>,
    recurring_event_id: Option<String>,
    start: Option<GTime>,
    end: Option<GTime>,
}

#[derive(Deserialize)]
pub struct UserInfo {
    pub email: Option<String>,
    pub name: Option<String>,
}

pub async fn user_info(http: &reqwest::Client, token: &str) -> Result<UserInfo> {
    let resp = http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(token)
        .send()
        .await?;
    Ok(check(resp).await?.json().await?)
}

fn parse_gtime(t: &GTime) -> Option<(DateTime<Utc>, bool)> {
    if let Some(dt) = &t.date_time {
        return DateTime::parse_from_rfc3339(dt)
            .ok()
            .map(|d| (d.with_timezone(&Utc), false));
    }
    let d = NaiveDate::parse_from_str(t.date.as_deref()?, "%Y-%m-%d").ok()?;
    Some((date_to_utc(d), true))
}

fn to_remote(e: GEvent) -> Option<RemoteEvent> {
    if e.status.as_deref() == Some("cancelled") {
        return None;
    }
    let (start, all_day) = parse_gtime(e.start.as_ref()?)?;
    let (end, _) = parse_gtime(e.end.as_ref()?)?;
    Some(RemoteEvent {
        remote_id: e.id,
        title: e.summary.unwrap_or_else(|| "(Sin título)".into()),
        description: e.description,
        location: e.location,
        all_day,
        start,
        end,
        etag: e.etag,
        url: e.html_link,
        recurring: e.recurring_event_id.is_some(),
        extra: None,
    })
}

fn body(ev: &NewEvent) -> Value {
    let time = |d: &DateTime<Utc>| {
        if ev.all_day {
            json!({ "date": d.format("%Y-%m-%d").to_string(), "dateTime": null, "timeZone": null })
        } else {
            json!({ "dateTime": d.to_rfc3339_opts(SecondsFormat::Secs, true), "date": null })
        }
    };
    json!({
        "summary": ev.title,
        "description": ev.description.clone().unwrap_or_default(),
        "location": ev.location.clone().unwrap_or_default(),
        "start": time(&ev.start),
        "end": time(&ev.end),
    })
}

pub async fn list_calendars(ctx: &Ctx<'_>) -> Result<Vec<RemoteCalendar>> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut url = url_with(API, &["users", "me", "calendarList"])?;
        url.query_pairs_mut().append_pair("maxResults", "250");
        if let Some(p) = &page {
            url.query_pairs_mut().append_pair("pageToken", p);
        }
        let list: CalendarList = check(ctx.req(Method::GET, url).send().await?)
            .await?
            .json()
            .await?;
        for c in list.items {
            if c.access_role.as_deref() == Some("freeBusyReader") {
                continue;
            }
            out.push(RemoteCalendar {
                read_only: !matches!(c.access_role.as_deref(), Some("owner") | Some("writer")),
                name: c
                    .summary_override
                    .or(c.summary)
                    .unwrap_or_else(|| c.id.clone()),
                remote_id: c.id,
                color: c.background_color,
                primary: c.primary,
            });
        }
        match list.next_page_token {
            Some(p) => page = Some(p),
            None => break,
        }
    }
    Ok(out)
}

pub async fn list_events(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<RemoteEvent>> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut url = url_with(API, &["calendars", &cal.remote_id, "events"])?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("timeMin", &start.to_rfc3339_opts(SecondsFormat::Secs, true))
                .append_pair("timeMax", &end.to_rfc3339_opts(SecondsFormat::Secs, true))
                .append_pair("singleEvents", "true")
                .append_pair("maxResults", "2500");
            if let Some(p) = &page {
                q.append_pair("pageToken", p);
            }
        }
        let list: EventList = check(ctx.req(Method::GET, url).send().await?)
            .await?
            .json()
            .await?;
        out.extend(list.items.into_iter().filter_map(to_remote));
        match list.next_page_token {
            Some(p) => page = Some(p),
            None => break,
        }
    }
    Ok(out)
}

pub async fn create_event(ctx: &Ctx<'_>, cal: &Calendar, ev: &NewEvent) -> Result<RemoteEvent> {
    let url = url_with(API, &["calendars", &cal.remote_id, "events"])?;
    let resp = ctx.req(Method::POST, url).json(&body(ev)).send().await?;
    let e: GEvent = check(resp).await?.json().await?;
    match to_remote(e) {
        Some(r) => Ok(r),
        None => fail("Google devolvió un evento inválido"),
    }
}

pub async fn update_event(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    old: &RemoteEvent,
    ev: &NewEvent,
) -> Result<RemoteEvent> {
    let url = url_with(
        API,
        &["calendars", &cal.remote_id, "events", &old.remote_id],
    )?;
    let resp = ctx.req(Method::PATCH, url).json(&body(ev)).send().await?;
    let e: GEvent = check(resp).await?.json().await?;
    match to_remote(e) {
        Some(r) => Ok(r),
        None => fail("Google devolvió un evento inválido"),
    }
}

pub async fn delete_event(ctx: &Ctx<'_>, cal: &Calendar, old: &RemoteEvent) -> Result<()> {
    let url = url_with(
        API,
        &["calendars", &cal.remote_id, "events", &old.remote_id],
    )?;
    let resp = ctx.req(Method::DELETE, url).send().await?;
    // 410 Gone: ya estaba borrado.
    if resp.status() == reqwest::StatusCode::GONE || resp.status() == reqwest::StatusCode::NOT_FOUND
    {
        return Ok(());
    }
    check(resp).await?;
    Ok(())
}
