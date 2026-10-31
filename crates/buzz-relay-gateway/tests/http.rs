mod common;

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_relay_gateway::Limits;
use common::{body_of, limits, setup, sha256_hex, Gateway, Mode};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};

/// A POST /query frame, for requests whose answer the test reads later.
fn query(id: &str) -> Value {
    json!({"type":"http","id":id,"method":"POST","path":"/query","headers":{"authorization":"x"},"body_b64":""})
}

/// Deterministic bytes that are not valid UTF-8.
fn noise(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9e37_79b9;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8 | 0x80
        })
        .collect()
}

#[tokio::test]
async fn bodies_and_headers_arrive_untouched() {
    let (relay, _gw, mut w) = setup(limits()).await;
    let odd_json = b" [ {\"kinds\" :[0] ,\n\t\"limit\":1 } ]  ";
    for (i, body) in [noise(64 << 10), odd_json.to_vec()].iter().enumerate() {
        let res = w.post(&format!("b{i}"), "/events", body).await;
        assert_eq!(
            (res["type"].as_str(), res["status"].as_u64()),
            (Some("http_res"), Some(200))
        );
        let echo: Value = serde_json::from_slice(&body_of(&res)).unwrap();
        assert_eq!(echo["sha256"], sha256_hex(body));
        assert_eq!(echo["len"], body.len());
        let seen = echo["headers"].as_object().unwrap();
        assert_eq!(seen["host"], relay.addr.to_string());
        assert_eq!(
            seen["user-agent"],
            concat!("buzz-relay-gateway/", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(seen["authorization"], "Nostr test");
        for name in seen.keys() {
            let banned = name.starts_with("x-forwarded")
                || [
                    "forwarded",
                    "x-real-ip",
                    "x-pubkey",
                    "accept-encoding",
                    "cookie",
                    "origin",
                ]
                .contains(&name.as_str());
            assert!(!banned, "{name} reached the relay");
        }
        assert_eq!(res["headers"], json!({"content-type": "application/json"}));
    }
    assert_eq!(relay.hits("/events"), 2);
}

#[tokio::test]
async fn response_bytes_come_back_identical() {
    let (relay, _gw, mut w) = setup(limits()).await;
    let bytes = noise(100_000);
    relay.set(Mode::Reply(200, bytes.clone()));
    let res = w.post("r", "/query", b"[]").await;
    assert_eq!(body_of(&res), bytes);
    // Only content-type and retry-after come back; set-cookie stays at the gateway.
    assert_eq!(
        res["headers"],
        json!({"content-type": "application/octet-stream"})
    );
}

#[tokio::test]
async fn a_signed_nip98_request_still_verifies_at_the_relay() {
    let (relay, _gw, mut w) = setup(limits()).await;
    let keys = Keys::generate();
    let body = br#"[{"kinds":[9],"limit":5}]"#;
    let url = format!("http://{}/query", relay.addr);
    let tags = [
        ["u", url.as_str()],
        ["method", "POST"],
        ["payload", &sha256_hex(body)],
    ]
    .map(|t| Tag::parse(t).unwrap());
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(&keys)
        .unwrap();
    let auth = format!(
        "Nostr {}",
        STANDARD.encode(serde_json::to_string(&event).unwrap())
    );
    let headers = json!({"authorization": auth, "content-type": "application/json", "accept": "application/json"});
    let res = w.http("n", "POST", "/query", headers, Some(body)).await;
    let echo: Value = serde_json::from_slice(&body_of(&res)).unwrap();
    assert_eq!(echo["nip98"], true);
}

#[tokio::test]
async fn relay_answers_pass_through_once() {
    let (relay, _gw, mut w) = setup(limits()).await;
    let limited = br#"{"error":"rate-limited: quota exceeded; retry in 12s"}"#.to_vec();
    relay.set(Mode::Reply(429, limited.clone()));
    let res = w.post("a", "/query", b"[]").await;
    assert_eq!(res["status"], 429);
    assert_eq!(body_of(&res), limited);
    relay.set(Mode::Reply(500, b"boom".to_vec()));
    assert_eq!(w.post("b", "/count", b"[]").await["status"], 500);
    relay.set(Mode::CloseMid);
    let res = w.post("c", "/events", b"{}").await;
    assert_eq!(res["code"], "upstream_connect");
    relay.set(Mode::Redirect);
    let res = w.post("d", "/events", b"{}").await;
    assert_eq!(
        (res["status"].as_u64(), &res["headers"]),
        (Some(302), &json!({}))
    );
    // Every request reached the relay exactly once, and the redirect was not followed.
    assert_eq!(relay.hits("/query"), 1);
    assert_eq!(relay.hits("/count"), 1);
    assert_eq!(relay.hits("/events"), 2);
    assert_eq!(relay.hits("/elsewhere"), 0);
}

#[tokio::test]
async fn size_limits_hold_both_ways() {
    let (relay, _gw, mut w) = setup(limits()).await;
    let res = w.post("big", "/events", &vec![b'a'; (1 << 20) + 1]).await;
    assert_eq!(res["code"], "body_too_large");
    assert_eq!(relay.hits("/events"), 0);
    relay.set(Mode::Reply(200, vec![b'a'; (4 << 20) + 1]));
    assert_eq!(
        w.post("huge", "/query", b"[]").await["code"],
        "response_too_large"
    );
}

#[tokio::test]
async fn frames_outside_the_table_are_refused() {
    let (relay, _gw, mut w) = setup(limits()).await;
    let auth = json!({"authorization": "Nostr test"});
    let cases = [
        (
            json!({"type":"http","id":"1","method":"POST","path":"/query?x=1","headers":auth,"body_b64":""}),
            "not_allowed",
        ),
        (
            json!({"type":"http","id":"2","method":"POST","path":"/query","headers":{"authorization":"x","x-pubkey":"00"},"body_b64":""}),
            "not_allowed",
        ),
        (
            json!({"type":"http","id":"3","method":"POST","path":"/query","headers":auth}),
            "not_allowed",
        ),
        (
            json!({"type":"http","id":"4","method":"POST","path":"/query","headers":auth,"body_b64":"!!"}),
            "bad_base64",
        ),
        (
            json!({"type":"http","id":"5","method":"GET","path":"/","headers":{"accept":"text/html"}}),
            "not_allowed",
        ),
    ];
    for (frame, code) in cases {
        let id = frame["id"].clone();
        w.send(frame).await;
        let res = w.frame().await;
        assert_eq!((&res["id"], res["code"].as_str()), (&id, Some(code)));
    }
    assert_eq!(relay.hits("/query") + relay.hits("/"), 0);
    let nip11 = json!({"accept": "application/nostr+json"});
    let info = w.http("i", "GET", "/", nip11, None).await;
    let doc: Value = serde_json::from_slice(&body_of(&info)).unwrap();
    assert_eq!(doc["accept"], "application/nostr+json");
}

#[tokio::test]
async fn in_flight_cap_duplicates_and_timeouts() {
    let quick = Limits {
        request_timeout: Duration::from_millis(800),
        ..limits()
    };
    let (relay, _gw, mut w) = setup(quick).await;
    relay.set(Mode::Stall);
    for i in 0..16 {
        w.send(query(&format!("s{i}"))).await;
    }
    w.send(query("s0")).await;
    assert_eq!(w.frame().await["code"], "duplicate_id");
    assert_eq!(w.post("s16", "/query", b"").await["code"], "busy");
    for _ in 0..16 {
        assert_eq!(w.frame().await["code"], "upstream_timeout");
    }
    relay.set(Mode::Echo);
    assert_eq!(w.post("after", "/query", b"[]").await["status"], 200);
}

#[tokio::test]
async fn a_handshake_that_never_ends_is_upstream_connect() {
    // Takes the TCP connection and never answers, so the TLS handshake stalls.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = silent.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((tcp, _)) = silent.accept().await {
            held.push(tcp);
        }
    });
    let quick = Limits {
        connect_timeout: Duration::from_millis(300),
        ..limits()
    };
    let gw = Gateway::start(&format!("wss://{addr}"), quick).await;
    let mut w = gw.worker().await;
    let res = w.post("c", "/query", b"[]").await;
    assert_eq!(res["code"], "upstream_connect", "{res}");
}

#[tokio::test]
async fn shutdown_drains_in_flight_requests() {
    let (relay, gw, mut w) = setup(limits()).await;
    relay.set(Mode::Stall);
    w.send(query("slow")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    gw.shutdown.cancel();
    let health = reqwest::get(gw.url("/healthz")).await.unwrap();
    assert_eq!(health.status(), 503);
    assert_eq!(
        w.post("late", "/query", b"[]").await["code"],
        "shutting_down"
    );
    assert_eq!(w.closed().await, Some(1012));
    tokio::time::timeout(Duration::from_secs(5), gw.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
