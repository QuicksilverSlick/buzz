//! Environment configuration. Every value is public.

use std::net::SocketAddr;

use nostr::PublicKey;
use url::Url;

const BIND: &str = "BUZZ_RELAY_GATEWAY_BIND_ADDR";
const RELAY: &str = "BUZZ_RELAY_GATEWAY_RELAY_URL";
const LINK_URL: &str = "BUZZ_RELAY_GATEWAY_LINK_URL";
const LINK_PUBKEY: &str = "BUZZ_RELAY_GATEWAY_LINK_PUBKEY";

/// The gateway's settings. None of them is a secret.
#[derive(Clone, Debug)]
pub struct Config {
    /// Listen address; defaults to loopback.
    pub bind_addr: SocketAddr,
    /// The one relay the gateway reaches, `wss://host[:port]/`.
    pub relay_ws: Url,
    /// The same relay's HTTP origin, `https://host[:port]/`.
    pub relay_http: Url,
    /// The `u` tag every link bearer must carry.
    pub link_url: String,
    /// The gateway-auth public key that signs link bearers.
    pub link_pubkey: PublicKey,
}

/// A missing or invalid variable. It names the variable, never its value.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required variable is not set.
    #[error("{0} is not set")]
    Missing(&'static str),
    /// A variable is set but does not parse or is not allowed.
    #[error("{0} is invalid")]
    Invalid(&'static str),
}

impl Config {
    /// Reads `BUZZ_RELAY_GATEWAY_*` from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the settings through `get`, so tests need no process environment.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let required = |name: &'static str| get(name).ok_or(ConfigError::Missing(name));
        let bind_addr = get(BIND)
            .as_deref()
            .unwrap_or("127.0.0.1:8080")
            .parse()
            .map_err(|_| ConfigError::Invalid(BIND))?;
        let (relay_ws, relay_http) =
            relay_urls(&required(RELAY)?).ok_or(ConfigError::Invalid(RELAY))?;
        let link_url = required(LINK_URL)?;
        Url::parse(&link_url).map_err(|_| ConfigError::Invalid(LINK_URL))?;
        let hex = required(LINK_PUBKEY)?;
        let lower_hex =
            hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        let link_pubkey = PublicKey::from_hex(&hex)
            .ok()
            .filter(|_| lower_hex)
            .ok_or(ConfigError::Invalid(LINK_PUBKEY))?;
        Ok(Self {
            bind_addr,
            relay_ws,
            relay_http,
            link_url,
            link_pubkey,
        })
    }
}

/// `wss://host[:port]` with no userinfo, path, query or fragment; `ws://` only on loopback.
fn relay_urls(raw: &str) -> Option<(Url, Url)> {
    let ws = Url::parse(raw).ok()?;
    let loopback = matches!(ws.host_str()?, "127.0.0.1" | "[::1]" | "localhost");
    let scheme = match ws.scheme() {
        "wss" => "https",
        "ws" if loopback => "http",
        _ => return None,
    };
    let bare = ws.username().is_empty()
        && ws.password().is_none()
        && ws.path() == "/"
        && ws.query().is_none()
        && ws.fragment().is_none();
    let mut http = ws.clone();
    http.set_scheme(scheme).ok()?;
    bare.then_some((ws, http))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The secp256k1 generator x: a valid key whose secret (1) is public anyway.
    const PK: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    fn load(relay: &str, pubkey: &str) -> Result<Config, ConfigError> {
        Config::from_lookup(|name| match name {
            RELAY => Some(relay.to_owned()),
            LINK_URL => Some("https://gateway.bishop.invalid/v1/link".to_owned()),
            LINK_PUBKEY => Some(pubkey.to_owned()),
            _ => None,
        })
    }

    #[test]
    fn accepts_wss_and_loopback_ws() {
        let c = load("wss://relay.example.com", PK).unwrap();
        assert_eq!(c.relay_ws.as_str(), "wss://relay.example.com/");
        assert_eq!(c.relay_http.as_str(), "https://relay.example.com/");
        assert_eq!(c.bind_addr.to_string(), "127.0.0.1:8080");
        for ok in [
            "ws://127.0.0.1:7000",
            "ws://localhost:7000/",
            "ws://[::1]:7000",
            "wss://r.example:444",
        ] {
            assert!(load(ok, PK).is_ok(), "{ok}");
        }
        assert_eq!(
            load("ws://127.0.0.1:7000", PK).unwrap().relay_http.as_str(),
            "http://127.0.0.1:7000/"
        );
    }

    #[test]
    fn refuses_other_relay_urls() {
        for bad in [
            "ws://example.com",
            "wss://relay.example.com/path",
            "wss://relay.example.com/?q=1",
            "wss://relay.example.com/#f",
            "wss://user:pw@relay.example.com",
            "https://relay.example.com",
            "http://127.0.0.1:7000",
            "relay.example.com",
        ] {
            assert!(
                matches!(load(bad, PK), Err(ConfigError::Invalid(RELAY))),
                "{bad}"
            );
        }
    }

    #[test]
    fn refuses_bad_pubkey_and_names_missing_variables() {
        for bad in [&PK[..63], &PK.to_uppercase(), &format!("{}zz", &PK[..62])] {
            assert!(matches!(
                load("wss://r.example", bad),
                Err(ConfigError::Invalid(LINK_PUBKEY))
            ));
        }
        let err = Config::from_lookup(|_| None).unwrap_err();
        assert_eq!(err.to_string(), "BUZZ_RELAY_GATEWAY_RELAY_URL is not set");
    }
}
