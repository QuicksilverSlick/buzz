//! Shared harness: the gateway in-process, a mock Worker and a bearer signer.
#![allow(dead_code)]

use std::{net::SocketAddr, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_relay_gateway::{Config, Limits};
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::Value;
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
}
