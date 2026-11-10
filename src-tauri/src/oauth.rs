//! OAuth 2.0 para apps nativas (RFC 8252): Authorization Code + PKCE con
//! redirección a un servidor HTTP efímero en loopback. El navegador se abre con
//! `tauri-plugin-opener`, que dentro de Flatpak pasa por el portal OpenURI.

use std::collections::HashMap;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use crate::error::{fail, AppError, Result};
use crate::models::ProviderKind;
use crate::settings::Settings;

/// Configuración OAuth de un proveedor concreto.
#[derive(Clone, Debug)]
pub struct OAuthClient {
    pub provider: ProviderKind,
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scopes: String,
    pub extra_auth_params: Vec<(&'static str, &'static str)>,
    /// `127.0.0.1` o `localhost` según lo que admita el proveedor.
    pub redirect_host: &'static str,
    pub redirect_path: &'static str,
    /// 0 = puerto aleatorio.
    pub port: u16,
}

/// Mapa de "location" de Zoho a sus dominios (cuentas y API de Calendar).
pub fn zoho_domains(location: &str) -> (&'static str, &'static str) {
    match location {
        "eu" => ("https://accounts.zoho.eu", "https://calendar.zoho.eu"),
        "in" => ("https://accounts.zoho.in", "https://calendar.zoho.in"),
        "au" => (
            "https://accounts.zoho.com.au",
            "https://calendar.zoho.com.au",
        ),
        "jp" => ("https://accounts.zoho.jp", "https://calendar.zoho.jp"),
        "uk" => ("https://accounts.zoho.uk", "https://calendar.zoho.uk"),
        "ca" => (
            "https://accounts.zohocloud.ca",
            "https://calendar.zohocloud.ca",
        ),
        "sa" => ("https://accounts.zoho.sa", "https://calendar.zoho.sa"),
        "ae" => ("https://accounts.zoho.ae", "https://calendar.zoho.ae"),
        "sg" => ("https://accounts.zoho.sg", "https://calendar.zoho.sg"),
        "cn" => (
            "https://accounts.zoho.com.cn",
            "https://calendar.zoho.com.cn",
        ),
        _ => ("https://accounts.zoho.com", "https://calendar.zoho.com"),
    }
}

pub fn client_for(
    provider: ProviderKind,
    settings: &Settings,
    zoho_location: Option<&str>,
) -> Result<OAuthClient> {
    let need = |id: String, name: &str| -> Result<String> {
        if id.is_empty() {
            fail(format!(
                "Falta el Client ID de {name}. Configúralo en Ajustes → Credenciales OAuth."
            ))
        } else {
            Ok(id)
        }
    };
    let opt = |s: String| if s.is_empty() { None } else { Some(s) };
    Ok(match provider {
        ProviderKind::Google => OAuthClient {
            provider,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            client_id: need(settings.google_client_id(), "Google")?,
            client_secret: opt(settings.google_client_secret()),
            scopes: "openid email profile https://www.googleapis.com/auth/calendar".into(),
            extra_auth_params: vec![("access_type", "offline"), ("prompt", "consent")],
            redirect_host: "127.0.0.1",
            redirect_path: "",
            port: 0,
        },
        ProviderKind::Microsoft => {
            let tenant = settings.microsoft_tenant();
            OAuthClient {
                provider,
                auth_url: format!(
                    "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
                ),
                token_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
                client_id: need(settings.microsoft_client_id(), "Microsoft")?,
                client_secret: None,
                scopes: "openid profile email offline_access User.Read Calendars.ReadWrite".into(),
                extra_auth_params: vec![("prompt", "select_account")],
                // Azure ignora el puerto en redirecciones a http://localhost.
                redirect_host: "localhost",
                redirect_path: "",
                port: 0,
            }
        }
        ProviderKind::Zoho => {
            let (accounts, _) = zoho_domains(zoho_location.unwrap_or("us"));
            OAuthClient {
                provider,
                auth_url: format!("{accounts}/oauth/v2/auth"),
                token_url: format!("{accounts}/oauth/v2/token"),
                client_id: need(settings.zoho_client_id(), "Zoho")?,
                client_secret: opt(settings.zoho_client_secret()),
                scopes: "ZohoCalendar.calendar.ALL,ZohoCalendar.event.ALL,AaaServer.profile.READ"
                    .into(),
                extra_auth_params: vec![("access_type", "offline"), ("prompt", "consent")],
                // Zoho exige que la URI coincida exactamente con la registrada.
                redirect_host: "localhost",
                redirect_path: "/callback",
                port: settings.zoho_port(),
            }
        }
        _ => return fail("Este proveedor no usa OAuth"),
    })
}

/// Token guardado en el llavero (JSON).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Instante (epoch, segundos) en que caduca el access token.
    pub expires_at: i64,
    /// Zoho: servidor de cuentas del centro de datos del usuario.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts_server: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<serde_json::Value>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Resultado del flujo de autorización.
pub struct AuthResult {
    pub token: StoredToken,
    /// Parámetros extra devueltos en la redirección (p. ej. `location` de Zoho).
    pub callback: HashMap<String, String>,
}

fn random_b64(len: usize) -> Result<String> {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).map_err(|e| AppError::Msg(format!("Sin entropía: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// Ejecuta el flujo completo: abre el navegador, espera la redirección y
/// canjea el código por tokens. `cancel` permite abortarlo desde la UI.
pub async fn authorize(
    app: &AppHandle,
    http: &reqwest::Client,
    client: &OAuthClient,
    cancel: oneshot::Receiver<()>,
) -> Result<AuthResult> {
    let verifier = random_b64(64)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64(24)?;

    // Servidor loopback (IPv4 y, si el host es "localhost", también IPv6).
    let v4 = TcpListener::bind(("127.0.0.1", client.port))
        .await
        .map_err(|e| {
            AppError::Msg(format!(
                "No se pudo abrir el puerto local {} para recibir la autorización: {e}",
                client.port
            ))
        })?;
    let port = v4.local_addr()?.port();
    let mut listeners = vec![v4];
    if client.redirect_host == "localhost" {
        if let Ok(v6) = TcpListener::bind(("::1", port)).await {
            listeners.push(v6);
        }
    }
    let redirect_uri = format!(
        "http://{}:{}{}",
        client.redirect_host, port, client.redirect_path
    );

    let mut auth_url = url::Url::parse(&client.auth_url)?;
    {
        let mut q = auth_url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", &client.scopes)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        for (k, v) in &client.extra_auth_params {
            q.append_pair(k, v);
        }
    }

    app.opener()
        .open_url(auth_url.as_str(), None::<&str>)
        .map_err(|e| AppError::Msg(format!("No se pudo abrir el navegador: {e}")))?;

    let params = wait_for_callback(listeners, &state, cancel).await?;
    let code = params
        .get("code")
        .cloned()
        .ok_or_else(|| AppError::Msg("La redirección no incluyó el código".into()))?;

    // Zoho multi-DC: el canje se hace contra el servidor de cuentas del usuario.
    let mut token_url = client.token_url.clone();
    let mut accounts_server = None;
    if client.provider == ProviderKind::Zoho {
        let server = params
            .get("accounts-server")
            .cloned()
            .or_else(|| {
                params
                    .get("location")
                    .map(|l| zoho_domains(l).0.to_string())
            })
            .unwrap_or_else(|| {
                client
                    .token_url
                    .trim_end_matches("/oauth/v2/token")
                    .to_string()
            });
        if !server.starts_with("https://") || !server.contains(".zoho") {
            return fail("Servidor de cuentas Zoho inesperado en la redirección");
        }
        token_url = format!("{}/oauth/v2/token", server.trim_end_matches('/'));
        accounts_server = Some(server);
    }

    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "authorization_code".into()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client.client_id.clone()),
        ("code_verifier", verifier),
    ];
    if let Some(secret) = &client.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    if client.provider == ProviderKind::Microsoft {
        form.push(("scope", client.scopes.clone()));
    }
    let tr = post_token(http, &token_url, &form).await?;
    let token = StoredToken {
        access_token: tr.access_token.unwrap_or_default(),
        refresh_token: tr.refresh_token,
        expires_at: expiry(&tr.expires_in),
        accounts_server,
    };
    if token.refresh_token.is_none() {
        log::warn!("El proveedor no devolvió refresh_token; habrá que reconectar al caducar");
    }
    Ok(AuthResult {
        token,
        callback: params,
    })
}

fn expiry(v: &Option<serde_json::Value>) -> i64 {
    let secs = match v {
        Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(3600),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(3600),
        _ => 3600,
    };
    Utc::now().timestamp() + secs
}

async fn post_token(
    http: &reqwest::Client,
    url: &str,
    form: &[(&str, String)],
) -> Result<TokenResponse> {
    let resp = http.post(url).form(form).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    let tr: TokenResponse = serde_json::from_str(&body).map_err(|_| {
        AppError::Msg(format!(
            "Respuesta inesperada del servidor de tokens ({status}): {}",
            body.chars().take(300).collect::<String>()
        ))
    })?;
    if let Some(err) = &tr.error {
        if err == "invalid_grant" || err == "invalid_code" {
            return Err(AppError::AuthExpired);
        }
        return fail(format!(
            "Error de autorización: {err} {}",
            tr.error_description.clone().unwrap_or_default()
        ));
    }
    if tr.access_token.is_none() {
        return fail(format!("El servidor no devolvió access_token ({status})"));
    }
    Ok(tr)
}

/// Renueva el access token si caduca en menos de 2 minutos.
/// Devuelve `Some(token)` si cambió (para persistirlo).
pub async fn refresh_if_needed(
    http: &reqwest::Client,
    client: &OAuthClient,
    token: &StoredToken,
) -> Result<Option<StoredToken>> {
    if token.expires_at - 120 > Utc::now().timestamp() {
        return Ok(None);
    }
    let Some(refresh) = token.refresh_token.clone() else {
        return Err(AppError::AuthExpired);
    };
    let token_url = match (&client.provider, &token.accounts_server) {
        (ProviderKind::Zoho, Some(server)) => {
            format!("{}/oauth/v2/token", server.trim_end_matches('/'))
        }
        _ => client.token_url.clone(),
    };
    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh.clone()),
        ("client_id", client.client_id.clone()),
    ];
    if let Some(secret) = &client.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    if client.provider == ProviderKind::Microsoft {
        form.push(("scope", client.scopes.clone()));
    }
    let tr = post_token(http, &token_url, &form).await?;
    Ok(Some(StoredToken {
        access_token: tr.access_token.unwrap_or_default(),
        // Microsoft rota el refresh token; Google y Zoho mantienen el anterior.
        refresh_token: tr.refresh_token.or(Some(refresh)),
        expires_at: expiry(&tr.expires_in),
        accounts_server: token.accounts_server.clone(),
    }))
}

// ---------- Servidor loopback ----------

const PAGE_OK: &str = "<!doctype html><html lang=es><meta charset=utf-8><title>Kandy</title>\
<body style=\"font-family:system-ui,sans-serif;display:grid;place-items:center;height:90vh;\
background:#f6f7fb;color:#1f2430\"><div style=\"text-align:center\"><div style=\"font-size:48px\">✅</div>\
<h1 style=\"font-weight:600\">Cuenta conectada</h1><p>Ya puedes cerrar esta pestaña y volver a Kandy.</p>\
</div></body></html>";

fn page_error(msg: &str) -> String {
    let safe = msg
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><html lang=es><meta charset=utf-8><title>Kandy</title>\
<body style=\"font-family:system-ui,sans-serif;display:grid;place-items:center;height:90vh\">\
<div style=\"text-align:center\"><div style=\"font-size:48px\">⚠️</div><h1>No se pudo conectar</h1>\
<p>{safe}</p><p>Vuelve a Kandy e inténtalo de nuevo.</p></div></body></html>"
    )
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    if method != "GET" {
        return None;
    }
    parts.next().map(str::to_string)
}

async fn wait_for_callback(
    listeners: Vec<TcpListener>,
    expected_state: &str,
    mut cancel: oneshot::Receiver<()>,
) -> Result<HashMap<String, String>> {
    let (tx, mut rx) = mpsc::channel::<TcpStream>(8);
    let mut tasks = Vec::new();
    for l in listeners {
        let tx = tx.clone();
        tasks.push(tokio::spawn(async move {
            while let Ok((s, _)) = l.accept().await {
                if tx.send(s).await.is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);

    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);

    let result = loop {
        tokio::select! {
            _ = &mut cancel => break Err(AppError::Cancelled),
            _ = &mut deadline => break fail("Se agotó el tiempo de espera (5 min) para la autorización"),
            stream = rx.recv() => {
                let Some(mut stream) = stream else {
                    break fail("El servidor local de autorización se cerró");
                };
                let Some(target) = read_request_target(&mut stream).await else {
                    respond(&mut stream, "400 Bad Request", "").await;
                    continue;
                };
                let Ok(url) = url::Url::parse(&format!("http://localhost{target}")) else {
                    respond(&mut stream, "400 Bad Request", "").await;
                    continue;
                };
                let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
                if !params.contains_key("code") && !params.contains_key("error") {
                    // favicon.ico u otras peticiones del navegador.
                    respond(&mut stream, "404 Not Found", "").await;
                    continue;
                }
                if params.get("state").map(String::as_str) != Some(expected_state) {
                    respond(&mut stream, "400 Bad Request", &page_error("Parámetro state inválido.")).await;
                    break fail("Respuesta de autorización con state inválido");
                }
                if let Some(err) = params.get("error") {
                    let desc = params.get("error_description").cloned().unwrap_or_else(|| err.clone());
                    respond(&mut stream, "200 OK", &page_error(&desc)).await;
                    break fail(format!("Autorización denegada: {desc}"));
                }
                respond(&mut stream, "200 OK", PAGE_OK).await;
                break Ok(params);
            }
        }
    };
    for t in tasks {
        t.abort();
    }
    result
}
