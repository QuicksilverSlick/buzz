//! The link bearer: a single-use NIP-98 event signed by the gateway-auth key.

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use axum::http::{header::AUTHORIZATION, HeaderMap};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_auth::{nip98_replay::DEFAULT_REPLAY_TTL_SECS, verify_nip98_event};
use nostr::{Event, EventId, PublicKey};

/// Why a bearer was refused. Only [`Reject::code`] is logged: the underlying
/// error text quotes bearer content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// No `Authorization` header.
    Missing,
    /// Repeated, oversized, not `Nostr <base64>`, or not UTF-8.
    Malformed,
    /// Failed NIP-98: kind, signature, time window, `u` or `method`.
    Invalid,
    /// A valid signature by a key other than the pinned one.
    WrongKey,
    /// Not exactly one `nonce` tag of 16 to 128 characters.
    NoNonce,
    /// The event id was already used.
    Replayed,
    /// The replay cache is full; the gateway fails closed.
    CacheFull,
}

impl Reject {
    /// The fixed reason code for logs.
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Malformed => "malformed",
            Self::Invalid => "invalid",
            Self::WrongKey => "wrong_key",
            Self::NoNonce => "no_nonce",
            Self::Replayed => "replayed",
            Self::CacheFull => "cache_full",
        }
    }
}

/// Checks the link bearer on an upgrade request and returns its event id,
/// which the caller then claims in the [`ReplayCache`].
pub fn verify_link_bearer(
    headers: &HeaderMap,
    link_url: &str,
    pubkey: &PublicKey,
    max_len: usize,
) -> Result<EventId, Reject> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let (value, None) = (values.next().ok_or(Reject::Missing)?, values.next()) else {
        return Err(Reject::Malformed);
    };
    let json = (value.len() <= max_len)
        .then(|| value.to_str().ok()?.strip_prefix("Nostr "))
        .flatten()
        .and_then(|b64| STANDARD.decode(b64).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or(Reject::Malformed)?;
    verify_nip98_event(&json, link_url, "GET", None).map_err(|_| Reject::Invalid)?;
    let event: Event = serde_json::from_str(&json).map_err(|_| Reject::Invalid)?;
    if event.pubkey != *pubkey {
        return Err(Reject::WrongKey);
    }
    let mut nonces = event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .filter(|t| t.first().is_some_and(|n| n == "nonce"));
    match (nonces.next(), nonces.next()) {
        (Some([_, n, ..]), None) if (16..=128).contains(&n.chars().count()) => Ok(event.id),
        _ => Err(Reject::NoNonce),
    }
}

/// Event ids of accepted bearers, each kept for the relay's replay TTL
/// (twice the ±60 s window). A restart empties it.
pub struct ReplayCache {
    cap: usize,
    seen: Mutex<HashMap<EventId, Instant>>,
}

impl ReplayCache {
    /// A cache that holds at most `cap` live ids.
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Claims `id` once. Refuses a replay, and refuses every id while full.
    pub fn claim(&self, id: EventId, now: Instant) -> Result<(), Reject> {
        let ttl = Duration::from_secs(DEFAULT_REPLAY_TTL_SECS);
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.retain(|_, at| now.saturating_duration_since(*at) < ttl);
        if seen.contains_key(&id) {
            return Err(Reject::Replayed);
        }
        if seen.len() >= self.cap {
            return Err(Reject::CacheFull);
        }
        seen.insert(id, now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    const URL: &str = "https://gateway.bishop.invalid/v1/link";
    const NONCE: &str = "6f1c2c1e-4b9a-4c71-9a55-0f1d8c3a2b10";

    fn event(keys: &Keys, kind: u16, tags: &[&[&str]], age: i64) -> Event {
        let tags = tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap());
        let at = Timestamp::from((Timestamp::now().as_secs() as i64 - age) as u64);
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .custom_created_at(at)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn header(event: &Event) -> String {
        format!(
            "Nostr {}",
            STANDARD.encode(serde_json::to_string(event).unwrap())
        )
    }

    fn good(keys: &Keys) -> String {
        let nonce = uuid::Uuid::new_v4().to_string();
        header(&event(
            keys,
            27235,
            &[&["u", URL], &["method", "GET"], &["nonce", &nonce]],
            0,
        ))
    }

    fn check(keys: &Keys, values: &[&str]) -> Result<EventId, Reject> {
        let mut headers = HeaderMap::new();
        for v in values {
            headers.append(AUTHORIZATION, HeaderValue::from_str(v).unwrap());
        }
        verify_link_bearer(&headers, URL, &keys.public_key(), 8 << 10)
    }

    #[test]
    fn accepts_a_good_bearer_and_url_normalisation() {
        let k = Keys::generate();
        assert!(check(&k, &[&good(&k)]).is_ok());
        for u in [
            "https://gateway.bishop.invalid/v1/link/",
            "HTTPS://Gateway.Bishop.Invalid/v1/link",
        ] {
            let b = header(&event(
                &k,
                27235,
                &[&["u", u], &["method", "get"], &["nonce", NONCE]],
                0,
            ));
            assert!(check(&k, &[&b]).is_ok(), "{u}");
        }
    }

    #[test]
    fn refuses_bad_bearers() {
        let k = Keys::generate();
        let with = |kind, tags: &[&[&str]], age| header(&event(&k, kind, tags, age));
        let (u, get, nonce) = (
            &["u", URL][..],
            &["method", "GET"][..],
            &["nonce", NONCE][..],
        );
        let mut tampered: serde_json::Value =
            serde_json::to_value(event(&k, 27235, &[u, get, nonce], 0)).unwrap();
        tampered["content"] = "x".into();
        let tampered = format!("Nostr {}", STANDARD.encode(tampered.to_string()));
        let cases = [
            (good(&Keys::generate()), Reject::WrongKey),
            (with(27235, &[u, get, nonce], 120), Reject::Invalid),
            (with(27235, &[u, get, nonce], -120), Reject::Invalid),
            (
                with(
                    27235,
                    &[&["u", "https://other.invalid/v1/link"], get, nonce],
                    0,
                ),
                Reject::Invalid,
            ),
            (
                with(27235, &[u, &["method", "POST"], nonce], 0),
                Reject::Invalid,
            ),
            (with(22242, &[u, get, nonce], 0), Reject::Invalid),
            (tampered, Reject::Invalid),
            (with(27235, &[u, get], 0), Reject::NoNonce),
            (
                with(
                    27235,
                    &[u, get, nonce, &["nonce", "a-second-nonce-value"]],
                    0,
                ),
                Reject::NoNonce,
            ),
            (
                with(27235, &[u, get, &["nonce", "short"]], 0),
                Reject::NoNonce,
            ),
            (good(&k).replacen("Nostr ", "Bearer ", 1), Reject::Malformed),
            ("Nostr !!!".to_owned(), Reject::Malformed),
            (
                format!("Nostr {}", STANDARD.encode([0xff, 0xfe])),
                Reject::Malformed,
            ),
            (format!("Nostr {}", "A".repeat(8 << 10)), Reject::Malformed),
        ];
        for (i, (bearer, want)) in cases.iter().enumerate() {
            assert_eq!(check(&k, &[bearer]), Err(*want), "case {i}");
        }
        assert_eq!(check(&k, &[]), Err(Reject::Missing));
        assert_eq!(check(&k, &[&good(&k), &good(&k)]), Err(Reject::Malformed));
    }

    #[test]
    fn replay_cache_is_single_use_prunes_and_fails_closed() {
        let k = Keys::generate();
        let (a, b) = (
            check(&k, &[&good(&k)]).unwrap(),
            check(&k, &[&good(&k)]).unwrap(),
        );
        let cache = ReplayCache::new(1);
        let now = Instant::now();
        assert_eq!(cache.claim(a, now), Ok(()));
        assert_eq!(cache.claim(a, now), Err(Reject::Replayed));
        assert_eq!(cache.claim(b, now), Err(Reject::CacheFull));
        let later = now + Duration::from_secs(DEFAULT_REPLAY_TTL_SECS);
        assert_eq!(cache.claim(b, later), Ok(()));
    }
}
