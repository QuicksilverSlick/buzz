//! Its own test binary, because it installs the global subscriber the way
//! main.rs does: fmt().init() also bridges `log` records, which is how
//! tungstenite and reqwest log.
mod common;

use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use common::{bearer_with, limits, Gateway, Relay, Worker};
use nostr::{EventBuilder, Keys, RelayUrl};
use serde_json::json;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn logs_never_carry_bearers_bodies_or_frames() {
    const MARK: &str = "MARK7f3a";
    let capture = Capture::default();
    let writer = capture.clone();
    // RUST_LOG=trace: the most anyone can turn on.
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(buzz_relay_gateway::log_filter("trace"))
        .with_writer(move || writer.clone())
        .init();

    let relay = Relay::start().await;
    let gw = Gateway::start(&relay.ws_url(), limits()).await;
    let mut w = Worker::connect(&gw, &bearer_with(&gw.keys, MARK))
        .await
        .unwrap();
    let res = w
        .http(
            "h",
            "POST",
            "/query",
            json!({"authorization": format!("Nostr {MARK}")}),
            Some(MARK.as_bytes()),
        )
        .await;
    assert_eq!(res["status"], 200);
    let challenge = w.open("s").await;
    let url = RelayUrl::parse(&relay.ws_url()).unwrap();
    let event = EventBuilder::auth(challenge, url)
        .sign_with_keys(&Keys::generate())
        .unwrap();
    w.say("s", json!(["AUTH", event, MARK])).await;
    w.say("s", json!(["REQ", MARK])).await;
    w.send(json!({"type":"ws_close","sid":"s","reason":MARK}))
        .await;
    w.frame().await;

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("auth_reply_ms"), "the AUTH timing was logged");
    // reqwest logs through `log`, so this line shows the bridge is live.
    assert!(logs.contains("starting new connection"), "no `log` records");
    assert!(!logs.contains(MARK), "marker leaked into logs");
    assert!(!logs.contains("Nostr "), "a bearer leaked into logs");
}
