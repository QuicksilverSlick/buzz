//! One task per relay socket. It keeps reading the relay, so the relay's pings
//! are answered at once, and it reports exactly one `ws_closed`.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::{
    sync::mpsc,
    time::{sleep_until, timeout, Instant},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::USER_AGENT, HeaderValue},
        protocol::{CloseFrame, WebSocketConfig},
        Error, Message,
    },
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    frame::{Id, Out, Side},
    link::{ms, Outbox},
};

const AUTH_PREFIX: &str = r#"["AUTH","#;
const CLOSE_CAP: Duration = Duration::from_secs(1);

/// A Worker frame for one relay socket.
pub(crate) enum Cmd {
    Send(String),
    Close(u16, String),
}

/// One relay socket. The link keeps the sending half of `cmds`.
pub(crate) struct Socket {
    pub link_id: String,
    pub sid: Id,
    pub url: Url,
    pub out: Outbox,
    pub cmds: mpsc::Receiver<Cmd>,
    /// Cancelled when the link could not queue a Worker frame for this socket.
    pub overflow: CancellationToken,
    /// Cancelled when the link ends.
    pub link_closed: CancellationToken,
    pub shutdown: CancellationToken,
    pub connect_timeout: Duration,
    pub silence: Duration,
    pub max_frame: usize,
}

struct Closed {
    by: Side,
    code: Option<u16>,
    reason: String,
    http_status: Option<u16>,
}

fn by_gateway(code: Option<u16>, reason: &str) -> Closed {
    Closed {
        by: Side::Gateway,
        code,
        reason: reason.to_owned(),
        http_status: None,
    }
}

impl Socket {
    pub(crate) async fn run(mut self) {
        let opened = Instant::now();
        let closed = self.serve(opened).await;
        // Relay and Worker close reasons are frame content, so only gateway reasons are logged.
        let reason = (closed.by == Side::Gateway).then_some(closed.reason.as_str());
        tracing::info!(
            link_id = self.link_id,
            sid = self.sid.as_str(),
            by = ?closed.by,
            code = closed.code,
            reason,
            http_status = closed.http_status,
            ms = ms(opened),
            "relay socket closed"
        );
        self.out.send(&Out::WsClosed {
            sid: self.sid,
            by: closed.by,
            code: closed.code,
            reason: closed.reason,
            http_status: closed.http_status,
        });
    }

    fn end(&self) -> (u16, &'static str) {
        if self.shutdown.is_cancelled() {
            (1001, "shutdown")
        } else {
            (1000, "link_closed")
        }
    }

    async fn serve(&mut self, opened: Instant) -> Closed {
        let Ok(mut request) = self.url.as_str().into_client_request() else {
            return by_gateway(None, "connect_failed");
        };
        let agent = concat!("buzz-relay-gateway/", env!("CARGO_PKG_VERSION"));
        request
            .headers_mut()
            .insert(USER_AGENT, HeaderValue::from_static(agent));
        let config = WebSocketConfig::default()
            .max_message_size(Some(self.max_frame))
            .max_frame_size(Some(self.max_frame));
        let connecting = connect_async_with_config(request, Some(config), false);
        let connect = timeout(self.connect_timeout, connecting);
        let mut ws = tokio::select! {
            result = connect => match result {
                Err(_) => return by_gateway(None, "connect_timeout"),
                Ok(Err(Error::Http(response))) => {
                    let status = Some(response.status().as_u16());
                    return Closed { http_status: status, ..by_gateway(None, "upgrade_refused") };
                }
                Ok(Err(_)) => return by_gateway(None, "connect_failed"),
                Ok(Ok((ws, _))) => ws,
            },
            _ = self.link_closed.cancelled() => return by_gateway(None, self.end().1),
        };
        self.out.send(&Out::WsOpened {
            sid: self.sid.clone(),
            ms: ms(opened),
        });
        let (link_id, sid) = (self.link_id.as_str(), self.sid.as_str());
        let mut challenge = None;
        let mut silent_at = Instant::now() + self.silence;
        let (frame, closed) = loop {
            let (code, reason) = tokio::select! {
                msg = ws.next() => {
                    silent_at = Instant::now() + self.silence;
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if text.starts_with(AUTH_PREFIX) {
                                tracing::info!(link_id, sid, auth_challenge_ms = ms(opened), "relay sent AUTH");
                                challenge = Some(Instant::now());
                            }
                            self.out.send(&Out::WsMsg { sid: self.sid.clone(), text: text.as_str().to_owned() });
                            continue;
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let (code, reason) = frame.map_or((None, String::new()), |f| (Some(f.code.into()), f.reason.as_str().to_owned()));
                            return Closed { by: Side::Relay, code, reason, http_status: None };
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(Error::Capacity(_))) => (1009, "frame_too_large"),
                        Some(Err(_)) | None => {
                            return Closed { by: Side::Relay, code: None, reason: "connection_lost".into(), http_status: None };
                        }
                    }
                }
                cmd = self.cmds.recv() => match cmd {
                    Some(Cmd::Send(text)) => {
                        if let Some(at) = challenge.filter(|_| text.starts_with(AUTH_PREFIX)) {
                            tracing::info!(link_id, sid, auth_reply_ms = ms(at), "worker answered AUTH");
                            challenge = None;
                        }
                        if ws.send(Message::Text(text.into())).await.is_err() {
                            return Closed { by: Side::Relay, code: None, reason: "connection_lost".into(), http_status: None };
                        }
                        continue;
                    }
                    Some(Cmd::Close(code, reason)) => {
                        let frame = CloseFrame { code: code.into(), reason: reason.clone().into() };
                        break (frame, Closed { by: Side::Worker, code: Some(code), reason, http_status: None });
                    }
                    None => self.end(),
                },
                _ = self.overflow.cancelled() => (1000, "backpressure"),
                _ = self.link_closed.cancelled() => self.end(),
                _ = sleep_until(silent_at) => (1001, "relay_silent"),
            };
            break (
                CloseFrame {
                    code: code.into(),
                    reason: reason.into(),
                },
                by_gateway(Some(code), reason),
            );
        };
        // The relay's close reply ends the stream; capped so teardown stays bounded.
        let _ = timeout(CLOSE_CAP, async {
            if ws.close(Some(frame)).await.is_ok() {
                while let Some(Ok(_)) = ws.next().await {}
            }
        })
        .await;
        closed
    }
}
