//! Remote control from a phone: a small web page on the local network to see downloads,
//! pause and resume them, and add links.
//!
//! Off by default. When on, it answers only computers on private networks (home Wi-Fi,
//! Tailscale), and only with the secret token from the QR code; repeated wrong tokens lock
//! everyone out for a while. Plain HTTP on the local network: the token keeps strangers out,
//! not people who can watch the Wi-Fi traffic, and the settings say so.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use klyxdown_engine::{AddRequest, Id};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::AppState;

const PAGE: &str = include_str!("remote.html");
/// Wrong tokens allowed per window before everyone is locked out for it.
const MAX_MISSES: usize = 10;
const MISS_WINDOW: Duration = Duration::from_secs(600);

#[derive(Clone)]
struct Remote {
    app: AppHandle,
    gate: Arc<Gate>,
}

/// Who gets in: private networks with the token, until too many wrong tokens.
struct Gate {
    token: String,
    misses: Mutex<Vec<Instant>>,
}

/// The running server, stopped when dropped.
pub struct Running {
    task: tauri::async_runtime::JoinHandle<()>,
    pub url: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn start(app: AppHandle, port: u16, token: String) -> std::io::Result<Running> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    let remote = Remote {
        app,
        gate: Arc::new(Gate {
            token: token.clone(),
            misses: Mutex::default(),
        }),
    };
    let api = Router::new()
        .route("/api/list", get(list))
        .route("/api/add", post(add))
        .route("/api/{id}/{action}", post(act))
        .layer(middleware::from_fn_with_state(remote.clone(), guard));
    let router = Router::new()
        .route("/", get(page))
        .merge(api)
        .with_state(remote)
        .into_make_service_with_connect_info::<SocketAddr>();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::warn!("remote control stopped: {e}");
        }
    });
    Ok(Running {
        task,
        url: format!("http://{}:{port}/?t={token}", lan_ip()),
    })
}

/// The address phones on the same network reach this computer at. No packet is sent:
/// connecting a UDP socket only picks the outgoing interface.
pub fn lan_ip() -> IpAddr {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|()| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]))
}

/// Home and office networks, link-local, Tailscale's 100.64/10, and this computer.
pub fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return private(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// Same length every time, no early exit on the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

impl Gate {
    /// Checks the token; counts misses and refuses everything once there were too many.
    fn admit(&self, peer: IpAddr, token: Option<&str>) -> Result<(), StatusCode> {
        if !private(peer) {
            return Err(StatusCode::FORBIDDEN);
        }
        let mut misses = self.misses.lock().unwrap_or_else(|e| e.into_inner());
        misses.retain(|t| t.elapsed() < MISS_WINDOW);
        if misses.len() >= MAX_MISSES {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        if token.is_some_and(|t| same(t, &self.token)) {
            return Ok(());
        }
        misses.push(Instant::now());
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn guard(
    State(remote): State<Remote>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let token = headers
        .get("x-klyxdown-token")
        .and_then(|v| v.to_str().ok());
    match remote.gate.admit(peer.ip(), token) {
        Ok(()) => next.run(request).await,
        Err(code) => code.into_response(),
    }
}

#[derive(Deserialize)]
struct PageQuery {
    t: Option<String>,
}

async fn page(
    State(remote): State<Remote>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(q): Query<PageQuery>,
) -> Response {
    match remote.gate.admit(peer.ip(), q.t.as_deref()) {
        Ok(()) => (
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::REFERRER_POLICY, "no-referrer"),
                (
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'",
                ),
            ],
            Html(PAGE),
        )
            .into_response(),
        Err(code) => (code, "Open the link or QR code from KlyxDown's settings.").into_response(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: Id,
    name: String,
    state: klyxdown_engine::State,
    bytes: u64,
    size: Option<u64>,
    speed: u64,
}

async fn list(State(remote): State<Remote>) -> impl IntoResponse {
    let state = remote.app.state::<AppState>();
    let rows: Vec<Row> = state
        .engine
        .list()
        .into_iter()
        .rev()
        .map(|d| Row {
            id: d.id,
            name: d.name,
            state: d.state,
            bytes: d.bytes,
            size: d.size,
            speed: d.speed,
        })
        .collect();
    axum::Json(rows)
}

async fn act(State(remote): State<Remote>, Path((id, action)): Path<(Id, String)>) -> StatusCode {
    let engine = &remote.app.state::<AppState>().engine;
    let done = match action.as_str() {
        "pause" => engine.pause(id),
        "resume" => engine.resume(id),
        _ => return StatusCode::NOT_FOUND,
    };
    if done.is_ok() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

#[derive(Deserialize)]
struct AddBody {
    url: String,
}

async fn add(State(remote): State<Remote>, axum::Json(body): axum::Json<AddBody>) -> Response {
    let url = body.url.trim().to_string();
    let link =
        url.starts_with("https://") || url.starts_with("http://") || url.starts_with("magnet:?");
    if !link || url.len() > 8 * 1024 {
        return (
            StatusCode::BAD_REQUEST,
            "Send an http, https or magnet link.",
        )
            .into_response();
    }
    let state = remote.app.state::<AppState>();
    let (dir, connections, seed) = {
        let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
        (
            s.folder_for(&crate::settings::name_from_url(&url)),
            s.connections,
            s.seed,
        )
    };
    let req = AddRequest {
        url,
        dir,
        connections,
        start: true,
        seed,
        ..AddRequest::default()
    };
    match state.engine.add(req).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
    }
}

/// The remote link as a QR code, in SVG.
pub fn qr_svg(text: &str) -> Option<String> {
    use qrcodegen::{QrCode, QrCodeEcc};
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let size = qr.size();
    let border = 2;
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + border, y + border));
            }
        }
    }
    let full = size + border * 2;
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {full} {full}" shape-rendering="crispEdges"><rect width="100%" height="100%" fill="#fff"/><path d="{path}" fill="#000"/></svg>"##
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_private_networks() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for ok in [
            "127.0.0.1",
            "192.168.1.20",
            "10.0.0.7",
            "172.20.1.1",
            "100.101.102.103",
            "fe80::1",
            "fd7a:115c:a1e0::1",
            "::ffff:192.168.0.2",
            "::1",
        ] {
            assert!(private(ip(ok)), "{ok}");
        }
        for no in [
            "8.8.8.8",
            "100.128.0.1",
            "172.32.0.1",
            "2606:4700::1111",
            "::ffff:1.1.1.1",
        ] {
            assert!(!private(ip(no)), "{no}");
        }
    }

    #[test]
    fn wrong_tokens_lock_the_door() {
        let gate = Gate {
            token: "secret-token".into(),
            misses: Mutex::default(),
        };
        let home: IpAddr = "192.168.1.9".parse().unwrap();
        assert_eq!(gate.admit(home, Some("secret-token")), Ok(()));
        assert_eq!(
            gate.admit("8.8.8.8".parse().unwrap(), Some("secret-token")),
            Err(StatusCode::FORBIDDEN)
        );
        for _ in 0..MAX_MISSES {
            assert_eq!(
                gate.admit(home, Some("guess")),
                Err(StatusCode::UNAUTHORIZED)
            );
        }
        assert_eq!(
            gate.admit(home, Some("secret-token")),
            Err(StatusCode::TOO_MANY_REQUESTS)
        );
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd") && !same("abc", "ab"));
        assert!(
            qr_svg("http://192.168.1.2:47950/?t=0123456789abcdef")
                .unwrap()
                .starts_with("<svg")
        );
    }
}
