mod common;

use std::time::Duration;

use buzz_relay_gateway::Limits;
use common::{bearer, limits, Gateway, Worker};
use futures_util::SinkExt;
use nostr::Keys;
use serde_json::json;
use tokio_tungstenite::tungstenite::{Error, Message};

// Nothing listens on port 9: link control never reaches the relay.
const NO_RELAY: &str = "ws://127.0.0.1:9";

#[tokio::test]
async fn bearer_gate_is_single_use_and_fails_closed() {
    let gw = Gateway::start(
        NO_RELAY,
        Limits {
            replay_entries: 1,
            ..limits()
        },
    )
    .await;
    let http = reqwest::Client::new();
    let link = |auth: Option<&str>| {
        let req = http.get(gw.url("/v1/link"));
        let req = match auth {
            Some(auth) => req.header("authorization", auth),
            None => req,
        };
        async move { req.send().await.unwrap() }
    };
    let none = link(None).await;
    assert_eq!(none.status(), 401);
    assert_eq!(none.text().await.unwrap(), r#"{"error":"unauthorized"}"#);
    assert_eq!(link(Some(&bearer(&Keys::generate()))).await.status(), 401);
    // Refused bearers used no replay slot, so the one-slot cache still admits this one;
    // it then fails only the upgrade, because this is a plain GET.
    let good = bearer(&gw.keys);
    assert_eq!(link(Some(&good)).await.status(), 400);
    assert_eq!(link(Some(&good)).await.status(), 401);
    assert_eq!(link(Some(&bearer(&gw.keys))).await.status(), 503);

    let health = http.get(gw.url("/healthz")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    assert_eq!(health.text().await.unwrap(), "ok");
    assert_eq!(
        http.get(gw.url("/v1/other")).send().await.unwrap().status(),
        404
    );

    gw.shutdown.cancel();
    gw.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn bad_bearers_get_no_upgrade() {
    let gw = Gateway::start(NO_RELAY, limits()).await;
    for bearer in [bearer(&Keys::generate()), "Nostr x".to_owned()] {
        let Err(Error::Http(response)) = Worker::connect(&gw, &bearer).await else {
            panic!("upgraded with a bad bearer");
        };
        assert_eq!(response.status(), 401);
    }
}

#[tokio::test]
async fn hello_then_ping_pong() {
    let gw = Gateway::start(NO_RELAY, limits()).await;
    let mut w = gw.worker().await;
    let hello = &w.hello;
    assert_eq!(hello["v"], 1);
    assert_eq!(
        hello["expires_at"].as_u64().unwrap() - hello["rotate_at"].as_u64().unwrap(),
        300
    );
    assert_eq!(
        hello["limits"],
        json!({"max_request_body":1048576,"max_response_body":4194304,"max_ws_text":524288,
               "max_http_inflight":16,"max_sockets":8})
    );
    w.send(json!({"type":"ping","ts":1793149200123u64})).await;
    assert_eq!(
        w.frame().await,
        json!({"type":"pong","ts":1793149200123u64})
    );
    w.send(json!({"type":"later","x":1})).await;
    assert_eq!(w.frame().await["code"], "unknown_type");
    w.send(json!({"type":"ping"})).await;
    assert_eq!(w.frame().await, json!({"type":"pong"}));
}

#[tokio::test]
async fn gateway_pings_and_silence_closes_4002() {
    let quick = Limits {
        ping_every: Duration::from_millis(100),
        idle: Duration::from_millis(600),
        ..limits()
    };
    let gw = Gateway::start(NO_RELAY, quick).await;
    let mut w = gw.worker().await;
    let ping = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Ok(Message::Text(text))) = futures_util::StreamExt::next(&mut w.ws).await {
                if text.contains(r#""type":"ping""#) {
                    return;
                }
            }
        }
    });
    ping.await.unwrap();
    assert_eq!(w.closed().await, Some(4002));
}

#[tokio::test]
async fn rotate_then_close_4001() {
    let short = Limits {
        link_lifetime: Duration::from_secs(2),
        rotate_before: Duration::from_secs(1),
        ..limits()
    };
    let gw = Gateway::start(NO_RELAY, short).await;
    let mut w = gw.worker().await;
    let rotate = w.frame().await;
    assert_eq!(rotate["type"], "rotate");
    assert_eq!(rotate["close_at"], w.hello["expires_at"]);
    assert_eq!(w.closed().await, Some(4001));
}

#[tokio::test]
async fn fifth_link_gets_503() {
    let gw = Gateway::start(NO_RELAY, limits()).await;
    let mut links = Vec::new();
    for _ in 0..4 {
        links.push(gw.worker().await);
    }
    let Err(Error::Http(response)) = Worker::connect(&gw, &bearer(&gw.keys)).await else {
        panic!("a fifth link opened");
    };
    assert_eq!(response.status(), 503);
    links.pop().unwrap().ws.close(None).await.unwrap();
    // The freed slot is usable again once that link has torn down.
    tokio::time::sleep(Duration::from_millis(300)).await;
    gw.worker().await;
}

#[tokio::test]
async fn protocol_violations_close_the_link() {
    let gw = Gateway::start(
        NO_RELAY,
        Limits {
            max_frame: 1024,
            ..limits()
        },
    )
    .await;
    let mut w = gw.worker().await;
    w.ws.send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_eq!(w.closed().await, Some(1003));

    let mut w = gw.worker().await;
    w.ws.send(Message::text("not json")).await.unwrap();
    assert_eq!(w.frame().await["code"], "bad_frame");
    assert_eq!(w.closed().await, Some(1008));

    let mut w = gw.worker().await;
    w.ws.send(Message::text("x".repeat(2048))).await.unwrap();
    assert_eq!(w.closed().await, Some(1009));
}

#[tokio::test]
async fn shutdown_closes_links_1012_and_serve_returns() {
    let gw = Gateway::start(NO_RELAY, limits()).await;
    let mut w = gw.worker().await;
    gw.shutdown.cancel();
    assert_eq!(w.closed().await, Some(1012));
    tokio::time::timeout(Duration::from_secs(5), gw.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
