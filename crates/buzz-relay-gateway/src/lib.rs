//! Buzz relay gateway: a keyless forwarder between one Worker link and one Buzz relay.
//!
//! The Worker keeps every key and signs every event; the gateway checks the
//! link bearer and forwards bytes. It holds no secret.
#![deny(unsafe_code)]

pub mod auth;
pub mod config;
pub mod frame;
pub mod policy;

use std::{
    future::IntoFuture,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use nostr::Timestamp;
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

pub use config::{Config, Limits};

// hyper's graceful wait never ends for a connection that stopped mid-request,
// so once the work is done the open connections get this long.
const LAST_REQUESTS: Duration = Duration::from_secs(2);

struct AppState {
    config: Config,
    limits: Limits,
    replay: auth::ReplayCache,
    links: Arc<Semaphore>,
    shutdown: CancellationToken,
}

/// Serves `/healthz` and `/v1/link` on `listener` until `shutdown` is cancelled.
pub async fn serve(
    listener: TcpListener,
    config: Config,
    limits: Limits,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let state = Arc::new(AppState {
        replay: auth::ReplayCache::new(limits.replay_entries),
        links: Arc::new(Semaphore::new(limits.max_links)),
        shutdown: shutdown.clone(),
        config,
        limits,
    });
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/link", get(link))
        .with_state(state);
    let server =
        axum::serve(listener, app).with_graceful_shutdown(shutdown.clone().cancelled_owned());
    tokio::select! {
        served = server.into_future() => served,
        () = async {
            shutdown.cancelled().await;
            tokio::time::sleep(LAST_REQUESTS).await;
        } => Ok(()),
    }
}

/// The log filter: `directives` in `RUST_LOG` syntax, or `info` if they do not
/// parse. The WebSocket crates stay off at any level, because at trace they
/// print whole frames: bearers, NIP-98 headers, bodies and relay messages.
pub fn log_filter(directives: &str) -> EnvFilter {
    let filter = EnvFilter::try_new(directives).unwrap_or_else(|_| EnvFilter::new("info"));
    // shortcut: a more specific target (tungstenite::protocol=trace) still wins;
    // whoever sets RUST_LOG also picks the image, so pinning harder buys nothing.
    ["tungstenite=off", "tokio_tungstenite=off"]
        .into_iter()
        .filter_map(|d| d.parse().ok())
        .fold(filter, EnvFilter::add_directive)
}

async fn healthz(State(s): State<Arc<AppState>>) -> Response {
    if s.shutdown.is_cancelled() {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    } else {
        "ok".into_response()
    }
}

async fn link(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if s.shutdown.is_cancelled() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Ok(_permit) = s.links.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let claimed = auth::verify_link_bearer(
        &headers,
        &s.config.link_url,
        &s.config.link_pubkey,
        s.limits.max_auth_header,
    )
    .and_then(|bearer| {
        s.replay
            .claim(bearer, Instant::now(), Timestamp::now().as_secs())
    });
    match claimed {
        Err(reject) => {
            tracing::warn!(reason = reject.code(), "link bearer refused");
            if reject == auth::Reject::CacheFull {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            let body = Json(serde_json::json!({ "error": "unauthorized" }));
            (StatusCode::UNAUTHORIZED, body).into_response()
        }
        // The link's frame loop lands in a later change.
        Ok(()) => StatusCode::NOT_IMPLEMENTED.into_response(),
    }
}
