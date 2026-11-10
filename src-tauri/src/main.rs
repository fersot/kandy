// Evita la consola extra en Windows en compilaciones de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;
mod error;
mod ical;
mod models;
mod oauth;
mod providers;
mod secrets;
mod settings;
mod sync;

use std::collections::HashSet;
use std::time::Duration;

use tauri::Manager;
use tokio::sync::oneshot;

pub struct AppState {
    pub db: db::Db,
    pub secrets: secrets::Secrets,
    pub http: reqwest::Client,
    /// Cliente sin redirecciones automáticas (CalDAV las gestiona a mano).
    pub dav_http: reqwest::Client,
    pub oauth_cancel: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    pub sync_lock: tokio::sync::Mutex<()>,
    pub token_lock: tokio::sync::Mutex<()>,
    pub notified: std::sync::Mutex<HashSet<String>>,
}

fn http_client(follow_redirects: bool) -> reqwest::Result<reqwest::Client> {
    let policy = if follow_redirects {
        reqwest::redirect::Policy::limited(10)
    } else {
        reqwest::redirect::Policy::none()
    };
    reqwest::Client::builder()
        .user_agent(concat!("Kandy/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .redirect(policy)
        .build()
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("kandy=info"))
        .init();

    // WebKitGTK + NVIDIA en Wayland puede mostrar la ventana en blanco con el
    // renderizador DMA-BUF; se desactiva solo en ese caso.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        && std::path::Path::new("/proc/driver/nvidia").exists()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            log::info!("Datos en {}", dir.display());
            let state = AppState {
                db: db::Db::open(&dir.join("kandy.db"))?,
                secrets: secrets::Secrets::new(dir.clone()),
                http: http_client(true)?,
                dav_http: http_client(false)?,
                oauth_cancel: Default::default(),
                sync_lock: Default::default(),
                token_lock: Default::default(),
                notified: Default::default(),
            };
            app.manage(state);
            sync::spawn_background(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::set_calendar_visibility,
            commands::set_calendar_color,
            commands::remove_account,
            commands::connect_oauth,
            commands::cancel_oauth,
            commands::connect_caldav,
            commands::add_ics,
            commands::sync_now,
            commands::list_events,
            commands::create_event,
            commands::update_event,
            commands::delete_event,
            commands::get_settings,
            commands::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error al iniciar Kandy");
}
