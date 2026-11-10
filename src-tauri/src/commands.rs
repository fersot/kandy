//! Comandos expuestos al frontend (`invoke` desde Vue).

use chrono::Duration;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::oneshot;

use crate::error::{fail, AppError, Result};
use crate::models::*;
use crate::oauth::{self, zoho_domains};
use crate::providers::{self, caldav, google, ics, microsoft, zoho, Auth, Ctx};
use crate::settings::{BuiltIn, Settings};
use crate::sync;
use crate::AppState;

fn account_view(state: &AppState, a: &Account) -> Result<AccountView> {
    Ok(AccountView {
        id: a.id.clone(),
        provider: a.provider,
        name: a.name.clone(),
        email: a.email.clone(),
        last_sync: a.last_sync.clone(),
        last_error: a.last_error.clone(),
        calendars: state
            .db
            .list_calendars(Some(&a.id))?
            .iter()
            .map(CalendarView::from)
            .collect(),
    })
}

fn parse_range(
    start: &str,
    end: &str,
) -> Result<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    match (parse_any_datetime(start), parse_any_datetime(end)) {
        (Some(s), Some(e)) if e > s => Ok((s, e)),
        _ => fail("Rango de fechas inválido"),
    }
}

// ---------- Cuentas y calendarios ----------

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountView>> {
    state
        .db
        .list_accounts()?
        .iter()
        .map(|a| account_view(&state, a))
        .collect()
}

#[tauri::command]
pub fn set_calendar_visibility(
    state: State<'_, AppState>,
    calendar_id: String,
    visible: bool,
) -> Result<()> {
    state.db.set_calendar_visible(&calendar_id, visible)
}

#[tauri::command]
pub fn set_calendar_color(
    state: State<'_, AppState>,
    calendar_id: String,
    color: String,
) -> Result<()> {
    state.db.set_calendar_color(&calendar_id, &color)
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<()> {
    state.secrets.delete(&account_id).await?;
    state.db.delete_account(&account_id)
}

#[tauri::command]
pub async fn connect_oauth(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: ProviderKind,
    zoho_location: Option<String>,
    account_id: Option<String>,
) -> Result<AccountView> {
    let settings = state.db.settings()?;
    let client = oauth::client_for(provider, &settings, zoho_location.as_deref())?;

    let (tx, rx) = oneshot::channel();
    if let Some(prev) = state
        .oauth_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .replace(tx)
    {
        let _ = prev.send(());
    }
    let result = oauth::authorize(&app, &state.http, &client, rx).await;
    state
        .oauth_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    let auth = result?;
    let token = auth.token.access_token.clone();

    let mut zoho_loc = None;
    let (email, display) = match provider {
        ProviderKind::Google => {
            let u = google::user_info(&state.http, &token).await?;
            (u.email, u.name)
        }
        ProviderKind::Microsoft => {
            let me = microsoft::me(&state.http, &token).await?;
            (me.mail.or(me.user_principal_name), me.display_name)
        }
        ProviderKind::Zoho => {
            let loc = auth
                .callback
                .get("location")
                .cloned()
                .or(zoho_location)
                .unwrap_or_else(|| "us".into());
            let server = auth
                .token
                .accounts_server
                .clone()
                .unwrap_or_else(|| zoho_domains(&loc).0.to_string());
            zoho_loc = Some(loc);
            match zoho::user_info(&state.http, &server, &token).await {
                Ok(u) => (u.email, u.display_name),
                Err(e) => {
                    log::warn!("No se pudo leer el perfil de Zoho: {e}");
                    (None, None)
                }
            }
        }
        _ => return fail("Proveedor sin OAuth"),
    };
    log::info!("Cuenta {} conectada ({:?})", provider.label(), display);

    let existing = match &account_id {
        Some(id) => Some(state.db.get_account(id)?),
        None => match &email {
            Some(e) => state.db.find_account(provider, e)?,
            None => None,
        },
    };
    let account = match existing {
        Some(mut a) => {
            a.email = email.or(a.email);
            if zoho_loc.is_some() {
                a.config.zoho_location = zoho_loc;
            }
            state.db.update_account_identity(&a)?;
            state.db.set_account_status(&a.id, true, None)?;
            a
        }
        None => {
            let a = Account {
                id: uuid::Uuid::new_v4().to_string(),
                provider,
                name: provider.label().to_string(),
                email,
                config: AccountConfig {
                    zoho_location: zoho_loc,
                    ..Default::default()
                },
                last_sync: None,
                last_error: None,
            };
            state.db.insert_account(&a)?;
            a
        }
    };
    state
        .secrets
        .set(&account.id, &serde_json::to_string(&auth.token)?)
        .await?;
    // Primer listado de calendarios para que aparezcan de inmediato.
    let ctx = Ctx {
        http: &state.http,
        account: &account,
        auth: if provider == ProviderKind::Zoho {
            Auth::Zoho(token)
        } else {
            Auth::Bearer(token)
        },
    };
    match providers::list_calendars(&ctx).await {
        Ok(cals) => state.db.upsert_calendars(&account.id, &cals)?,
        Err(e) => log::warn!("No se pudieron listar los calendarios: {e}"),
    }
    account_view(&state, &state.db.get_account(&account.id)?)
}

#[tauri::command]
pub fn cancel_oauth(state: State<'_, AppState>) {
    if let Some(tx) = state
        .oauth_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
    {
        let _ = tx.send(());
    }
}

#[tauri::command]
pub async fn connect_caldav(
    state: State<'_, AppState>,
    name: String,
    url: String,
    username: String,
    password: String,
) -> Result<AccountView> {
    if url.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
        return fail("URL, usuario y contraseña son obligatorios");
    }
    let host = url::Url::parse(&if url.contains("://") {
        url.trim().to_string()
    } else {
        format!("https://{}", url.trim())
    })?
    .host_str()
    .unwrap_or("CalDAV")
    .to_string();
    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        provider: ProviderKind::Caldav,
        name: if name.trim().is_empty() {
            host
        } else {
            name.trim().to_string()
        },
        email: Some(username.trim().to_string()),
        config: AccountConfig {
            url: Some(url.trim().to_string()),
            username: Some(username.trim().to_string()),
            ..Default::default()
        },
        last_sync: None,
        last_error: None,
    };
    let ctx = Ctx {
        http: &state.dav_http,
        account: &account,
        auth: Auth::Basic {
            user: username.trim().to_string(),
            pass: password.clone(),
        },
    };
    // Valida credenciales y descubre calendarios antes de guardar nada.
    let cals = caldav::list_calendars(&ctx).await?;
    state.db.insert_account(&account)?;
    state.secrets.set(&account.id, &password).await?;
    state.db.upsert_calendars(&account.id, &cals)?;
    account_view(&state, &account)
}

#[tauri::command]
pub async fn add_ics(
    state: State<'_, AppState>,
    name: String,
    url: String,
    color: Option<String>,
) -> Result<AccountView> {
    let url = ics::normalize_url(&url)?;
    let text = ics::fetch(&state.http, &url).await?;
    let name = if name.trim().is_empty() {
        crate::ical::calendar_name(&text).unwrap_or_else(|| "Suscripción".into())
    } else {
        name.trim().to_string()
    };
    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        provider: ProviderKind::Ics,
        name,
        email: None,
        config: AccountConfig {
            url: Some(url),
            color: color.as_deref().and_then(normalize_color),
            ..Default::default()
        },
        last_sync: None,
        last_error: None,
    };
    state.db.insert_account(&account)?;
    let ctx = Ctx {
        http: &state.http,
        account: &account,
        auth: Auth::None,
    };
    let cals = ics::list_calendars(&ctx).await?;
    state.db.upsert_calendars(&account.id, &cals)?;
    account_view(&state, &account)
}

// ---------- Sincronización ----------

#[tauri::command]
pub async fn sync_now(
    app: AppHandle,
    state: State<'_, AppState>,
    start: String,
    end: String,
    account_id: Option<String>,
) -> Result<()> {
    let (start, end) = parse_range(&start, &end)?;
    sync::sync_all(&app, &state, start, end, account_id.as_deref()).await
}

// ---------- Eventos ----------

#[tauri::command]
pub fn list_events(
    state: State<'_, AppState>,
    start: String,
    end: String,
) -> Result<Vec<EventView>> {
    let (start, end) = parse_range(&start, &end)?;
    // Margen de un día para eventos de día completo cerca de los bordes.
    let rows = state
        .db
        .list_events(&(start - Duration::days(1)), &(end + Duration::days(1)))?;
    Ok(rows
        .iter()
        .map(|(ev, cal, p)| EventView::build(ev, cal, *p))
        .collect())
}

#[tauri::command]
pub async fn create_event(
    state: State<'_, AppState>,
    calendar_id: String,
    draft: EventDraft,
) -> Result<EventView> {
    let cal = state.db.get_calendar(&calendar_id)?;
    if cal.read_only {
        return fail("Este calendario es de solo lectura");
    }
    let account = state.db.get_account(&cal.account_id)?;
    let new = draft.normalize()?;
    let ctx = sync::ctx_for(&state, &account).await?;
    let remote = providers::create_event(&ctx, &cal, &new).await?;
    let stored = state.db.upsert_event(&cal.id, &remote)?;
    Ok(EventView::build(&stored, &cal, account.provider))
}

#[tauri::command]
pub async fn update_event(
    state: State<'_, AppState>,
    event_id: String,
    draft: EventDraft,
) -> Result<EventView> {
    let stored = state.db.get_event(&event_id)?;
    let cal = state.db.get_calendar(&stored.calendar_id)?;
    if cal.read_only {
        return fail("Este calendario es de solo lectura");
    }
    let account = state.db.get_account(&cal.account_id)?;
    let new = draft.normalize()?;
    let ctx = sync::ctx_for(&state, &account).await?;
    let remote = providers::update_event(&ctx, &cal, &stored.ev, &new).await?;
    if remote.remote_id != stored.ev.remote_id {
        state.db.delete_event(&stored.id)?;
    }
    let saved = state.db.upsert_event(&cal.id, &remote)?;
    Ok(EventView::build(&saved, &cal, account.provider))
}

#[tauri::command]
pub async fn delete_event(state: State<'_, AppState>, event_id: String) -> Result<()> {
    let stored = state.db.get_event(&event_id)?;
    let cal = state.db.get_calendar(&stored.calendar_id)?;
    if cal.read_only {
        return fail("Este calendario es de solo lectura");
    }
    let account = state.db.get_account(&cal.account_id)?;
    let ctx = sync::ctx_for(&state, &account).await?;
    match providers::delete_event(&ctx, &cal, &stored.ev).await {
        Ok(()) => {}
        Err(AppError::AuthExpired) => return Err(AppError::AuthExpired),
        Err(e) => return Err(e),
    }
    state.db.delete_event(&stored.id)
}

// ---------- Ajustes ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPayload {
    settings: Settings,
    built_in: BuiltIn,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<SettingsPayload> {
    Ok(SettingsPayload {
        settings: state.db.settings()?,
        built_in: Settings::built_in(),
    })
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<()> {
    state.db.save_settings(&settings)
}
