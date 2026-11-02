//! What may reach the relay: an exact route table and closed header sets (design §4).

use std::collections::BTreeMap;

use axum::http::{header, HeaderMap, HeaderValue, Method};
use serde::Deserialize;

const NIP11: &str = "application/nostr+json";

/// A relay HTTP route. The upstream path is always one of these literals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpRoute {
    /// `POST /query`
    Query,
    /// `POST /events`
    Events,
    /// `POST /count`
    Count,
    /// `GET /` for the NIP-11 relay information document.
    Info,
}

impl HttpRoute {
    /// The exact path sent upstream.
    pub fn path(self) -> &'static str {
        match self {
            Self::Query => "/query",
            Self::Events => "/events",
            Self::Count => "/count",
            Self::Info => "/",
        }
    }

    /// The upstream method; every POST route carries a body and GET carries none.
    pub fn method(self) -> Method {
        if self == Self::Info {
            Method::GET
        } else {
            Method::POST
        }
    }
}

/// Why a frame may not go upstream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not in the table, or a header that is not allowed.
    NotAllowed,
    /// A route reserved for a later phase (the huddle audio path).
    NotSupported,
}

/// Matches a frame's method and path exactly: no normalisation, no query string.
pub fn http_route(method: &str, path: &str) -> Result<HttpRoute, Refused> {
    match (method, path) {
        ("POST", "/query") => Ok(HttpRoute::Query),
        ("POST", "/events") => Ok(HttpRoute::Events),
        ("POST", "/count") => Ok(HttpRoute::Count),
        ("GET", "/") => Ok(HttpRoute::Info),
        _ => Err(Refused::NotAllowed),
    }
}

/// Relay socket paths: `/` only. The huddle audio path is reserved for Phase 2.
pub fn ws_route(path: &str) -> Result<&'static str, Refused> {
    if path == "/" {
        return Ok("/");
    }
    let audio = path
        .strip_prefix("/huddle/")
        .and_then(|rest| rest.strip_suffix("/audio"))
        .is_some_and(|id| id.len() == 36 && uuid::Uuid::try_parse(id).is_ok());
    Err(if audio {
        Refused::NotSupported
    } else {
        Refused::NotAllowed
    })
}

// A struct, not a map: serde refuses unknown and duplicated keys for it.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameHeaders {
    authorization: Option<String>,
    #[serde(rename = "content-type")]
    content_type: Option<String>,
    accept: Option<String>,
}

/// Builds the upstream request headers from a frame's raw `headers` object.
/// POST routes need `authorization`; `GET /` needs exactly the NIP-11 `accept`.
pub fn request_headers(
    route: HttpRoute,
    raw: Option<&str>,
    max_auth: usize,
) -> Result<HeaderMap, Refused> {
    let h: FrameHeaders = match raw {
        Some(raw) => serde_json::from_str(raw).map_err(|_| Refused::NotAllowed)?,
        None => FrameHeaders::default(),
    };
    let ok = match route {
        HttpRoute::Info => {
            h.authorization.is_none()
                && h.content_type.is_none()
                && h.accept.as_deref() == Some(NIP11)
        }
        _ => h
            .authorization
            .as_ref()
            .is_some_and(|a| a.len() <= max_auth),
    };
    if !ok {
        return Err(Refused::NotAllowed);
    }
    let mut out = HeaderMap::new();
    for (name, value) in [
        (header::AUTHORIZATION, h.authorization),
        (header::CONTENT_TYPE, h.content_type),
        (header::ACCEPT, h.accept),
    ] {
        if let Some(value) = value {
            out.insert(
                name,
                HeaderValue::from_str(&value).map_err(|_| Refused::NotAllowed)?,
            );
        }
    }
    Ok(out)
}

/// The relay response headers the Worker sees: `content-type` and `retry-after` only.
pub fn response_headers(headers: &HeaderMap) -> BTreeMap<&'static str, String> {
    [
        ("content-type", header::CONTENT_TYPE),
        ("retry-after", header::RETRY_AFTER),
    ]
    .into_iter()
    .filter_map(|(key, name)| Some((key, headers.get(name)?.to_str().ok()?.to_owned())))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_table_is_exact() {
        assert_eq!(http_route("POST", "/query"), Ok(HttpRoute::Query));
        assert_eq!(http_route("POST", "/events"), Ok(HttpRoute::Events));
        assert_eq!(http_route("POST", "/count"), Ok(HttpRoute::Count));
        assert_eq!(http_route("GET", "/"), Ok(HttpRoute::Info));
        for (m, p) in [
            ("GET", "/query"),
            ("PUT", "/query"),
            ("post", "/query"),
            ("POST", "/QUERY"),
            ("POST", "/query/"),
            ("POST", "//query"),
            ("POST", "/query?x=1"),
            ("POST", "/./query"),
            ("POST", "/%71uery"),
            ("POST", "/"),
            ("GET", "/hooks/1"),
        ] {
            assert_eq!(http_route(m, p), Err(Refused::NotAllowed), "{m} {p}");
        }
        assert_eq!(ws_route("/"), Ok("/"));
        assert_eq!(
            ws_route("/huddle/6f1c2c1e-4b9a-4c71-9a55-0f1d8c3a2b10/audio"),
            Err(Refused::NotSupported)
        );
        for p in [
            "",
            "/query",
            "//",
            "/?x",
            "/huddle/x/audio",
            "/huddle/6f1c2c1e4b9a4c719a550f1d8c3a2b10/audio",
        ] {
            assert_eq!(ws_route(p), Err(Refused::NotAllowed), "{p}");
        }
    }

    #[test]
    fn request_headers_are_a_closed_set() {
        let post = |raw: &str| request_headers(HttpRoute::Query, Some(raw), 16);
        let h = post(r#"{"authorization":"Nostr abc","content-type":"application/json"}"#).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[header::AUTHORIZATION], "Nostr abc");
        for bad in [
            r#"{"authorization":"Nostr abc","x-pubkey":"00"}"#,
            r#"{"authorization":"Nostr abc","host":"evil.example"}"#,
            r#"{"authorization":"Nostr abc","x-forwarded-for":"1.2.3.4"}"#,
            r#"{"authorization":"Nostr abc","accept":"a","accept":"b"}"#,
            r#"{"authorization":"Nostr abc\r\nx-pubkey: 00"}"#,
            r#"{"authorization":"Nostr abc","content-type":"a\nb"}"#,
            r#"{"Authorization":"Nostr abc"}"#,
            r#"{"authorization":"Nostr 0123456789abc"}"#,
            r#"{"content-type":"application/json"}"#,
            r#"["authorization"]"#,
        ] {
            assert_eq!(post(bad), Err(Refused::NotAllowed), "{bad}");
        }
        assert_eq!(
            request_headers(HttpRoute::Query, None, 16),
            Err(Refused::NotAllowed)
        );
        let info = |raw: &str| request_headers(HttpRoute::Info, Some(raw), 16);
        assert!(info(r#"{"accept":"application/nostr+json"}"#).is_ok());
        assert_eq!(
            info(r#"{"accept":"application/json"}"#),
            Err(Refused::NotAllowed)
        );
        assert_eq!(
            info(r#"{"accept":"application/nostr+json","authorization":"x"}"#),
            Err(Refused::NotAllowed)
        );
        assert_eq!(
            request_headers(HttpRoute::Info, None, 16),
            Err(Refused::NotAllowed)
        );
    }

    #[test]
    fn response_headers_keep_two_names() {
        let mut h = HeaderMap::new();
        for (name, value) in [
            ("content-type", "application/json"),
            ("retry-after", "12"),
            ("set-cookie", "a=b"),
            ("location", "/elsewhere"),
            ("server", "cloudflare"),
            ("cf-ray", "1"),
        ] {
            h.insert(
                header::HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        let kept = response_headers(&h);
        assert_eq!(
            kept.into_iter().collect::<Vec<_>>(),
            [
                ("content-type", "application/json".into()),
                ("retry-after", "12".into())
            ]
        );
    }
}
