//! Buzz relay gateway: a keyless forwarder between one Worker link and one Buzz relay.
//!
//! The Worker keeps every key and signs every event; the gateway checks the
//! link bearer and forwards bytes. It holds no secret.
#![deny(unsafe_code)]

pub mod config;
pub mod policy;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub use config::Config;

/// Serves `/healthz` on `listener` until `shutdown` is cancelled.
pub async fn serve(listener: TcpListener, shutdown: CancellationToken) -> std::io::Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state(shutdown.clone());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
}

async fn healthz(State(shutdown): State<CancellationToken>) -> Response {
    if shutdown.is_cancelled() {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    } else {
        "ok".into_response()
    }
}
