mod common;

use std::time::Duration;

use buzz_relay_gateway::Limits;
use common::{limits, setup, Relay, Worker};
use nostr::{EventBuilder, Keys, RelayUrl};
use serde_json::json;

const AUDIO: &str = "/huddle/6f1c2c1e-4b9a-4c71-9a55-0f1d8c3a2b10/audio";

async fn authenticate(w: &mut Worker, relay: &Relay, sid: &str) {
    let challenge = w.open(sid).await;
    let url = RelayUrl::parse(&relay.ws_url()).unwrap();
    let event = EventBuilder::auth(challenge, url)
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let ok = w.say(sid, json!(["AUTH", event])).await;
    assert_eq!(ok, json!(["OK", event.id, true, ""]));
}

async fn wait_for(what: &str, check: impl Fn() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn nip42_auth_and_subscriptions_pass_through() {
    let (relay, _gw, mut w) = setup(limits()).await;
    authenticate(&mut w, &relay, "s1").await;
    let eose = w.say("s1", json!(["REQ", "sub-1", {"kinds": [9]}])).await;
    assert_eq!(eose, json!(["EOSE", "sub-1"]));
}

#[tokio::test]
async fn closes_reach_the_other_side() {
    let (relay, _gw, mut w) = setup(limits()).await;
    w.open("r").await;
    w.ws_send("r", r#"["CLOSE_WITH",1012]"#).await;
    assert_eq!(
        w.frame().await,
        json!({"type":"ws_closed","sid":"r","by":"relay","code":1012,"reason":"relay restarting"})
    );
    // RFC 6455 5.5.1: the relay gets its close echoed back.
    wait_for("the close echo", || {
        relay.socket_closes().contains(&Some(1012))
    })
    .await;
    w.open("w").await;
    w.send(json!({"type":"ws_close","sid":"w","code":4000,"reason":"done"}))
        .await;
    assert_eq!(
        w.frame().await,
        json!({"type":"ws_closed","sid":"w","by":"worker","code":4000,"reason":"done"})
    );
    wait_for("close 4000", || relay.socket_closes().contains(&Some(4000))).await;
    // A closed socket is gone, and its sid can be opened again.
    w.ws_send("w", "[]").await;
    assert_eq!(w.frame().await["code"], "unknown_socket");
    tokio::time::sleep(Duration::from_millis(100)).await;
    w.open("w").await;
}

#[tokio::test]
async fn closing_the_link_closes_every_relay_socket() {
    let (relay, _gw, mut w) = setup(limits()).await;
    for sid in ["a", "b", "c"] {
        w.open(sid).await;
    }
    assert_eq!(relay.open_sockets(), 3);
    w.ws.close(None).await.unwrap();
    wait_for("every relay socket to close", || relay.open_sockets() == 0).await;
}

#[tokio::test]
async fn socket_limits() {
    let (relay, _gw, mut w) = setup(Limits {
        max_ws_text: 64,
        ..limits()
    })
    .await;
    for i in 0..8 {
        w.open(&format!("s{i}")).await;
    }
    w.send(json!({"type":"ws_open","sid":"s8","path":"/"}))
        .await;
    assert_eq!(
        w.frame().await,
        json!({"type":"error","sid":"s8","code":"busy","message":"too many requests or sockets in flight"})
    );
    w.send(json!({"type":"ws_open","sid":"s0","path":"/"}))
        .await;
    assert_eq!(w.frame().await["code"], "duplicate_id");
    w.ws_send("s0", &"x".repeat(65)).await;
    assert_eq!(w.frame().await["code"], "frame_too_large");
    // The socket stays usable after a refused frame.
    assert_eq!(w.say("s0", json!(["REQ", "q"])).await, json!(["EOSE", "q"]));
    for (path, code) in [(AUDIO, "not_supported"), ("/x", "not_allowed")] {
        w.send(json!({"type":"ws_close","sid":"s7"})).await;
        assert_eq!(w.frame().await["by"], "worker");
        tokio::time::sleep(Duration::from_millis(100)).await;
        w.send(json!({"type":"ws_open","sid":"s7","path":path}))
            .await;
        assert_eq!(w.frame().await["code"], code);
        w.open("s7").await;
    }
    assert_eq!(relay.hits("/x"), 0);
}

#[tokio::test]
async fn refused_and_failed_opens_report_ws_closed() {
    let (relay, _gw, mut w) = setup(limits()).await;
    relay.set(common::Mode::Reply(503, b"busy".to_vec()));
    w.send(json!({"type":"ws_open","sid":"x","path":"/"})).await;
    assert_eq!(
        w.frame().await,
        json!({"type":"ws_closed","sid":"x","by":"gateway","code":null,"reason":"upgrade_refused","http_status":503})
    );
}

#[tokio::test]
async fn relay_side_limits_close_one_socket() {
    let quick = Limits {
        max_relay_frame: 1024,
        relay_silence: Duration::from_millis(500),
        ..limits()
    };
    let (_relay, _gw, mut w) = setup(quick).await;
    w.open("big").await;
    w.ws_send("big", r#"["SEND",1,2048]"#).await;
    let closed = w.frame().await;
    assert_eq!(closed["by"], "gateway");
    assert_eq!(closed["reason"], "frame_too_large");
    w.open("quiet").await;
    w.ws_send("quiet", r#"["SILENT"]"#).await;
    assert_eq!(w.frame().await["reason"], "relay_silent");
}

#[tokio::test]
async fn a_relay_that_stops_reading_closes_that_socket() {
    // A queue that never fills, so only the silence timer can end the socket.
    let stalled = Limits {
        relay_queue: 1024,
        relay_silence: Duration::from_secs(5),
        ..limits()
    };
    let (_relay, _gw, mut w) = setup(stalled).await;
    w.open("stuck").await;
    // The mock relay stops reading and sending; the write stalls once the TCP buffers fill.
    w.ws_send("stuck", r#"["SILENT"]"#).await;
    let big = "x".repeat(256 << 10);
    for _ in 0..100 {
        w.ws_send("stuck", &big).await;
    }
    let closed = w.frame().await;
    assert_eq!(
        (closed["type"].as_str(), closed["reason"].as_str()),
        (Some("ws_closed"), Some("relay_silent")),
        "{closed}"
    );
}

#[tokio::test]
async fn a_worker_that_stops_reading_loses_the_link() {
    let slow = Limits {
        out_frames: 16,
        out_bytes: 64 << 10,
        ..limits()
    };
    let (relay, _gw, mut w) = setup(slow).await;
    w.open("f").await;
    w.ws_send("f", r#"["SEND",4000,4096]"#).await;
    // Stop reading while the relay floods; the queue fills behind the full TCP buffers.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(w.closed().await, Some(4003));
    wait_for("the relay socket to close", || relay.open_sockets() == 0).await;
}

#[tokio::test]
async fn shutdown_closes_sockets_then_the_link() {
    let (relay, gw, mut w) = setup(limits()).await;
    w.open("s").await;
    gw.shutdown.cancel();
    let closed = w.frame().await;
    assert_eq!(closed["by"], "gateway");
    assert_eq!(closed["reason"], "shutdown");
    assert_eq!(w.closed().await, Some(1012));
    wait_for("close 1001", || relay.socket_closes().contains(&Some(1001))).await;
}
