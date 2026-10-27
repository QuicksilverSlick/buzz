mod common;

use buzz_relay_gateway::Limits;
use common::{bearer, Gateway};
use nostr::Keys;

#[tokio::test]
async fn bearer_gate_is_single_use_and_fails_closed() {
    let gw = Gateway::start(
        "ws://127.0.0.1:9",
        Limits {
            replay_entries: 1,
            ..Limits::default()
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
    // Refused bearers used no replay slot, so the one-slot cache still admits this one.
    let good = bearer(&gw.keys);
    assert_eq!(link(Some(&good)).await.status(), 501);
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
