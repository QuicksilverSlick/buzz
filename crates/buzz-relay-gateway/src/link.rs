//! One task per link: `hello`, frame dispatch, pings, rotation, idle and
//! backpressure, and the teardown of everything the link opened.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use tokio::{
    sync::{mpsc, mpsc::error::TrySendError, OwnedSemaphorePermit, Semaphore},
    task::{AbortHandle, JoinSet},
    time::{interval_at, sleep_until, timeout, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::{
    frame::{ErrorCode, HelloLimits, HttpFrame, Id, Inbound, Out},
    http_forward, policy,
    relay_ws::{Cmd, Socket},
    AppState,
};

// Teardown waits this long for the relay sockets, then for the Worker to take the close frame.
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

struct SocketHandle {
    tx: mpsc::Sender<Cmd>,
    overflow: CancellationToken,
}

struct Link {
    state: Arc<AppState>,
    id: String,
    out: Outbox,
    draining: bool,
    http: JoinSet<()>,
    inflight: HashMap<Id, AbortHandle>,
    sockets: HashMap<Id, SocketHandle>,
    socket_tasks: JoinSet<()>,
    sockets_closed: CancellationToken,
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
        draining: false,
        http: JoinSet::new(),
        inflight: HashMap::new(),
        sockets: HashMap::new(),
        socket_tasks: JoinSet::new(),
        sockets_closed: CancellationToken::new(),
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
    // No relay socket outlives its link.
    link.http.abort_all();
    link.sockets_closed.cancel();
    let _ = timeout(CLOSE_WAIT, async {
        while link.socket_tasks.join_next().await.is_some() {}
    })
    .await;
    link.socket_tasks.abort_all();
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
        let mut drain_until = None;
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
                _ = self.state.shutdown.cancelled(), if !self.draining => {
                    self.draining = true;
                    drain_until = Some(Instant::now() + lim.shutdown_grace);
                    if self.http.is_empty() {
                        return SHUTDOWN;
                    }
                }
                _ = sleep_until(drain_until.unwrap_or(expires)), if drain_until.is_some() => return SHUTDOWN,
                Some(_) = self.http.join_next(), if !self.http.is_empty() => {
                    self.inflight.retain(|_, task| !task.is_finished());
                    if self.draining && self.http.is_empty() {
                        return SHUTDOWN;
                    }
                }
                Some(_) = self.socket_tasks.join_next(), if !self.socket_tasks.is_empty() => {
                    self.sockets.retain(|_, socket| !socket.tx.is_closed());
                }
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
            Inbound::Http(f) => {
                let id = f.id.clone();
                if let Err(code) = self.http(f) {
                    self.refuse(code, Some(id), None);
                }
            }
            Inbound::WsOpen { sid, path } => {
                if let Err(code) = self.ws_open(sid.clone(), &path) {
                    self.refuse(code, None, Some(sid));
                }
            }
            Inbound::WsSend { sid, text } => {
                let result = if text.len() > self.state.limits.max_ws_text {
                    Err(ErrorCode::FrameTooLarge)
                } else {
                    self.to_socket(&sid, Cmd::Send(text))
                };
                if let Err(code) = result {
                    self.refuse(code, None, Some(sid));
                }
            }
            Inbound::WsClose { sid, code, reason } => {
                let code = code.unwrap_or(1000);
                let reason = reason.unwrap_or_default();
                // A close frame allows 1000 or an application code, and a 123-byte reason.
                let result =
                    if (code == 1000 || (3000..=4999).contains(&code)) && reason.len() <= 123 {
                        self.to_socket(&sid, Cmd::Close(code, reason))
                    } else {
                        Err(ErrorCode::NotAllowed)
                    };
                if let Err(code) = result {
                    self.refuse(code, None, Some(sid));
                }
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

    fn http(&mut self, f: HttpFrame) -> Result<(), ErrorCode> {
        let lim = &self.state.limits;
        if self.draining {
            return Err(ErrorCode::ShuttingDown);
        }
        if self.inflight.contains_key(&f.id) {
            return Err(ErrorCode::DuplicateId);
        }
        if self.inflight.len() >= lim.max_http_inflight {
            return Err(ErrorCode::Busy);
        }
        let route = policy::http_route(&f.method, &f.path)?;
        let raw_headers = f.headers.as_deref().map(|raw| raw.get());
        let headers = policy::request_headers(route, raw_headers, lim.max_auth_header)?;
        let body = match (route.method() == axum::http::Method::POST, f.body_b64) {
            (true, Some(b64)) => {
                let body = STANDARD.decode(b64).map_err(|_| ErrorCode::BadBase64)?;
                if body.len() > lim.max_request_body {
                    return Err(ErrorCode::BodyTooLarge);
                }
                Some(body)
            }
            (false, None) => None,
            _ => return Err(ErrorCode::NotAllowed),
        };
        let url = self
            .state
            .config
            .relay_http
            .join(route.path())
            .map_err(|_| ErrorCode::NotAllowed)?;
        let (state, out, link_id, id) = (
            self.state.clone(),
            self.out.clone(),
            self.id.clone(),
            f.id.clone(),
        );
        let task = self.http.spawn(async move {
            let started = Instant::now();
            let result = http_forward::forward(
                &state.client,
                route.method(),
                url,
                headers,
                body,
                state.limits.max_response_body,
            )
            .await;
            let (link_id, path, ms) = (link_id.as_str(), route.path(), ms(started));
            out.send(&match result {
                Ok(answer) => {
                    tracing::info!(link_id, id = id.as_str(), path, status = answer.status, ms, "http forwarded");
                    let body_b64 = STANDARD.encode(answer.body);
                    Out::HttpRes { id, status: answer.status, headers: answer.headers, body_b64, ms }
                }
                Err(code) => {
                    tracing::info!(link_id, id = id.as_str(), path, code = ?code, ms, "http failed");
                    Out::error(code, Some(id), None)
                }
            });
        });
        self.inflight.insert(f.id, task);
        Ok(())
    }

    fn ws_open(&mut self, sid: Id, path: &str) -> Result<(), ErrorCode> {
        let lim = &self.state.limits;
        if self.draining {
            return Err(ErrorCode::ShuttingDown);
        }
        if self.sockets.contains_key(&sid) {
            return Err(ErrorCode::DuplicateId);
        }
        if self.sockets.len() >= lim.max_sockets {
            return Err(ErrorCode::Busy);
        }
        let url = self
            .state
            .config
            .relay_ws
            .join(policy::ws_route(path)?)
            .map_err(|_| ErrorCode::NotAllowed)?;
        let (tx, cmds) = mpsc::channel(lim.relay_queue);
        let overflow = CancellationToken::new();
        let socket = Socket {
            link_id: self.id.clone(),
            sid: sid.clone(),
            url,
            out: self.out.clone(),
            cmds,
            overflow: overflow.clone(),
            link_closed: self.sockets_closed.clone(),
            shutdown: self.state.shutdown.clone(),
            connect_timeout: lim.connect_timeout,
            silence: lim.relay_silence,
            max_frame: lim.max_relay_frame,
        };
        self.socket_tasks.spawn(socket.run());
        self.sockets.insert(sid, SocketHandle { tx, overflow });
        Ok(())
    }

    fn to_socket(&self, sid: &Id, cmd: Cmd) -> Result<(), ErrorCode> {
        let socket = self.sockets.get(sid).ok_or(ErrorCode::UnknownSocket)?;
        match socket.tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(TrySendError::Closed(_)) => Err(ErrorCode::UnknownSocket),
            // The relay is not taking frames: close that socket, not the link.
            Err(TrySendError::Full(_)) => {
                socket.overflow.cancel();
                Ok(())
            }
        }
    }
}
