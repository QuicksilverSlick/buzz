//! Frames v1 between a PC voice bridge and Talos `/df/voice`.
//!
//! JSON text frames carry `"v": 1` and a `type`; audio is binary PCM16 LE, mono, 24 kHz.
//! `frames-v1/schema.json` and one fixture per frame in `frames-v1/fixtures/` are the contract
//! Talos copies. Times: `t_ms`, `start_ms` and `end_ms` are session audio ms; `from_ms` and
//! `to_ms` are unix ms; `expires_at` and `until` are unix seconds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `v` every frame carries. An incompatible change bumps it in both repos on the same day.
pub const VERSION: u64 = 1;

/// One text frame. Kept one per line so it reads as the design's list.
#[rustfmt::skip]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    // PC to Talos.
    Open { huddle: String, agent: String, proofs: Proofs, seed: Seed, profile: Profile },
    Resume { lease_id: String },
    Seed(Seed),
    Speaker { span: u64, pubkey: String, role: Role, echo: bool, t_ms: u64 },
    Onset { pubkey: String },
    Replay { span: u64, from_ms: u64, to_ms: u64 },
    ReplayEnd { span: u64 },
    Roster { humans: Vec<String>, voices: Vec<String> },
    ToolResult { id: String, ok: bool, text: String },
    Handoff { kind: HandoffKind, text: String },
    Pong { t: u64 },
    // Talos to PC.
    Opened { lease_id: String, expires_at: u64 },
    Transcript { dir: Dir, delta: String, start_ms: u64, end_ms: u64 },
    Tool { id: String, name: String, args: Value },
    Floor { holder: Option<String>, token: u64 },
    Mute { on: bool },
    Quiet {},
    Awake { expires_at: u64 },
    Reseed { reason: String },
    ReplayReq { from_ms: u64, to_ms: u64 },
    Elevated { until: u64 },
    Secret { from_ms: u64, to_ms: u64 },
    Meter { seconds: u64, left_usd: f64, today_left_s: u64 },
    Notice { kind: NoticeKind, text: String },
    Summary { text: String, read_channels: Vec<String> },
    Ping { t: u64 },
    Close { reason: String },
}

/// Signed events, kept as raw JSON for Talos to verify: the owner's 30177 for the agent and
/// the relay's 44100 adding it to this huddle.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Proofs {
    pub agent_record: Value,
    pub added: Value,
}

/// Startup context built on the PC under the room intersection; `input` is GPT-Live `input`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Seed {
    pub intersection_hash: String,
    pub input: Vec<Value>,
}

/// The agent's voice profile: a built-in voice name or custom voice id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Profile {
    pub voice: String,
    pub persona: String,
    pub aliases: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Admitted,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    In,
    Out,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HandoffKind {
    Money,
    Card,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Cap80,
    Cap100,
    Offline,
}

/// Parse one text frame, refusing any other version.
pub fn parse(text: &str) -> Result<Frame, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("frame: {e}"))?;
    if value.get("v").and_then(Value::as_u64) != Some(VERSION) {
        return Err("frame: not v1".into());
    }
    Frame::deserialize(value).map_err(|e| format!("frame: {e}"))
}

/// Serialize one text frame with its `v`.
pub fn to_text(frame: &Frame) -> String {
    let mut value = serde_json::to_value(frame).expect("frames are plain data");
    value["v"] = VERSION.into();
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/frames-v1");

    fn fixtures() -> Vec<(String, Value)> {
        let mut out: Vec<_> = std::fs::read_dir(format!("{DIR}/fixtures"))
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                let text = std::fs::read_to_string(&path).unwrap();
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                (name, serde_json::from_str(&text).unwrap())
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    #[test]
    fn every_fixture_round_trips() {
        let fixtures = fixtures();
        assert_eq!(fixtures.len(), 27, "one fixture per frame");
        for (name, value) in fixtures {
            assert_eq!(value["type"], name.as_str(), "{name}: named after its type");
            let frame = parse(&value.to_string()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let back: Value = serde_json::from_str(&to_text(&frame)).unwrap();
            assert_eq!(back, value, "{name}: round trip");
        }
    }

    /// The schema Talos copies names the same frames and fields as the fixtures.
    #[test]
    fn schema_matches_fixtures() {
        let text = std::fs::read_to_string(format!("{DIR}/schema.json")).unwrap();
        let schema: Value = serde_json::from_str(&text).unwrap();
        let branches = schema["oneOf"].as_array().unwrap();
        let fixtures = fixtures();
        assert_eq!(branches.len(), fixtures.len());
        let keys = |v: &Value| -> BTreeSet<String> {
            match v {
                Value::Object(m) => m.keys().filter(|k| *k != "v").cloned().collect(),
                Value::Array(a) => a.iter().map(|k| k.as_str().unwrap().to_owned()).collect(),
                _ => panic!("schema shape"),
            }
        };
        for (name, value) in fixtures {
            let branch = branches
                .iter()
                .find(|b| b["properties"]["type"]["const"] == name.as_str())
                .unwrap_or_else(|| panic!("{name}: no schema branch"));
            let declared = keys(&branch["properties"]);
            assert_eq!(
                keys(&value),
                declared,
                "{name}: fixture fields match the schema"
            );
            assert_eq!(
                keys(&branch["required"]),
                declared,
                "{name}: every field required"
            );
        }
    }

    #[test]
    fn other_versions_are_refused() {
        assert!(parse(r#"{"v":2,"type":"ping","t":1}"#).is_err());
        assert!(parse(r#"{"type":"ping","t":1}"#).is_err());
        assert_eq!(
            parse(r#"{"v":1,"type":"ping","t":1}"#),
            Ok(Frame::Ping { t: 1 })
        );
    }
}
