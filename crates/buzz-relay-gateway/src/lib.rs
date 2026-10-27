//! Buzz relay gateway: a keyless forwarder between one Worker link and one Buzz relay.
//!
//! The Worker keeps every key and signs every event; the gateway checks the
//! link bearer and forwards bytes. It holds no secret.
#![deny(unsafe_code)]

pub mod auth;
pub mod config;
pub mod frame;
mod link;
pub mod policy;

use std::{
    future::IntoFuture,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{ws::rejection::WebSocketUpgradeRejection, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use nostr::Timestamp;
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
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
    tracker: TaskTracker,
}

/// Serves `/healthz` and `/v1/link` on `listener`. Once `shutdown` is
/// cancelled both answer 503, every link drains and closes, and this returns.
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
        tracker: TaskTracker::new(),
        config,
        limits,
    });
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/link", get(link))
        .with_state(state.clone());
    // Keep accepting (and answering 503) until every link has closed.
    let tracker = state.tracker.clone();
    let drained = async move {
        shutdown.cancelled().await;
        state.tracker.close();
        state.tracker.wait().await;
    };
    let server = axum::serve(listener, app).with_graceful_shutdown(drained);
    tokio::select! {
        served = server.into_future() => served,
        // wait() returns only after `drained` has closed the tracker and every link has ended.
        () = async {
            tracker.wait().await;
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

async fn link(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    if s.shutdown.is_cancelled() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Ok(permit) = s.links.clone().try_acquire_owned() else {
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
    match (claimed, upgrade) {
        (Err(reject), _) => {
            tracing::warn!(reason = reject.code(), "link bearer refused");
            if reject == auth::Reject::CacheFull {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            let body = Json(serde_json::json!({ "error": "unauthorized" }));
            (StatusCode::UNAUTHORIZED, body).into_response()
        }
        (Ok(()), Err(rejection)) => rejection.into_response(),
        (Ok(()), Ok(upgrade)) => {
            let max = s.limits.max_frame;
            let tracked = s.tracker.clone();
            upgrade
                .max_message_size(max)
                .max_frame_size(max)
                .on_upgrade(move |socket| tracked.track_future(link::run(socket, s, permit)))
        }
    }
}
