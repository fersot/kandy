//! CalDAV genérico (RFC 4791): iCloud, Nextcloud, Fastmail, Yahoo, Radicale,
//! Baïkal, SOGo, Zoho (CalDAV)...
//!
//! Descubrimiento: current-user-principal → calendar-home-set → colecciones.
//! El cliente HTTP usado aquí NO sigue redirecciones automáticamente: se hace
//! a mano para conservar el método (PROPFIND/REPORT) y el cuerpo.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use reqwest::{header, Method, StatusCode};
use serde_json::json;

use super::Ctx;
use crate::error::{check, fail, AppError, Result};
use crate::ical;
use crate::models::*;

const PROPFIND: &[u8] = b"PROPFIND";
const REPORT: &[u8] = b"REPORT";

#[derive(Debug, Default)]
struct PropVal {
    text: String,
    hrefs: Vec<String>,
    children: Vec<String>,
    comps: Vec<String>,
}

#[derive(Debug, Default)]
struct DavResp {
    href: String,
    props: HashMap<String, PropVal>,
}

fn parse_multistatus(xml: &str) -> Result<Vec<DavResp>> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| AppError::Msg(format!("XML inválido del servidor CalDAV: {e}")))?;
    let mut out = Vec::new();
    for resp in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "response")
    {
        let mut r = DavResp::default();
        for child in resp.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "href" => r.href = child.text().unwrap_or_default().trim().to_string(),
                "propstat" => {
                    let ok = child
                        .children()
                        .find(|n| n.tag_name().name() == "status")
                        .and_then(|n| n.text())
                        .map(|s| s.contains(" 200"))
                        .unwrap_or(true);
                    if !ok {
                        continue;
                    }
                    let Some(prop) = child.children().find(|n| n.tag_name().name() == "prop")
                    else {
                        continue;
                    };
                    for p in prop.children().filter(|n| n.is_element()) {
                        let mut v = PropVal::default();
                        for d in p.descendants() {
                            if d.is_text() {
                                v.text.push_str(d.text().unwrap_or_default());
                            } else if d.is_element() && d != p {
                                let name = d.tag_name().name();
                                if name == "href" {
                                    v.hrefs
                                        .push(d.text().unwrap_or_default().trim().to_string());
                                }
                                if name == "comp" {
                                    if let Some(c) = d.attribute("name") {
                                        v.comps.push(c.to_ascii_uppercase());
                                    }
                                }
                                v.children.push(name.to_string());
                            }
                        }
                        v.text = v.text.trim().to_string();
                        r.props.insert(p.tag_name().name().to_string(), v);
                    }
                }
                _ => {}
            }
        }
        out.push(r);
    }
    Ok(out)
}

/// Envía una petición DAV siguiendo redirecciones manualmente.
async fn dav(
    ctx: &Ctx<'_>,
    method: Method,
    url: &url::Url,
    depth: Option<&str>,
    body: Option<String>,
) -> Result<(url::Url, reqwest::Response)> {
    let mut current = url.clone();
    for _ in 0..6 {
        let mut rb = ctx.req(method.clone(), current.clone());
        if let Some(d) = depth {
            rb = rb.header("Depth", d);
        }
        if let Some(b) = &body {
            rb = rb
                .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
                .body(b.clone());
        }
        let resp = rb.send().await?;
        if resp.status().is_redirection() {
            if let Some(loc) = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                current = current.join(loc)?;
                continue;
            }
        }
        return Ok((current, resp));
    }
    fail("Demasiadas redirecciones en el servidor CalDAV")
}

async fn propfind(
    ctx: &Ctx<'_>,
    url: &url::Url,
    depth: &str,
    props: &str,
) -> Result<(url::Url, Vec<DavResp>)> {
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
  <d:prop>{props}</d:prop>
</d:propfind>"#
    );
    let (final_url, resp) = dav(
        ctx,
        Method::from_bytes(PROPFIND).unwrap(),
        url,
        Some(depth),
        Some(body),
    )
    .await?;
    let text = check(resp).await?.text().await?;
    Ok((final_url, parse_multistatus(&text)?))
}

fn base_url(ctx: &Ctx<'_>) -> Result<url::Url> {
    let raw = ctx
        .account
        .config
        .url
        .clone()
        .ok_or_else(|| AppError::Msg("Falta la URL del servidor CalDAV".into()))?;
    let raw = raw.trim();
    let raw = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    Ok(url::Url::parse(&raw)?)
}

fn is_calendar(r: &DavResp) -> bool {
    r.props
        .get("resourcetype")
        .is_some_and(|v| v.children.iter().any(|c| c == "calendar"))
}

fn first_href(responses: &[DavResp], prop: &str) -> Option<String> {
    responses
        .iter()
        .find_map(|r| r.props.get(prop).and_then(|v| v.hrefs.first().cloned()))
        .filter(|h| !h.is_empty())
}

const COLLECTION_PROPS: &str = "<d:resourcetype/><d:displayname/><a:calendar-color/>\
<c:supported-calendar-component-set/><d:current-user-privilege-set/>";

fn to_calendar(base: &url::Url, r: &DavResp) -> Option<RemoteCalendar> {
    if !is_calendar(r) {
        return None;
    }
    if let Some(comps) = r.props.get("supported-calendar-component-set") {
        if !comps.comps.is_empty() && !comps.comps.iter().any(|c| c == "VEVENT") {
            return None; // Solo tareas (VTODO) u otros.
        }
    }
    let url = base.join(&r.href).ok()?;
    let read_only = r.props.get("current-user-privilege-set").is_some_and(|p| {
        !p.children.is_empty()
            && !p
                .children
                .iter()
                .any(|c| c == "write" || c == "write-content" || c == "all")
    });
    let name = r
        .props
        .get("displayname")
        .map(|v| v.text.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            url.path_segments()
                .and_then(|mut s| s.rfind(|p| !p.is_empty()).map(str::to_string))
                .unwrap_or_else(|| "Calendario".into())
        });
    Some(RemoteCalendar {
        remote_id: url.to_string(),
        name,
        color: r.props.get("calendar-color").map(|v| v.text.clone()),
        read_only,
        primary: false,
    })
}

pub async fn list_calendars(ctx: &Ctx<'_>) -> Result<Vec<RemoteCalendar>> {
    let base = base_url(ctx)?;
    // 1) ¿La URL ya es un calendario o expone el principal?
    let first = propfind(
        ctx,
        &base,
        "0",
        &format!("<d:current-user-principal/>{COLLECTION_PROPS}"),
    )
    .await;
    let (mut at, mut responses) = match first {
        Ok(v) => v,
        Err(AppError::AuthExpired) => {
            return fail("Usuario o contraseña incorrectos (usa una contraseña de aplicación si tu proveedor lo exige)")
        }
        Err(e) => {
            log::info!("PROPFIND en la URL base falló ({e}); probando /.well-known/caldav");
            (base.clone(), Vec::new())
        }
    };
    if let Some(c) = responses.first().and_then(|r| to_calendar(&at, r)) {
        return Ok(vec![c]);
    }
    let mut principal = first_href(&responses, "current-user-principal");
    if principal.is_none() {
        let wk = base.join("/.well-known/caldav")?;
        let (u, r) = propfind(ctx, &wk, "0", "<d:current-user-principal/>").await?;
        at = u;
        responses = r;
        principal = first_href(&responses, "current-user-principal");
    }
    let Some(principal) = principal else {
        return fail("El servidor no anunció un principal CalDAV. Revisa la URL.");
    };
    // 2) Carpeta de calendarios del usuario.
    let principal_url = at.join(&principal)?;
    let (at2, responses) = propfind(ctx, &principal_url, "0", "<c:calendar-home-set/>").await?;
    let home = first_href(&responses, "calendar-home-set")
        .ok_or_else(|| AppError::Msg("No se encontró calendar-home-set".into()))?;
    let home_url = at2.join(&home)?;
    // 3) Colecciones de calendario.
    let (at3, responses) = propfind(ctx, &home_url, "1", COLLECTION_PROPS).await?;
    let mut cals: Vec<RemoteCalendar> = responses
        .iter()
        .filter_map(|r| to_calendar(&at3, r))
        .collect();
    if let Some(first) = cals.first_mut() {
        first.primary = true;
    }
    if cals.is_empty() {
        return fail("La cuenta CalDAV no tiene calendarios de eventos");
    }
    Ok(cals)
}

pub async fn list_events(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<RemoteEvent>> {
    let url = url::Url::parse(&cal.remote_id)?;
    let fmt = |d: &DateTime<Utc>| d.format("%Y%m%dT%H%M%SZ").to_string();
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{}" end="{}"/>
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#,
        fmt(&start),
        fmt(&end)
    );
    let (at, resp) = dav(
        ctx,
        Method::from_bytes(REPORT).unwrap(),
        &url,
        Some("1"),
        Some(body),
    )
    .await?;
    let text = check(resp).await?.text().await?;
    let mut out = Vec::new();
    for r in parse_multistatus(&text)? {
        let Some(data) = r.props.get("calendar-data") else {
            continue;
        };
        if data.text.is_empty() {
            continue;
        }
        let href = at
            .join(&r.href)
            .map(|u| u.to_string())
            .unwrap_or(r.href.clone());
        let etag = r.props.get("getetag").map(|v| v.text.clone());
        let extra = json!({ "href": href }).to_string();
        let events = ical::vevents(&data.text);
        for mut ev in ical::expand(&events, start, end, Some(extra)) {
            ev.etag = etag.clone();
            out.push(ev);
        }
    }
    Ok(out)
}

fn href_of(ev: &RemoteEvent) -> Result<url::Url> {
    let extra: serde_json::Value = ev
        .extra
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    match extra["href"].as_str() {
        Some(h) => Ok(url::Url::parse(h)?),
        None => fail("El evento no tiene href CalDAV; sincroniza de nuevo"),
    }
}

fn etag_header(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn result_event(ev: &NewEvent, uid: &str, href: &url::Url, etag: Option<String>) -> RemoteEvent {
    RemoteEvent {
        remote_id: uid.to_string(),
        title: ev.title.clone(),
        description: ev.description.clone(),
        location: ev.location.clone(),
        all_day: ev.all_day,
        start: ev.start,
        end: ev.end,
        etag,
        url: None,
        recurring: false,
        extra: Some(json!({ "href": href.to_string() }).to_string()),
    }
}

pub async fn create_event(ctx: &Ctx<'_>, cal: &Calendar, ev: &NewEvent) -> Result<RemoteEvent> {
    let uid = format!("{}@kandy", uuid::Uuid::new_v4());
    let mut base = cal.remote_id.clone();
    if !base.ends_with('/') {
        base.push('/');
    }
    let href = url::Url::parse(&base)?.join(&format!("{}.ics", uuid::Uuid::new_v4()))?;
    let resp = ctx
        .req(Method::PUT, href.clone())
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .header(header::IF_NONE_MATCH, "*")
        .body(ical::build_calendar(&uid, ev))
        .send()
        .await?;
    let resp = check(resp).await?;
    Ok(result_event(ev, &uid, &href, etag_header(&resp)))
}

pub async fn update_event(ctx: &Ctx<'_>, old: &RemoteEvent, ev: &NewEvent) -> Result<RemoteEvent> {
    if old.recurring {
        return fail("Edita los eventos recurrentes CalDAV en su aplicación original");
    }
    let href = href_of(old)?;
    let (_, resp) = dav(ctx, Method::GET, &href, None, None).await?;
    let resp = check(resp).await?;
    let etag = etag_header(&resp).or_else(|| old.etag.clone());
    let original = resp.text().await?;
    let body = ical::patch_calendar(&original, ev);
    let mut rb = ctx
        .req(Method::PUT, href.clone())
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .body(body);
    if let Some(e) = &etag {
        rb = rb.header(header::IF_MATCH, e);
    }
    let resp = rb.send().await?;
    if resp.status() == StatusCode::PRECONDITION_FAILED {
        return fail("El evento cambió en el servidor; sincroniza e inténtalo de nuevo");
    }
    let resp = check(resp).await?;
    Ok(result_event(ev, &old.remote_id, &href, etag_header(&resp)))
}

pub async fn delete_event(ctx: &Ctx<'_>, old: &RemoteEvent) -> Result<()> {
    if old.recurring {
        return fail("Elimina los eventos recurrentes CalDAV en su aplicación original");
    }
    let href = href_of(old)?;
    let resp = ctx.req(Method::DELETE, href).send().await?;
    if resp.status() == StatusCode::NOT_FOUND {
        return Ok(());
    }
    check(resp).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multistatus() {
        let xml = r##"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:x="http://apple.com/ns/ical/">
 <d:response>
  <d:href>/dav/calendars/user/work/</d:href>
  <d:propstat>
   <d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
    <d:displayname>Trabajo</d:displayname>
    <x:calendar-color>#FF0000FF</x:calendar-color>
    <cal:supported-calendar-component-set><cal:comp name="VEVENT"/></cal:supported-calendar-component-set>
    <d:current-user-privilege-set><d:privilege><d:read/></d:privilege></d:current-user-privilege-set>
   </d:prop>
   <d:status>HTTP/1.1 200 OK</d:status>
  </d:propstat>
 </d:response>
 <d:response>
  <d:href>/dav/calendars/user/tasks/</d:href>
  <d:propstat><d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
    <cal:supported-calendar-component-set><cal:comp name="VTODO"/></cal:supported-calendar-component-set>
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"##;
        let rs = parse_multistatus(xml).unwrap();
        let base = url::Url::parse("https://example.com/dav/").unwrap();
        let cals: Vec<_> = rs.iter().filter_map(|r| to_calendar(&base, r)).collect();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].name, "Trabajo");
        assert_eq!(
            cals[0].remote_id,
            "https://example.com/dav/calendars/user/work/"
        );
        assert!(cals[0].read_only);
        assert_eq!(
            normalize_color(cals[0].color.as_deref().unwrap()).unwrap(),
            "#ff0000"
        );
    }
}
