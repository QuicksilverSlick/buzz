mod common;

use std::time::Duration;

use buzz_relay_gateway::Limits;
use common::Gateway;
use tokio::{io::AsyncWriteExt, net::TcpStream, time::timeout};

#[tokio::test]
async fn a_half_sent_request_does_not_hold_up_shutdown() {
    let gw = Gateway::start("ws://127.0.0.1:9", Limits::default()).await;
    let mut tcp = TcpStream::connect(gw.addr).await.unwrap();
    // No blank line after the headers: hyper keeps waiting for the rest.
    tcp.write_all(b"GET /healthz HTTP/1.1\r\nhost: x\r\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    gw.shutdown.cancel();
    let served = timeout(Duration::from_secs(5), gw.task).await;
    assert!(matches!(served, Ok(Ok(Ok(())))), "serve did not return");
}
