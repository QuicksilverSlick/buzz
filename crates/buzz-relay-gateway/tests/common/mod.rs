//! Shared harness: the gateway in-process, a mock relay (HTTP side), a mock Worker
//! and a bearer signer.
#![allow(dead_code)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_relay_gateway::{Config, Limits};
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tokio_util::sync::CancellationToken;

pub const LINK_URL: &str = "http://gateway.test/v1/link";

/// A fresh link bearer signed by `keys`, carrying `content`.
pub fn bearer_with(keys: &Keys, content: &str) -> String {
    let nonce = uuid::Uuid::new_v4().to_string();
    let tags = [
        ["u", LINK_URL],
        ["method", "GET"],
        ["nonce", nonce.as_str()],
    ]
    .map(|t| Tag::parse(t).unwrap());
    let event = EventBuilder::new(Kind::HttpAuth, content)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        STANDARD.encode(serde_json::to_string(&event).unwrap())
    )
}

pub fn bearer(keys: &Keys) -> String {
    bearer_with(keys, "")
}

/// Short timers for tests; every other limit stays at its default.
pub fn limits() -> Limits {
    Limits {
        request_timeout: Duration::from_secs(2),
        shutdown_grace: Duration::from_millis(500),
        ..Limits::default()
    }
}

pub struct Gateway {
    pub addr: SocketAddr,
    pub keys: Keys,
    pub shutdown: CancellationToken,
    pub task: JoinHandle<std::io::Result<()>>,
}

impl Gateway {
    pub async fn start(relay_ws: &str, limits: Limits) -> Self {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let config = Config::from_lookup(|name| match name {
            "BUZZ_RELAY_GATEWAY_RELAY_URL" => Some(relay_ws.to_owned()),
            "BUZZ_RELAY_GATEWAY_LINK_URL" => Some(LINK_URL.to_owned()),
            "BUZZ_RELAY_GATEWAY_LINK_PUBKEY" => Some(pubkey.clone()),
            _ => None,
        })
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(buzz_relay_gateway::serve(
            listener,
            config,
            limits,
            shutdown.clone(),
        ));
        Self {
            addr,
            keys,
            shutdown,
            task,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// A Worker on a fresh link, past `hello`.
    pub async fn worker(&self) -> Worker {
        Worker::connect(self, &bearer(&self.keys)).await.unwrap()
    }
}

/// The mock Worker: one link, frames as JSON values.
pub struct Worker {
    pub ws: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    pub hello: Value,
}

pub enum Next {
    Frame(Value),
    Closed(Option<u16>),
}

impl Worker {
    pub async fn connect(
        gw: &Gateway,
        bearer: &str,
    ) -> Result<Self, tokio_tungstenite::tungstenite::Error> {
        let mut request = format!("ws://{}/v1/link", gw.addr).into_client_request()?;
        request
            .headers_mut()
            .insert("authorization", bearer.parse().unwrap());
        let (ws, _) = connect_async(request).await?;
        let mut worker = Self {
            ws,
            hello: Value::Null,
        };
        worker.hello = worker.frame().await;
        assert_eq!(worker.hello["type"], "hello");
        Ok(worker)
    }

    pub async fn send(&mut self, frame: Value) {
        self.ws
            .send(Message::text(frame.to_string()))
            .await
            .unwrap();
    }

    /// The next frame other than a gateway `ping`.
    pub async fn next(&mut self) -> Next {
        loop {
            let msg = timeout(Duration::from_secs(10), self.ws.next())
                .await
                .expect("no frame within 10 s");
            match msg {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] != "ping" {
                        return Next::Frame(frame);
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    return Next::Closed(frame.map(|f| f.code.into()))
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return Next::Closed(None),
            }
        }
    }

    pub async fn frame(&mut self) -> Value {
        match self.next().await {
            Next::Frame(frame) => frame,
            Next::Closed(code) => panic!("link closed: {code:?}"),
        }
    }

    /// Reads until the link closes; returns the close code.
    pub async fn closed(&mut self) -> Option<u16> {
        loop {
            if let Next::Closed(code) = self.next().await {
                return code;
            }
        }
    }

    pub async fn http(
        &mut self,
        id: &str,
        method: &str,
        path: &str,
        headers: Value,
        body: Option<&[u8]>,
    ) -> Value {
        let mut frame =
            json!({ "type": "http", "id": id, "method": method, "path": path, "headers": headers });
        if let Some(body) = body {
            frame["body_b64"] = STANDARD.encode(body).into();
        }
        self.send(frame).await;
        self.frame().await
    }

    pub async fn post(&mut self, id: &str, path: &str, body: &[u8]) -> Value {
        self.http(
            id,
            "POST",
            path,
            json!({ "authorization": "Nostr test" }),
            Some(body),
        )
        .await
    }
}

/// A mock relay, a gateway pointed at it, and a Worker on a fresh link.
pub async fn setup(limits: Limits) -> (Relay, Gateway, Worker) {
    let relay = Relay::start().await;
    let gw = Gateway::start(&relay.ws_url(), limits).await;
    let w = gw.worker().await;
    (relay, gw, w)
}

pub fn body_of(frame: &Value) -> Vec<u8> {
    STANDARD
        .decode(frame["body_b64"].as_str().unwrap())
        .unwrap()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// How the mock relay answers HTTP.
#[derive(Clone, Default)]
pub enum Mode {
    /// Report the hash, length and headers of what arrived.
    #[default]
    Echo,
    /// This status and body.
    Reply(u16, Vec<u8>),
    /// 302 to `/elsewhere`.
    Redirect,
    /// Never answer.
    Stall,
    /// Send part of a body, then drop the connection.
    CloseMid,
}

#[derive(Default)]
pub struct RelayState {
    pub mode: Mutex<Mode>,
    pub hits: Mutex<HashMap<String, usize>>,
    pub url: Mutex<String>,
}

/// The mock relay: HTTP routes on every path.
pub struct Relay {
    pub addr: SocketAddr,
    pub state: Arc<RelayState>,
}

impl Relay {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(RelayState::default());
        *state.url.lock().unwrap() = format!("ws://{addr}");
        let app = Router::new().fallback(handle).with_state(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, state }
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    pub fn set(&self, mode: Mode) {
        *self.state.mode.lock().unwrap() = mode;
    }

    pub fn hits(&self, path: &str) -> usize {
        self.state
            .hits
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}

async fn handle(
    State(s): State<Arc<RelayState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    *s.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
    let mode = s.mode.lock().unwrap().clone();
    match mode {
        Mode::Echo if method == Method::GET => {
            json!({ "name": "mock relay", "accept": headers["accept"].to_str().unwrap() })
                .to_string()
                .into_response()
        }
        Mode::Echo => {
            let seen: serde_json::Map<String, Value> = headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").into()))
                .collect();
            let url = format!(
                "http://{}{path}",
                s.url.lock().unwrap().trim_start_matches("ws://")
            );
            let nip98 = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()?.strip_prefix("Nostr ").map(str::to_owned))
                .and_then(|b64| String::from_utf8(STANDARD.decode(b64).ok()?).ok())
                .is_some_and(|event| {
                    buzz_auth::verify_nip98_event(&event, &url, "POST", Some(&body)).is_ok()
                });
            let echo = json!({ "sha256": sha256_hex(&body), "len": body.len(), "headers": seen, "nip98": nip98 });
            ([("content-type", "application/json")], echo.to_string()).into_response()
        }
        Mode::Reply(status, body) => (
            StatusCode::from_u16(status).unwrap(),
            [("set-cookie", "a=b")],
            body,
        )
            .into_response(),
        Mode::Redirect => (StatusCode::FOUND, [("location", "/elsewhere")]).into_response(),
        Mode::Stall => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            StatusCode::OK.into_response()
        }
        Mode::CloseMid => {
            let chunks = [
                Ok(Bytes::from_static(b"partial")),
                Err(std::io::Error::other("cut")),
            ];
            Body::from_stream(futures_util::stream::iter(chunks)).into_response()
        }
    }
}
