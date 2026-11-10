//! Despacho a cada proveedor de calendario. Todos exponen las mismas
//! operaciones sobre modelos normalizados (`RemoteCalendar`, `RemoteEvent`).

pub mod caldav;
pub mod google;
pub mod ics;
pub mod microsoft;
pub mod zoho;

use chrono::{DateTime, Utc};
use reqwest::{Method, RequestBuilder};

use crate::error::{fail, Result};
use crate::models::*;

pub enum Auth {
    Bearer(String),
    Zoho(String),
    Basic { user: String, pass: String },
    None,
}

pub struct Ctx<'a> {
    pub http: &'a reqwest::Client,
    pub account: &'a Account,
    pub auth: Auth,
}

impl Ctx<'_> {
    pub fn req(&self, method: Method, url: impl reqwest::IntoUrl) -> RequestBuilder {
        let rb = self.http.request(method, url);
        match &self.auth {
            Auth::Bearer(t) => rb.bearer_auth(t),
            Auth::Zoho(t) => rb.header("Authorization", format!("Zoho-oauthtoken {t}")),
            Auth::Basic { user, pass } => rb.basic_auth(user, Some(pass)),
            Auth::None => rb,
        }
    }
}

/// Construye una URL añadiendo segmentos de ruta codificados correctamente
/// (los IDs de calendario pueden contener `@`, `#`, `/`, `=`...).
pub fn url_with(base: &str, segments: &[&str]) -> Result<url::Url> {
    let mut u = url::Url::parse(base)?;
    {
        let Ok(mut segs) = u.path_segments_mut() else {
            return fail("URL base inválida");
        };
        segs.pop_if_empty();
        for s in segments {
            segs.push(s);
        }
    }
    Ok(u)
}

pub async fn list_calendars(ctx: &Ctx<'_>) -> Result<Vec<RemoteCalendar>> {
    match ctx.account.provider {
        ProviderKind::Google => google::list_calendars(ctx).await,
        ProviderKind::Microsoft => microsoft::list_calendars(ctx).await,
        ProviderKind::Zoho => zoho::list_calendars(ctx).await,
        ProviderKind::Caldav => caldav::list_calendars(ctx).await,
        ProviderKind::Ics => ics::list_calendars(ctx).await,
    }
}

pub async fn list_events(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<RemoteEvent>> {
    match ctx.account.provider {
        ProviderKind::Google => google::list_events(ctx, cal, start, end).await,
        ProviderKind::Microsoft => microsoft::list_events(ctx, cal, start, end).await,
        ProviderKind::Zoho => zoho::list_events(ctx, cal, start, end).await,
        ProviderKind::Caldav => caldav::list_events(ctx, cal, start, end).await,
        ProviderKind::Ics => ics::list_events(ctx, start, end).await,
    }
}

pub async fn create_event(ctx: &Ctx<'_>, cal: &Calendar, ev: &NewEvent) -> Result<RemoteEvent> {
    match ctx.account.provider {
        ProviderKind::Google => google::create_event(ctx, cal, ev).await,
        ProviderKind::Microsoft => microsoft::create_event(ctx, cal, ev).await,
        ProviderKind::Zoho => zoho::create_event(ctx, cal, ev).await,
        ProviderKind::Caldav => caldav::create_event(ctx, cal, ev).await,
        ProviderKind::Ics => fail("Las suscripciones ICS son de solo lectura"),
    }
}

pub async fn update_event(
    ctx: &Ctx<'_>,
    cal: &Calendar,
    old: &RemoteEvent,
    ev: &NewEvent,
) -> Result<RemoteEvent> {
    match ctx.account.provider {
        ProviderKind::Google => google::update_event(ctx, cal, old, ev).await,
        ProviderKind::Microsoft => microsoft::update_event(ctx, old, ev).await,
        ProviderKind::Zoho => zoho::update_event(ctx, cal, old, ev).await,
        ProviderKind::Caldav => caldav::update_event(ctx, old, ev).await,
        ProviderKind::Ics => fail("Las suscripciones ICS son de solo lectura"),
    }
}

pub async fn delete_event(ctx: &Ctx<'_>, cal: &Calendar, old: &RemoteEvent) -> Result<()> {
    match ctx.account.provider {
        ProviderKind::Google => google::delete_event(ctx, cal, old).await,
        ProviderKind::Microsoft => microsoft::delete_event(ctx, old).await,
        ProviderKind::Zoho => zoho::delete_event(ctx, cal, old).await,
        ProviderKind::Caldav => caldav::delete_event(ctx, old).await,
        ProviderKind::Ics => fail("Las suscripciones ICS son de solo lectura"),
    }
}
