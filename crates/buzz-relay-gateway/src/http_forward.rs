//! HTTP forwarding: one upstream request per `http` frame, never retried.

use std::collections::BTreeMap;

use axum::http::{HeaderMap, Method};
use reqwest::{redirect::Policy, Client};
use url::Url;

use crate::{frame::ErrorCode, policy, Config, Limits};

/// The relay's answer, read up to the response cap.
pub(crate) struct Answer {
    pub status: u16,
    pub headers: BTreeMap<&'static str, String>,
    pub body: Vec<u8>,
}

/// The one client for the relay: no redirects, no proxy, no decompression.
pub(crate) fn client(config: &Config, limits: &Limits) -> reqwest::Result<Client> {
    // The no_* calls also hold if workspace feature unification turns a codec on.
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .https_only(config.relay_http.scheme() == "https")
        .connect_timeout(limits.connect_timeout)
        .timeout(limits.request_timeout)
        .user_agent(concat!("buzz-relay-gateway/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Sends one request and reads the whole answer, whatever its status.
pub(crate) async fn forward(
    client: &Client,
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Option<Vec<u8>>,
    max_body: usize,
) -> Result<Answer, ErrorCode> {
    let mut request = client.request(method, url).headers(headers);
    if let Some(body) = body {
        request = request.body(body);
    }
    let mut response = request.send().await.map_err(upstream)?;
    let status = response.status().as_u16();
    let headers = policy::response_headers(response.headers());
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(upstream)? {
        if body.len() + chunk.len() > max_body {
            return Err(ErrorCode::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Answer {
        status,
        headers,
        body,
    })
}

fn upstream(e: reqwest::Error) -> ErrorCode {
    // An expired connect_timeout is also a timeout, but then nothing reached the relay.
    if e.is_timeout() && !e.is_connect() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamConnect
    }
}
