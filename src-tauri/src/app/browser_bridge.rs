//! Browser extension bridge bound exclusively to the local loopback interface.
//!
//! Binding to loopback is not enough on its own: any local process can forge an
//! extension `Origin`, any installed extension can call the bridge, and a web
//! page can reach it through DNS rebinding. Every request must therefore carry
//! a loopback `Host` header, and imports must also carry the pairing token that
//! the user copies from the app into the extension popup.

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Mutex, RwLock},
};
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Emitter, Manager};

const BRIDGE_ADDR: &str = "127.0.0.1:17654";
/// `Host` values a genuine request to the bridge can carry (DNS-rebinding defense).
const ALLOWED_HOSTS: [&str; 2] = ["127.0.0.1:17654", "localhost:17654"];
/// Header the extension uses to send the pairing token.
const TOKEN_HEADER: &str = "x-ydl-token";
/// File in the app data directory that keeps the pairing token across restarts.
const TOKEN_FILE: &str = "browser-bridge-token";
const MAX_COOKIES: usize = 20_000;

#[derive(Clone)]
struct BridgeState {
    app: AppHandle,
}

#[derive(Default)]
pub(crate) struct BrowserBridgeState(Mutex<VecDeque<BrowserImportResult>>);

/// Pairing token shared with the browser extension.
pub(crate) struct BrowserBridgeToken(RwLock<String>);

impl BrowserBridgeToken {
    fn get(&self) -> String {
        self.0.read().map(|token| token.clone()).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CookieImport {
    url: Option<String>,
    urls: Option<Vec<String>>,
    mode: Option<String>,
    request_id: String,
    cookies: Vec<BrowserCookie>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserCookie {
    domain: String,
    host_only: bool,
    path: String,
    secure: bool,
    #[allow(dead_code)]
    http_only: bool,
    expiration_date: Option<f64>,
    name: String,
    value: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserImportResult {
    pub url: String,
    pub urls: Vec<String>,
    pub mode: Option<String>,
    pub request_id: String,
    pub cookie_file: Option<String>,
    pub cookie_count: usize,
}

/// 32 random bytes from the OS, hex encoded.
fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("err_generate_token:{e}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn token_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(TOKEN_FILE))
        .map_err(|e| format!("err_app_data_dir:{e}"))
}

async fn store_token(app: &AppHandle, token: &str) -> Result<(), String> {
    let path = token_path(app)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("err_save_token:{e}"))?;
    }
    crate::utils::write_private_file(&path, token.as_bytes())
        .await
        .map_err(|e| format!("err_save_token:{e}"))
}

/// Reads the saved pairing token, creating and saving a new one when missing.
async fn load_or_create_token(app: &AppHandle) -> Result<String, String> {
    if let Ok(saved) = tokio::fs::read_to_string(token_path(app)?).await {
        let saved = saved.trim();
        if saved.len() == 64 && saved.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(saved.to_string());
        }
    }
    let token = generate_token()?;
    store_token(app, &token).await?;
    Ok(token)
}

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Without a token the bridge would have to accept unauthenticated
        // imports, so it stays off instead.
        let token = match load_or_create_token(&app).await {
            Ok(token) => token,
            Err(error) => {
                eprintln!("browser bridge disabled: {error}");
                return;
            }
        };
        app.manage(BrowserBridgeToken(RwLock::new(token)));
        let Ok(listener) = tokio::net::TcpListener::bind(BRIDGE_ADDR).await else {
            eprintln!("browser bridge unavailable: {BRIDGE_ADDR} is already in use");
            return;
        };
        let router = Router::new()
            .route("/v1/health", get(health).options(preflight))
            .route("/v1/import", post(import).options(preflight))
            .with_state(BridgeState { app });
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("browser bridge stopped: {error}");
        }
    });
}

async fn health(State(state): State<BridgeState>, headers: HeaderMap) -> Response {
    if !host_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // `paired` lets the extension tell a missing or wrong token apart from a
    // closed app; the comparison is constant-time like the import check.
    let paired = token_matches(&headers, &current_token(&state.app));
    response(
        &headers,
        StatusCode::OK,
        Json(serde_json::json!({ "app": "ydl-gui", "version": 1, "paired": paired })),
    )
}

async fn preflight(headers: HeaderMap, method: Method) -> Response {
    if method != Method::OPTIONS || !host_allowed(&headers) || extension_origin(&headers).is_none()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    response(&headers, StatusCode::NO_CONTENT, ())
}

async fn import(State(state): State<BridgeState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(status) = authorize(&headers, &current_token(&state.app)) {
        return response(&headers, status, ());
    }
    // Parse only after the request is authorized.
    let payload: CookieImport = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(_) => {
            return response(
                &headers,
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid_payload" })),
            )
        }
    };
    match persist_import(&state.app, payload).await {
        Ok(result) => {
            if let Ok(mut pending) = state.app.state::<BrowserBridgeState>().0.lock() {
                pending.push_back(result.clone());
            }
            let _ = state.app.emit("browser-extension-import-ready", ());
            if let Some(window) = state.app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            response(&headers, StatusCode::OK, Json(result))
        }
        Err(message) => response(
            &headers,
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": message })),
        ),
    }
}

#[tauri::command]
pub(crate) fn get_browser_bridge_token(app: AppHandle) -> Result<String, String> {
    app.try_state::<BrowserBridgeToken>()
        .map(|token| token.get())
        .ok_or_else(|| "err_browser_bridge_unavailable".to_string())
}

/// Replaces the pairing token; the extension must be paired again afterwards.
#[tauri::command]
pub(crate) async fn reset_browser_bridge_token(app: AppHandle) -> Result<String, String> {
    let token = generate_token()?;
    store_token(&app, &token).await?;
    let state = app
        .try_state::<BrowserBridgeToken>()
        .ok_or_else(|| "err_browser_bridge_unavailable".to_string())?;
    *state.0.write().map_err(|e| e.to_string())? = token.clone();
    Ok(token)
}

#[tauri::command]
pub(crate) fn take_browser_extension_imports(
    state: tauri::State<'_, BrowserBridgeState>,
) -> Vec<BrowserImportResult> {
    state
        .0
        .lock()
        .map(|mut pending| pending.drain(..).collect())
        .unwrap_or_default()
}

async fn persist_import(
    app: &AppHandle,
    payload: CookieImport,
) -> Result<BrowserImportResult, String> {
    let mut target_urls = Vec::new();
    if let Some(list) = payload.urls {
        for u in list {
            let trimmed = u.trim().to_string();
            if (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
                && !target_urls.contains(&trimmed)
            {
                target_urls.push(trimmed);
            }
        }
    }
    if let Some(single) = payload.url {
        let trimmed = single.trim().to_string();
        if (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
            && !target_urls.contains(&trimmed)
        {
            target_urls.insert(0, trimmed);
        }
    }
    if target_urls.is_empty() {
        return Err("invalid_url".into());
    }
    let primary_url = target_urls.first().cloned().unwrap_or_default();

    if payload.request_id.len() < 8 || payload.request_id.len() > 128 {
        return Err("invalid_request_id".into());
    }
    if payload.cookies.len() > MAX_COOKIES {
        return Err("too_many_cookies".into());
    }

    let mut lines = Vec::with_capacity(payload.cookies.len() + 2);
    lines.push("# Netscape HTTP Cookie File".to_string());
    lines.push("# Generated locally by YDL GUI Browser Extension".to_string());
    let mut seen = HashSet::new();
    for cookie in payload.cookies {
        validate_field(&cookie.domain)?;
        validate_field(&cookie.path)?;
        validate_field(&cookie.name)?;
        validate_field(&cookie.value)?;
        if cookie.domain.is_empty() || cookie.name.is_empty() || !cookie.path.starts_with('/') {
            return Err("invalid_cookie".into());
        }
        let key = format!("{}\0{}\0{}", cookie.domain, cookie.path, cookie.name);
        if !seen.insert(key) {
            continue;
        }
        let domain = &cookie.domain;
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            domain,
            if cookie.host_only { "FALSE" } else { "TRUE" },
            cookie.path,
            if cookie.secure { "TRUE" } else { "FALSE" },
            cookie
                .expiration_date
                .filter(|v| v.is_finite() && *v > 0.0)
                .map_or(0, |v| v.floor() as i64),
            cookie.name,
            cookie.value
        ));
    }

    let cookie_file = if lines.len() > 2 {
        let path = crate::utils::get_cookie_path(app)?;
        let temporary = path.with_extension("txt.tmp");
        crate::utils::write_private_file(&temporary, format!("{}\n", lines.join("\n")).as_bytes())
            .await
            .map_err(|e| format!("err_save_cookie:{e}"))?;
        if tokio::fs::rename(&temporary, &path).await.is_err() {
            let _ = tokio::fs::remove_file(&path).await;
            tokio::fs::rename(&temporary, &path)
                .await
                .map_err(|e| format!("err_save_cookie:{e}"))?;
        }
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };

    Ok(BrowserImportResult {
        url: primary_url,
        urls: target_urls,
        mode: payload.mode,
        request_id: payload.request_id,
        cookie_file,
        cookie_count: lines.len().saturating_sub(2),
    })
}

fn validate_field(value: &str) -> Result<(), String> {
    if value.contains(['\t', '\r', '\n']) {
        Err("invalid_cookie_field".into())
    } else {
        Ok(())
    }
}

fn current_token(app: &AppHandle) -> String {
    app.try_state::<BrowserBridgeToken>()
        .map(|token| token.get())
        .unwrap_or_default()
}

/// Only requests addressed to the loopback bridge itself are served.
fn host_allowed(headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| ALLOWED_HOSTS.contains(&host.to_ascii_lowercase().as_str()))
}

/// Constant-time comparison of the request's pairing token.
fn token_matches(headers: &HeaderMap, expected: &str) -> bool {
    let Some(provided) = headers.get(TOKEN_HEADER).map(HeaderValue::as_bytes) else {
        return false;
    };
    !expected.is_empty() && bool::from(provided.ct_eq(expected.as_bytes()))
}

/// Checks `Host`, extension `Origin` and the pairing token, in that order.
fn authorize(headers: &HeaderMap, expected_token: &str) -> Result<(), StatusCode> {
    if !host_allowed(headers) || extension_origin(headers).is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    if !token_matches(headers, expected_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

fn extension_origin(headers: &HeaderMap) -> Option<&str> {
    let origin = headers.get("origin")?.to_str().ok()?;
    (origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://"))
        .then_some(origin)
}

fn response<T: IntoResponse>(headers: &HeaderMap, status: StatusCode, body: T) -> Response {
    let mut response = (status, body).into_response();
    if let Some(origin) = extension_origin(headers).and_then(|v| HeaderValue::from_str(v).ok()) {
        response
            .headers_mut()
            .insert("access-control-allow-origin", origin);
        response
            .headers_mut()
            .insert("vary", HeaderValue::from_static("Origin"));
        response.headers_mut().insert(
            "access-control-allow-methods",
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        response.headers_mut().insert(
            "access-control-allow-headers",
            HeaderValue::from_static("Content-Type, X-YDL-Token"),
        );
        response.headers_mut().insert(
            "access-control-allow-private-network",
            HeaderValue::from_static("true"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{authorize, generate_token, host_allowed, token_matches};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};

    const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    fn extension_request(token: &str) -> HeaderMap {
        headers(&[
            ("host", "127.0.0.1:17654"),
            ("origin", "chrome-extension://abcdefghijklmnop"),
            ("x-ydl-token", token),
        ])
    }

    #[test]
    fn accepts_a_paired_extension_request() {
        assert_eq!(authorize(&extension_request(TOKEN), TOKEN), Ok(()));
        let mut localhost = extension_request(TOKEN);
        localhost.insert("host", HeaderValue::from_static("LOCALHOST:17654"));
        assert_eq!(authorize(&localhost, TOKEN), Ok(()));
    }

    #[test]
    fn rejects_missing_or_wrong_tokens() {
        let mut missing = extension_request(TOKEN);
        missing.remove("x-ydl-token");
        assert_eq!(authorize(&missing, TOKEN), Err(StatusCode::UNAUTHORIZED));
        let wrong = extension_request(&TOKEN.replace('0', "1"));
        assert_eq!(authorize(&wrong, TOKEN), Err(StatusCode::UNAUTHORIZED));
        assert_eq!(
            authorize(&extension_request(&TOKEN[..32]), TOKEN),
            Err(StatusCode::UNAUTHORIZED)
        );
        // An unset token must never match an empty header.
        assert!(!token_matches(&extension_request(""), ""));
    }

    #[test]
    fn rejects_rebound_hosts_and_non_extension_origins() {
        for host in [
            "evil.example:17654",
            "127.0.0.1",
            "127.0.0.1:80",
            "localhost",
        ] {
            let mut request = extension_request(TOKEN);
            request.insert("host", HeaderValue::from_str(host).unwrap());
            assert!(!host_allowed(&request), "{host}");
            assert_eq!(authorize(&request, TOKEN), Err(StatusCode::FORBIDDEN));
        }
        let mut web_page = extension_request(TOKEN);
        web_page.insert("origin", HeaderValue::from_static("https://evil.example"));
        assert_eq!(authorize(&web_page, TOKEN), Err(StatusCode::FORBIDDEN));
        let mut no_host = extension_request(TOKEN);
        no_host.remove("host");
        assert_eq!(authorize(&no_host, TOKEN), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn generates_distinct_256_bit_tokens() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}
