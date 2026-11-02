//! Frames v1 between a PC voice bridge and Talos `/df/voice`.
//!
//! JSON text frames carry `"v": 1` and a `type`; audio is binary PCM16 LE, mono, 24 kHz.
//!
//! Talos sends a response's PCM paced to real time, and none between responses. The PC queues
//! at most 320 ms and drops the rest; after a barge-in it drops PCM until none has come for
//! 300 ms, or a newer `floor` token arrives. A huddle's `floor` token never goes down.
//! [`ToTalos`] and [`ToPc`] are the two directions: each side parses only the other side's
//! frames, and unknown fields are refused. `frames-v1/schema.json` and one fixture per frame in
//! `frames-v1/fixtures/` are the contract Talos copies.
//!
//! Clocks:
//! - `t_ms`, `start_ms` and `end_ms` are session audio ms: ms of PCM the PC has sent on this
//!   socket, live and replayed alike, so both sides count the same samples.
//! - `from_ms` and `to_ms` are unix ms on the sender's clock. The PC widens each `secret`
//!   window by a skew margin on both sides (a knob, 1 s to start) and starts dropping on
//!   receipt even if `from_ms` is still ahead on its own clock, so skew only widens a secret
//!   window. `replay` windows are best effort on the same clock.
//! - `expires_at` and `until` are unix seconds.
//!
//! Between `replay` and `replay_end` the PC sends only the replayed PCM, no live audio.
//!
//! Binding: Talos refuses an `open` whose `agent` is not the socket's NIP-98 signer, and a
//! `resume` whose lease is not keyed to that pubkey and huddle. `speaker`, `onset` and `roster`
//! grant no authority; Talos uses them only for turns, waking and labels (design §3.6).

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

/// The `v` every frame carries. An incompatible change bumps it in both repos on the same day.
pub const VERSION: u64 = 1;

/// PC to Talos. Kept one per line so it reads as the design's list.
#[rustfmt::skip]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToTalos {
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
}

/// Talos to PC.
#[rustfmt::skip]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToPc {
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
#[serde(deny_unknown_fields)]
pub struct Proofs {
    pub agent_record: Value,
    pub added: Value,
}

/// Startup context built on the PC under the room intersection; `input` is GPT-Live `input`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub intersection_hash: String,
    pub input: Vec<Value>,
}

/// The agent's voice profile: a built-in voice name or custom voice id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
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

/// Parse one text frame of one direction, refusing any other version.
pub fn parse<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    let mut value: Value = serde_json::from_str(text).map_err(|e| format!("frame: {e}"))?;
    let v = value.as_object_mut().and_then(|m| m.remove("v"));
    if v.as_ref().and_then(Value::as_u64) != Some(VERSION) {
        return Err("frame: not v1".into());
    }
    T::deserialize(value).map_err(|e| format!("frame: {e}"))
}

/// Serialize one text frame with its `v`.
pub fn to_text<T: Serialize>(frame: &T) -> String {
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

    fn schema() -> Value {
        let text = std::fs::read_to_string(format!("{DIR}/schema.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> Option<Value> {
        let frame = parse::<T>(&value.to_string()).ok()?;
        Some(serde_json::from_str(&to_text(&frame)).unwrap())
    }

    #[test]
    fn every_fixture_round_trips_in_exactly_one_direction() {
        let fixtures = fixtures();
        assert_eq!(fixtures.len(), 27, "one fixture per frame");
        let mut to_talos = 0;
        for (name, value) in fixtures {
            assert_eq!(value["type"], name.as_str(), "{name}: named after its type");
            let back = match (round_trip::<ToTalos>(&value), round_trip::<ToPc>(&value)) {
                (Some(back), None) => {
                    to_talos += 1;
                    back
                }
                (None, Some(back)) => back,
                other => panic!("{name}: parsed as {other:?}"),
            };
            assert_eq!(back, value, "{name}: round trip");
            let mut extra = value.clone();
            extra["claimed_role"] = "owner".into();
            assert!(
                round_trip::<ToTalos>(&extra).is_none(),
                "{name}: extra field"
            );
            assert!(round_trip::<ToPc>(&extra).is_none(), "{name}: extra field");
        }
        assert_eq!(to_talos, 11, "PC to Talos frames");
    }

    /// A small JSON Schema walker for the keywords schema.json uses. It returns the property
    /// names it evaluated, for `unevaluatedProperties`.
    fn check(root: &Value, s: &Value, v: &Value, at: &str) -> Result<BTreeSet<String>, String> {
        let fail = |what: &str| Err(format!("{at}: {what}"));
        let mut seen = BTreeSet::new();
        if let Some(r) = s["$ref"].as_str() {
            let def = &root["$defs"][r.strip_prefix("#/$defs/").unwrap()];
            seen.extend(check(root, def, v, at)?);
        }
        if s.get("const").is_some_and(|c| c != v) {
            return fail("const");
        }
        if s.get("enum")
            .is_some_and(|e| !e.as_array().unwrap().contains(v))
        {
            return fail("enum");
        }
        let typed = match s["type"].as_str() {
            None => true,
            Some("string") => v.is_string(),
            Some("integer") => v.is_i64() || v.is_u64(),
            Some("number") => v.is_number(),
            Some("boolean") => v.is_boolean(),
            Some("object") => v.is_object(),
            Some("array") => v.is_array(),
            Some("null") => v.is_null(),
            Some(t) => panic!("type {t}"),
        };
        if !typed {
            return fail("type");
        }
        match s["pattern"].as_str() {
            None => {}
            Some("^[0-9a-f]{64}$") => {
                let h = v.as_str().unwrap();
                if h.len() != 64 || !h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    return fail("hex");
                }
            }
            Some(p) => panic!("pattern {p}: teach this walker"),
        }
        if s["format"] == "uuid" && uuid::Uuid::parse_str(v.as_str().unwrap()).is_err() {
            return fail("uuid");
        }
        if s["minimum"]
            .as_f64()
            .is_some_and(|m| v.as_f64().unwrap() < m)
        {
            return fail("minimum");
        }
        for k in s["required"].as_array().into_iter().flatten() {
            if v.get(k.as_str().unwrap()).is_none() {
                return fail(&format!("required {k}"));
            }
        }
        if let (Some(props), Some(obj)) = (s["properties"].as_object(), v.as_object()) {
            for (k, sub) in props.iter().filter(|(k, _)| obj.contains_key(*k)) {
                check(root, sub, &obj[k], &format!("{at}.{k}"))?;
                seen.insert(k.clone());
            }
            if s["additionalProperties"] == false && obj.keys().any(|k| !props.contains_key(k)) {
                return fail("additional property");
            }
        }
        for (i, item) in v.as_array().into_iter().flatten().enumerate() {
            check(root, &s["items"], item, &format!("{at}[{i}]"))?;
        }
        if let Some(branches) = s["oneOf"].as_array() {
            let ok: Vec<_> = branches
                .iter()
                .filter_map(|b| check(root, b, v, at).ok())
                .collect();
            match ok.as_slice() {
                [one] => seen.extend(one.iter().cloned()),
                _ => return fail(&format!("{} oneOf branches match", ok.len())),
            }
        }
        let obj = v.as_object();
        if s["unevaluatedProperties"] == false
            && obj.is_some_and(|o| o.keys().any(|k| !seen.contains(k)))
        {
            return fail("unevaluated property");
        }
        Ok(seen)
    }

    /// The schema Talos copies accepts every fixture, in the same direction Rust parses it,
    /// and refuses an extra field on any of them.
    #[test]
    fn schema_accepts_each_fixture_in_its_direction() {
        let root = schema();
        let (talos, pc) = (&root["$defs"]["to_talos"], &root["$defs"]["to_pc"]);
        let count = |d: &Value| d["oneOf"].as_array().unwrap().len();
        assert_eq!((count(talos), count(pc)), (11, 16), "one branch per frame");
        for (name, value) in fixtures() {
            check(&root, &root, &value, &name).unwrap_or_else(|e| panic!("{e}"));
            let in_talos = check(&root, talos, &value, &name).is_ok();
            assert_ne!(in_talos, check(&root, pc, &value, &name).is_ok(), "{name}");
            assert_eq!(in_talos, round_trip::<ToTalos>(&value).is_some(), "{name}");
            let mut extra = value.clone();
            extra["x"] = 1.into();
            assert!(check(&root, &root, &extra, &name).is_err(), "{name}: extra");
        }
        all_required(&root);
    }

    /// The contract has no optional fields: every declared property is required.
    fn all_required(s: &Value) {
        match s {
            Value::Object(m) => {
                for k in m
                    .get("properties")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    let required = m.get("required").and_then(Value::as_array);
                    assert!(
                        required.is_some_and(|r| r.contains(&k.0.as_str().into())),
                        "{k:?}"
                    );
                }
                m.values().for_each(all_required);
            }
            Value::Array(a) => a.iter().for_each(all_required),
            _ => {}
        }
    }

    /// Fixtures show one value of each enum; the schema must list exactly Rust's values.
    #[test]
    fn schema_enums_match_rust() {
        let root = schema();
        let enum_of = |dir: &str, ty: &str, field: &str| {
            let branches = root["$defs"][dir]["oneOf"].as_array().unwrap();
            let branch = branches
                .iter()
                .find(|b| b["properties"]["type"]["const"] == ty);
            branch.unwrap()["properties"][field]["enum"].clone()
        };
        use serde_json::json;
        let role = json!([Role::Owner, Role::Admitted]);
        assert_eq!(enum_of("to_talos", "speaker", "role"), role);
        let kind = json!([HandoffKind::Money, HandoffKind::Card]);
        assert_eq!(enum_of("to_talos", "handoff", "kind"), kind);
        assert_eq!(
            enum_of("to_pc", "transcript", "dir"),
            json!([Dir::In, Dir::Out])
        );
        let notice = json!([NoticeKind::Cap80, NoticeKind::Cap100, NoticeKind::Offline]);
        assert_eq!(enum_of("to_pc", "notice", "kind"), notice);
    }

    /// The walker bites on drift the schema must catch, one fault per case.
    #[test]
    fn schema_refuses_bad_values() {
        let root = schema();
        let hex = "a".repeat(64);
        let bad = [
            r#"{"v":1,"type":"onset","pubkey":"Russell"}"#.to_owned(),
            r#"{"v":1,"type":"tool","id":"c","name":"n","args":"gh api -X DELETE"}"#.to_owned(),
            format!(
                r#"{{"v":1,"type":"speaker","span":1,"pubkey":"{hex}","role":"guest","echo":false,"t_ms":0}}"#
            ),
            r#"{"v":1,"type":"notice","kind":"cap_80","text":""}"#.to_owned(),
            r#"{"v":1,"type":"summary","text":"","read_channels":["general"]}"#.to_owned(),
            r#"{"v":1,"type":"hello"}"#.to_owned(),
            r#"{"v":1,"type":"ping","t":-1}"#.to_owned(),
            r#"{"v":2,"type":"ping","t":1}"#.to_owned(),
        ];
        for text in bad {
            let value: Value = serde_json::from_str(&text).unwrap();
            assert!(check(&root, &root, &value, &text).is_err(), "{text}");
        }
    }

    #[test]
    fn other_versions_are_refused() {
        assert!(parse::<ToPc>(r#"{"v":2,"type":"ping","t":1}"#).is_err());
        assert!(parse::<ToPc>(r#"{"type":"ping","t":1}"#).is_err());
        assert_eq!(
            parse::<ToPc>(r#"{"v":1,"type":"ping","t":1}"#),
            Ok(ToPc::Ping { t: 1 })
        );
    }
}
