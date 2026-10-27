//! One task per link: `hello`, frame dispatch, pings, rotation, idle and
//! backpressure, and the link's close.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use tokio::{
    sync::{mpsc, mpsc::error::TrySendError, OwnedSemaphorePermit, Semaphore},
    time::{interval_at, sleep_until, timeout, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::{
    frame::{ErrorCode, HelloLimits, Id, Inbound, Out},
    AppState,
};

// Teardown waits this long for the Worker to take the close frame.
const CLOSE_WAIT: Duration = Duration::from_secs(2);
const SHUTDOWN: (u16, &str) = (1012, "gateway restarting");

type Queued = (Message, Option<OwnedSemaphorePermit>);

/// Milliseconds since `since`, for logs and frames.
pub(crate) fn ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn unix_now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// The link's bounded outbound queue. Nothing waits on it: when it is full the
/// Worker is not reading, and the link closes with 4003.
#[derive(Clone)]
pub(crate) struct Outbox {
    tx: mpsc::Sender<Queued>,
    bytes: Arc<Semaphore>,
    overflow: CancellationToken,
}

impl Outbox {
    pub(crate) fn send(&self, frame: &Out) {
        let Ok(text) = serde_json::to_string(frame) else {
            return;
        };
        let permit = u32::try_from(text.len())
            .ok()
            .and_then(|n| self.bytes.clone().try_acquire_many_owned(n).ok());
        let full = match permit {
            Some(permit) => matches!(
                self.tx.try_send((Message::Text(text.into()), Some(permit))),
                Err(TrySendError::Full(_))
            ),
            None => true,
        };
        if full {
            self.overflow.cancel();
        }
    }
}

struct Link {
    state: Arc<AppState>,
    id: String,
    out: Outbox,
}

/// Runs one link until it closes. `_permit` holds its slot under the link cap.
pub(crate) async fn run(socket: WebSocket, state: Arc<AppState>, _permit: OwnedSemaphorePermit) {
    let lim = &state.limits;
    let (sink, mut stream) = socket.split();
    let (tx, rx) = mpsc::channel(lim.out_frames);
    let out = Outbox {
        tx,
        bytes: Arc::new(Semaphore::new(lim.out_bytes)),
        overflow: CancellationToken::new(),
    };
    let mut writer = tokio::spawn(write(sink, rx));
    let now = unix_now().as_secs();
    let rotate_in = lim.link_lifetime.saturating_sub(lim.rotate_before);
    let id = uuid::Uuid::new_v4().to_string();
    out.send(&Out::Hello {
        v: 1,
        link_id: id.clone(),
        expires_at: now + lim.link_lifetime.as_secs(),
        rotate_at: now + rotate_in.as_secs(),
        limits: HelloLimits {
            max_request_body: lim.max_request_body,
            max_response_body: lim.max_response_body,
            max_ws_text: lim.max_ws_text,
            max_http_inflight: lim.max_http_inflight,
            max_sockets: lim.max_sockets,
        },
    });
    tracing::info!(link_id = id, "link opened");
    let opened = Instant::now();
    let mut link = Link {
        state: state.clone(),
        id,
        out,
    };
    let (code, reason) = link
        .serve(&mut stream, opened + lim.link_lifetime, opened + rotate_in)
        .await;
    tracing::info!(
        link_id = link.id,
        code,
        reason,
        ms = ms(opened),
        "link closing"
    );
    let close = Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }));
    let _ = timeout(CLOSE_WAIT, link.out.tx.send((close, None))).await;
    if timeout(CLOSE_WAIT, &mut writer).await.is_err() {
        writer.abort();
    }
}

async fn write(mut sink: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<Queued>) {
    while let Some((msg, _bytes)) = rx.recv().await {
        let close = matches!(msg, Message::Close(_));
        if sink.send(msg).await.is_err() || close {
            break;
        }
    }
}

impl Link {
    async fn serve(
        &mut self,
        stream: &mut futures_util::stream::SplitStream<WebSocket>,
        expires: Instant,
        rotate: Instant,
    ) -> (u16, &'static str) {
        let lim = self.state.limits.clone();
        let mut ping = interval_at(Instant::now() + lim.ping_every, lim.ping_every);
        let mut idle = Instant::now() + lim.idle;
        let mut rotated = false;
        loop {
            tokio::select! {
                msg = stream.next() => {
                    idle = Instant::now() + lim.idle;
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Err(close) = self.on_text(text.as_str()) {
                                return close;
                            }
                        }
                        Some(Ok(Message::Binary(_))) => return (1003, "binary frames are reserved"),
                        Some(Ok(Message::Close(_))) | None => return (1000, "closed by worker"),
                        Some(Ok(_)) => {}
                        // axum's tungstenite is not ours to downcast; its capacity error reads this way.
                        Some(Err(e)) if e.to_string().contains("Space limit exceeded") => return (1009, "frame too large"),
                        Some(Err(_)) => return (1002, "protocol error"),
                    }
                }
                _ = ping.tick() => {
                    let ts = u64::try_from(unix_now().as_millis()).unwrap_or(u64::MAX);
                    self.out.send(&Out::Ping { ts: ts.into() });
                }
                _ = sleep_until(rotate), if !rotated => {
                    rotated = true;
                    let close_at = (unix_now() + expires.saturating_duration_since(Instant::now())).as_secs();
                    self.out.send(&Out::Rotate { close_at });
                }
                _ = sleep_until(expires) => return (4001, "rotate"),
                _ = sleep_until(idle) => return (4002, "idle"),
                _ = self.out.overflow.cancelled() => return (4003, "backpressure"),
                _ = self.state.shutdown.cancelled() => return SHUTDOWN,
            }
        }
    }

    /// Handles one Worker frame. `Err` closes the link with that code.
    fn on_text(&mut self, text: &str) -> Result<(), (u16, &'static str)> {
        let Ok(frame) = Inbound::parse(text) else {
            self.refuse(ErrorCode::BadFrame, None, None);
            return Err((1008, "bad frame"));
        };
        match frame {
            // HTTP forwarding and relay sockets land in later changes.
            Inbound::Http(f) => self.refuse(ErrorCode::NotSupported, Some(f.id), None),
            Inbound::WsOpen { sid, .. } => self.refuse(ErrorCode::NotSupported, None, Some(sid)),
            Inbound::WsSend { sid, .. } | Inbound::WsClose { sid, .. } => {
                self.refuse(ErrorCode::UnknownSocket, None, Some(sid))
            }
            Inbound::Ping { ts } => self.out.send(&Out::Pong { ts }),
            Inbound::Pong => {}
            Inbound::Unknown => self.refuse(ErrorCode::UnknownType, None, None),
        }
        Ok(())
    }

    fn refuse(&self, code: ErrorCode, id: Option<Id>, sid: Option<Id>) {
        tracing::info!(
            link_id = self.id,
            id = id.as_ref().map(Id::as_str),
            sid = sid.as_ref().map(Id::as_str),
            code = ?code,
            "frame refused"
        );
        self.out.send(&Out::error(code, id, sid));
    }
}
