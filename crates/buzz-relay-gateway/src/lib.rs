//! Buzz relay gateway: a keyless forwarder between one Worker link and one Buzz relay.
//!
//! The Worker keeps every key and signs every event; the gateway checks the
//! link bearer and forwards bytes. It holds no secret.
#![deny(unsafe_code)]

pub mod config;
pub mod policy;

use std::{future::IntoFuture, time::Duration};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

pub use config::Config;

// hyper's graceful wait never ends for a connection that stopped mid-request,
// so once the work is done the open connections get this long.
const LAST_REQUESTS: Duration = Duration::from_secs(2);

/// Serves `/healthz` on `listener` until `shutdown` is cancelled.
pub async fn serve(listener: TcpListener, shutdown: CancellationToken) -> std::io::Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state(shutdown.clone());
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

async fn healthz(State(shutdown): State<CancellationToken>) -> Response {
    if shutdown.is_cancelled() {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    } else {
        "ok".into_response()
    }
}
