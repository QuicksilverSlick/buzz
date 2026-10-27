//! Shared harness: the gateway in-process and a bearer signer.
#![allow(dead_code)]

use std::net::SocketAddr;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_relay_gateway::{Config, Limits};
use nostr::{EventBuilder, Keys, Kind, Tag};
use tokio::{net::TcpListener, task::JoinHandle};
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
}
