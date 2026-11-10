//! Sincronización con los proveedores, recordatorios y tarea en segundo plano.

use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::error::{AppError, Result};
use crate::models::*;
use crate::oauth::{self, StoredToken};
use crate::providers::{self, Auth, Ctx};
use crate::AppState;

/// Cliente HTTP adecuado: CalDAV necesita gestionar redirecciones a mano.
pub fn http_for<'a>(state: &'a AppState, account: &Account) -> &'a reqwest::Client {
    if account.provider == ProviderKind::Caldav {
        &state.dav_http
    } else {
        &state.http
    }
}

/// Obtiene credenciales válidas para la cuenta, renovando el token si hace falta.
pub async fn auth_for(state: &AppState, account: &Account) -> Result<Auth> {
    match account.provider {
        ProviderKind::Google | ProviderKind::Microsoft | ProviderKind::Zoho => {
            let _guard = state.token_lock.lock().await;
            let raw = state
                .secrets
                .get(&account.id)
                .await?
                .ok_or(AppError::AuthExpired)?;
            let token: StoredToken = serde_json::from_str(&raw)?;
            let settings = state.db.settings()?;
            let client = oauth::client_for(
                account.provider,
                &settings,
                account.config.zoho_location.as_deref(),
            )?;
            let token = match oauth::refresh_if_needed(&state.http, &client, &token).await? {
                Some(new) => {
                    state
                        .secrets
                        .set(&account.id, &serde_json::to_string(&new)?)
                        .await?;
                    new
                }
                None => token,
            };
            Ok(if account.provider == ProviderKind::Zoho {
                Auth::Zoho(token.access_token)
            } else {
                Auth::Bearer(token.access_token)
            })
        }
        ProviderKind::Caldav => {
            let pass = state.secrets.get(&account.id).await?.ok_or_else(|| {
                AppError::Msg(
                    "No se encontró la contraseña CalDAV; vuelve a añadir la cuenta".into(),
                )
            })?;
            Ok(Auth::Basic {
                user: account.config.username.clone().unwrap_or_default(),
                pass,
            })
        }
        ProviderKind::Ics => Ok(Auth::None),
    }
}

pub async fn ctx_for<'a>(state: &'a AppState, account: &'a Account) -> Result<Ctx<'a>> {
    Ok(Ctx {
        http: http_for(state, account),
        account,
        auth: auth_for(state, account).await?,
    })
}

async fn sync_account(
    state: &AppState,
    account: &Account,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<()> {
    let ctx = ctx_for(state, account).await?;
    let remote = providers::list_calendars(&ctx).await?;
    state.db.upsert_calendars(&account.id, &remote)?;
    let mut first_error = None;
    for cal in state
        .db
        .list_calendars(Some(&account.id))?
        .into_iter()
        .filter(|c| c.visible)
    {
        match providers::list_events(&ctx, &cal, start, end).await {
            Ok(events) => state.db.replace_events(&cal.id, &start, &end, &events)?,
            Err(e) => {
                log::warn!("Error sincronizando «{}»: {e}", cal.name);
                if first_error.is_none() {
                    first_error = Some(match e {
                        AppError::AuthExpired => AppError::AuthExpired,
                        other => AppError::Msg(format!("{}: {other}", cal.name)),
                    });
                }
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn emit(app: &AppHandle, account_id: &str, status: &'static str, message: Option<String>) {
    let _ = app.emit(
        "sync-progress",
        SyncProgress {
            account_id: account_id.to_string(),
            status,
            message,
        },
    );
}

/// Sincroniza todas las cuentas (o solo una) en el rango dado, en paralelo.
pub async fn sync_all(
    app: &AppHandle,
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    only: Option<&str>,
) -> Result<()> {
    let _guard = state.sync_lock.lock().await;
    let accounts: Vec<Account> = state
        .db
        .list_accounts()?
        .into_iter()
        .filter(|a| only.is_none_or(|id| a.id == id))
        .collect();
    let jobs = accounts.iter().map(|a| async move {
        emit(app, &a.id, "syncing", None);
        match sync_account(state, a, start, end).await {
            Ok(()) => {
                let _ = state.db.set_account_status(&a.id, true, None);
                emit(app, &a.id, "ok", None);
            }
            Err(e) => {
                let msg = e.to_string();
                log::warn!("Sincronización de {} fallida: {msg}", a.id);
                let _ = state.db.set_account_status(&a.id, false, Some(&msg));
                emit(app, &a.id, "error", Some(msg));
            }
        }
    });
    futures::future::join_all(jobs).await;
    let _ = app.emit("sync-finished", ());
    Ok(())
}

/// Envía notificaciones de escritorio para eventos que empiezan pronto.
fn check_reminders(app: &AppHandle, state: &AppState) {
    let Ok(settings) = state.db.settings() else {
        return;
    };
    if settings.reminder_minutes == 0 {
        return;
    }
    let now = Utc::now();
    let until = now + Duration::minutes(settings.reminder_minutes as i64);
    let Ok(events) = state.db.upcoming_events(&now, &until) else {
        return;
    };
    let mut notified = state.notified.lock().unwrap_or_else(|p| p.into_inner());
    for (ev, cal_name) in events {
        let key = format!("{}@{}", ev.id, ev.ev.start.timestamp());
        if !notified.insert(key) {
            continue;
        }
        let local = ev.ev.start.with_timezone(&chrono::Local);
        let mut body = format!("{} · {}", local.format("%H:%M"), cal_name);
        if let Some(loc) = &ev.ev.location {
            body.push_str(&format!("\n{loc}"));
        }
        if let Err(e) = app
            .notification()
            .builder()
            .title(&ev.ev.title)
            .body(body)
            .show()
        {
            log::warn!("No se pudo mostrar la notificación: {e}");
        }
    }
}

/// Bucle en segundo plano: recordatorios cada 30 s y sincronización periódica.
pub fn spawn_background(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_sync = Instant::now();
        loop {
            tokio::time::sleep(StdDuration::from_secs(30)).await;
            let state = app.state::<AppState>();
            check_reminders(&app, &state);
            let minutes = state
                .db
                .settings()
                .map(|s| s.sync_interval_minutes.max(5))
                .unwrap_or(15);
            if last_sync.elapsed() >= StdDuration::from_secs(minutes as u64 * 60) {
                last_sync = Instant::now();
                let now = Utc::now();
                let _ = sync_all(
                    &app,
                    &state,
                    now - Duration::days(31),
                    now + Duration::days(92),
                    None,
                )
                .await;
            }
        }
    });
}
