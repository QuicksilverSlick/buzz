use std::time::Duration;

use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn a_half_sent_request_does_not_hold_up_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut tcp = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let shutdown = CancellationToken::new();
    let server = tokio::spawn(buzz_relay_gateway::serve(listener, shutdown.clone()));
    // No blank line after the headers: hyper keeps waiting for the rest.
    tcp.write_all(b"GET /healthz HTTP/1.1\r\nhost: x\r\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    shutdown.cancel();
    let served = timeout(Duration::from_secs(5), server).await;
    assert!(matches!(served, Ok(Ok(Ok(())))), "serve did not return");
}
