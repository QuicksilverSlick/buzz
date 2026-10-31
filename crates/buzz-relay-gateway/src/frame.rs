//! Link frames, version 1 (design §3): one JSON object per WebSocket text frame.
//!
//! The frame types that carry headers, bodies or relay text do not derive
//! `Debug`, so none of them can reach a log line by accident.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Number};

use crate::policy::Refused;

/// A Worker-chosen request or socket id: 1 to 64 of `[A-Za-z0-9_-]`, so it is safe to log.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(try_from = "String")]
pub struct Id(String);

impl TryFrom<String> for Id {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let valid = (1..=64).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        valid.then_some(Self(s)).ok_or("invalid id")
    }
}

impl Id {
    /// The id, for logs.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An `http` frame: one request to forward to the relay.
#[derive(Deserialize)]
pub struct HttpFrame {
    /// Unique among this link's requests in flight.
    pub id: Id,
    /// `GET` or `POST`, matched exactly by [`crate::policy::http_route`].
    pub method: String,
    /// Matched exactly by [`crate::policy::http_route`].
    pub path: String,
    /// The raw `headers` object, checked by [`crate::policy::request_headers`].
    pub headers: Option<Box<RawValue>>,
    /// The request body, standard base64 (POST only).
    pub body_b64: Option<String>,
}

/// A frame from the Worker.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    /// Forward one HTTP request. Parsed on its own: `RawValue` does not
    /// survive serde's internally tagged enums.
    #[serde(skip)]
    Http(HttpFrame),
    /// Open a relay socket on `path`.
    WsOpen { sid: Id, path: String },
    /// Send `text` on a relay socket, as is.
    WsSend { sid: Id, text: String },
    /// Close a relay socket.
    WsClose {
        sid: Id,
        code: Option<u16>,
        reason: Option<String>,
    },
    /// Liveness; answered with a `pong` carrying the same `ts`.
    Ping { ts: Option<Number> },
    /// The answer to a gateway `ping`.
    Pong,
    /// A `type` this version does not know; the link stays open.
    #[serde(other)]
    Unknown,
}

/// Not JSON, no `type`, or a known type with a missing or mistyped field.
#[derive(Debug, PartialEq, Eq)]
pub struct BadFrame;

#[derive(Deserialize)]
struct TypeOnly {
    #[serde(rename = "type")]
    kind: String,
}

impl Inbound {
    /// Parses one text frame. Each type reads only its own fields, so a field
    /// that belongs to another type is ignored like any unknown field.
    pub fn parse(text: &str) -> Result<Self, BadFrame> {
        let TypeOnly { kind } = serde_json::from_str(text).map_err(|_| BadFrame)?;
        let frame = if kind == "http" {
            serde_json::from_str(text).map(Self::Http)
        } else {
            serde_json::from_str(text)
        };
        frame.map_err(|_| BadFrame)
    }
}

/// Which side ended a relay socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The relay closed it or the connection dropped.
    Relay,
    /// The gateway closed it (limits, link end, shutdown, failed open).
    Gateway,
    /// The Worker asked with `ws_close`.
    Worker,
}

/// The limits announced in `hello`.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct HelloLimits {
    pub max_request_body: usize,
    pub max_response_body: usize,
    pub max_ws_text: usize,
    pub max_http_inflight: usize,
    pub max_sockets: usize,
}

/// A frame to the Worker.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Out {
    /// The first frame on every link.
    Hello {
        v: u8,
        link_id: String,
        expires_at: u64,
        rotate_at: u64,
        limits: HelloLimits,
    },
    /// The relay's answer, whatever its status.
    HttpRes {
        id: Id,
        status: u16,
        headers: BTreeMap<&'static str, String>,
        body_b64: String,
        ms: u64,
    },
    /// The relay accepted the socket upgrade.
    WsOpened { sid: Id, ms: u64 },
    /// One text frame from the relay, as is.
    WsMsg { sid: Id, text: String },
    /// The end of a socket, sent exactly once per `sid`.
    WsClosed {
        sid: Id,
        by: Side,
        code: Option<u16>,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        http_status: Option<u16>,
    },
    /// A frame the gateway refused or could not perform.
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<Id>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sid: Option<Id>,
        code: ErrorCode,
        message: &'static str,
    },
    /// The link closes with 4001 at `close_at` (unix seconds).
    Rotate { close_at: u64 },
    /// Liveness; the Worker answers `pong`.
    Ping { ts: Number },
    /// The answer to a Worker `ping`.
    Pong {
        #[serde(skip_serializing_if = "Option::is_none")]
        ts: Option<Number>,
    },
}

impl Out {
    /// An `error` frame with the code's fixed message.
    pub fn error(code: ErrorCode, id: Option<Id>, sid: Option<Id>) -> Self {
        Self::Error {
            id,
            sid,
            code,
            message: code.message(),
        }
    }
}

/// `error.code`. Each code has a fixed message that never quotes frame content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    UnknownType,
    NotAllowed,
    NotSupported,
    BadBase64,
    BodyTooLarge,
    FrameTooLarge,
    ResponseTooLarge,
    DuplicateId,
    UnknownSocket,
    Busy,
    UpstreamConnect,
    UpstreamTimeout,
    ShuttingDown,
}

impl ErrorCode {
    /// The fixed English phrase for `message`.
    pub fn message(self) -> &'static str {
        match self {
            Self::BadFrame => "malformed frame",
            Self::UnknownType => "unknown frame type",
            Self::NotAllowed => "not allowed",
            Self::NotSupported => "not supported yet",
            Self::BadBase64 => "body_b64 is not valid base64",
            Self::BodyTooLarge => "request body too large",
            Self::FrameTooLarge => "text frame too large",
            Self::ResponseTooLarge => "relay response too large",
            Self::DuplicateId => "id already in use",
            Self::UnknownSocket => "no such socket",
            Self::Busy => "too many requests or sockets in flight",
            Self::UpstreamConnect => "could not reach the relay",
            Self::UpstreamTimeout => "the relay did not answer in time",
            Self::ShuttingDown => "gateway shutting down",
        }
    }
}

impl From<Refused> for ErrorCode {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::NotAllowed => Self::NotAllowed,
            Refused::NotSupported => Self::NotSupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(s: &str) -> Id {
        Id::try_from(s.to_owned()).unwrap()
    }

    #[test]
    fn parses_every_inbound_type() {
        let http = r#"{"type":"http","id":"q-1","method":"POST","path":"/query",
            "headers":{"accept":"a","accept":"b"},"body_b64":"e30=","extra":1}"#;
        let Ok(Inbound::Http(f)) = Inbound::parse(http) else {
            panic!("http")
        };
        assert_eq!(
            (f.id, f.method.as_str(), f.path.as_str()),
            (id("q-1"), "POST", "/query")
        );
        // Kept raw, so the policy can still see the duplicated key.
        assert_eq!(f.headers.unwrap().get(), r#"{"accept":"a","accept":"b"}"#);
        assert_eq!(f.body_b64.as_deref(), Some("e30="));
        let open = Inbound::parse(r#"{"type":"ws_open","sid":"s1","path":"/"}"#);
        assert!(
            matches!(open, Ok(Inbound::WsOpen { sid, path }) if sid == id("s1") && path == "/")
        );
        let send = Inbound::parse(r#"{"type":"ws_send","sid":"s1","text":"[\"AUTH\",{}]"}"#);
        assert!(matches!(send, Ok(Inbound::WsSend { text, .. }) if text == r#"["AUTH",{}]"#));
        let close = Inbound::parse(r#"{"type":"ws_close","sid":"s1","code":4000,"reason":"bye"}"#);
        assert!(
            matches!(close, Ok(Inbound::WsClose { code: Some(4000), reason: Some(r), .. }) if r == "bye")
        );
        let ping = Inbound::parse(r#"{"type":"ping","ts":1793149200123}"#);
        assert!(
            matches!(ping, Ok(Inbound::Ping { ts: Some(ts) }) if ts.to_string() == "1793149200123")
        );
        assert!(matches!(
            Inbound::parse(r#"{"type":"pong","ts":1}"#),
            Ok(Inbound::Pong)
        ));
        assert!(matches!(
            Inbound::parse(r#"{"type":"later"}"#),
            Ok(Inbound::Unknown)
        ));
        assert!(matches!(
            Inbound::parse(r#"{"type":"later","code":"x"}"#),
            Ok(Inbound::Unknown)
        ));
        // Another frame type's field, of another type, is still an unknown field here.
        for foreign in [
            r#"{"type":"ping","ts":1,"code":"later"}"#,
            r#"{"type":"ping","ts":1,"id":7}"#,
            r#"{"type":"pong","sid":[]}"#,
            r#"{"type":"ws_send","sid":"s1","text":"x","path":5}"#,
            r#"{"type":"http","id":"q","method":"GET","path":"/","ts":"x","text":1}"#,
        ] {
            assert!(Inbound::parse(foreign).is_ok(), "{foreign}");
        }
    }

    #[test]
    fn refuses_bad_frames_and_ids() {
        for bad in [
            "not json",
            "[]",
            r#"{"id":"x"}"#,
            r#"{"type":"http","id":"q","method":"POST"}"#,
            r#"{"type":"ws_send","sid":"s1"}"#,
            r#"{"type":"ws_open","sid":"has space","path":"/"}"#,
            r#"{"type":"ws_open","sid":"","path":"/"}"#,
            r#"{"type":"ping","ts":"soon"}"#,
            r#"{"type":5}"#,
            r#"{"type":"ws_open","sid":"s1","sid":"s2","path":"/"}"#,
        ] {
            assert!(Inbound::parse(bad).is_err(), "{bad}");
        }
        let long = format!(
            r#"{{"type":"ws_open","sid":"{}","path":"/"}}"#,
            "a".repeat(65)
        );
        assert_eq!(Inbound::parse(&long).err(), Some(BadFrame));
        assert!(Id::try_from("A-z_09".to_owned()).is_ok());
        assert!(Id::try_from("a".repeat(64)).is_ok());
    }

    #[test]
    fn serializes_every_outbound_type() {
        let limits = HelloLimits {
            max_request_body: 1,
            max_response_body: 2,
            max_ws_text: 3,
            max_http_inflight: 4,
            max_sockets: 5,
        };
        let headers = BTreeMap::from([("content-type", "application/json".to_owned())]);
        let cases = [
            (
                Out::Hello {
                    v: 1,
                    link_id: "l".into(),
                    expires_at: 10,
                    rotate_at: 9,
                    limits,
                },
                json!({"type":"hello","v":1,"link_id":"l","expires_at":10,"rotate_at":9,"limits":
                    {"max_request_body":1,"max_response_body":2,"max_ws_text":3,"max_http_inflight":4,"max_sockets":5}}),
            ),
            (
                Out::HttpRes {
                    id: id("q"),
                    status: 429,
                    headers,
                    body_b64: "e30=".into(),
                    ms: 7,
                },
                json!({"type":"http_res","id":"q","status":429,"headers":{"content-type":"application/json"},"body_b64":"e30=","ms":7}),
            ),
            (
                Out::WsOpened {
                    sid: id("s"),
                    ms: 3,
                },
                json!({"type":"ws_opened","sid":"s","ms":3}),
            ),
            (
                Out::WsMsg {
                    sid: id("s"),
                    text: "[1]".into(),
                },
                json!({"type":"ws_msg","sid":"s","text":"[1]"}),
            ),
            (
                Out::WsClosed {
                    sid: id("s"),
                    by: Side::Gateway,
                    code: None,
                    reason: "upgrade_refused".into(),
                    http_status: Some(503),
                },
                json!({"type":"ws_closed","sid":"s","by":"gateway","code":null,"reason":"upgrade_refused","http_status":503}),
            ),
            (
                Out::WsClosed {
                    sid: id("s"),
                    by: Side::Relay,
                    code: Some(1012),
                    reason: "r".into(),
                    http_status: None,
                },
                json!({"type":"ws_closed","sid":"s","by":"relay","code":1012,"reason":"r"}),
            ),
            (
                Out::error(ErrorCode::DuplicateId, Some(id("q")), None),
                json!({"type":"error","id":"q","code":"duplicate_id","message":"id already in use"}),
            ),
            (
                Out::error(ErrorCode::UnknownType, None, None),
                json!({"type":"error","code":"unknown_type","message":"unknown frame type"}),
            ),
            (
                Out::Rotate { close_at: 5 },
                json!({"type":"rotate","close_at":5}),
            ),
            (Out::Ping { ts: 6.into() }, json!({"type":"ping","ts":6})),
            (Out::Pong { ts: None }, json!({"type":"pong"})),
        ];
        for (frame, want) in cases {
            assert_eq!(serde_json::to_value(&frame).unwrap(), want);
        }
    }
}
