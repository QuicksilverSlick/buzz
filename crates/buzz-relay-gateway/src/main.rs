use buzz_relay_gateway::Config;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Workspace builds unify ring and aws-lc-rs; rustls then needs an explicit provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let directives = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned());
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(buzz_relay_gateway::log_filter(&directives))
        .init();
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            tracing::error!(error = %e, "invalid configuration");
            std::process::exit(1);
        }
    };
    tracing::info!(
        bind = %config.bind_addr,
        relay = config.relay_ws.host_str().unwrap_or_default(),
        link_url = %config.link_url,
        link_pubkey = %config.link_pubkey,
        "relay gateway starting"
    );
    let listener = TcpListener::bind(config.bind_addr).await?;
    let shutdown = CancellationToken::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            signal().await;
            tracing::info!("shutdown signal received");
            shutdown.cancel();
        }
    });
    buzz_relay_gateway::serve(listener, shutdown).await?;
    Ok(())
}

// The binary is PID 1 in the image, which has no default signal handling.
async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
