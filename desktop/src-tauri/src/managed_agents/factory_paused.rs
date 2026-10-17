//! The harness's paused state, read for the agent panel (spec 4.3, M1 step 5).
//!
//! A harness parked behind a usage limit, a spend cap or a lost sign-in keeps
//! running, so it never exits with an error for `last_error` to hold, and the
//! observer event window the panel otherwise derives this from is capped,
//! trimmed and gone after a restart. The harness therefore writes one small
//! record per agent beside its Bench cards (`buzz-acp`'s
//! `factory::paused_state`) and this reads it back.
//!
//! Every record carries `expires`; anything past that is treated as unknown
//! (spec 4.1), so a file left behind by a harness that has since stopped
//! cannot keep the panel claiming an outage.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The folder under the Bench root holding one record per agent pubkey.
/// Mirrors `buzz-acp`'s `factory::paused_state::PAUSED_DIR`.
const PAUSED_DIR: &str = "state";

/// Why an agent's work is paused, in the provider's own words.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactoryPaused {
    /// The failure class slug: `weekly_limit`, `session_limit`, `usage_limit`,
    /// `spend_cap`, `sign_in`.
    pub class: String,
    /// The provider's words, already scrubbed and truncated by the harness.
    pub text: String,
    /// `Claude`, `Codex`, or the adapter's own name.
    pub provider: String,
    /// `BLOCKED` (waiting on a limit) or `DOWN` (the sign-in is gone).
    pub state: String,
    /// When the limit lifts, if the provider said.
    #[serde(default, rename = "resetsAt", skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    /// When the harness next tries.
    #[serde(default, rename = "resumesAt", skip_serializing_if = "Option::is_none")]
    pub resumes_at: Option<String>,
    /// When this state began.
    pub since: String,
    /// After this, a reader treats the record as unknown.
    pub expires: String,
}

/// The paused state for `pubkey`, or `None` when the agent is not paused, no
/// record was ever written, or the one on disk has expired.
pub(crate) fn read(pubkey: &str) -> Option<FactoryPaused> {
    read_from(&path(pubkey)?, chrono::Utc::now())
}

/// `<bench root>/state/<pubkey>.json`. `None` for a pubkey that is not plain
/// 64-character hex: a file name must never come from anything that could
/// carry a path separator.
fn path(pubkey: &str) -> Option<PathBuf> {
    if pubkey.len() != 64 || !pubkey.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(
        crate::bench::root_dir()
            .ok()?
            .join(PAUSED_DIR)
            .join(format!("{pubkey}.json")),
    )
}

fn read_from(path: &std::path::Path, now: chrono::DateTime<chrono::Utc>) -> Option<FactoryPaused> {
    let paused: FactoryPaused = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    // An unparsable or past `expires` is not current state. Unparsable is
    // the stricter read on purpose: a record whose freshness cannot be
    // judged is not evidence that an agent is still stopped.
    let expires = chrono::DateTime::parse_from_rfc3339(&paused.expires).ok()?;
    (expires.with_timezone(&chrono::Utc) > now).then_some(paused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn write(dir: &std::path::Path, expires: &str) -> PathBuf {
        let path = dir.join("state.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 1,
                "class": "weekly_limit",
                "text": "You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)",
                "provider": "Claude",
                "state": "BLOCKED",
                "resetsAt": "2026-10-19T00:00:00Z",
                "resumesAt": "2026-10-19T00:02:00Z",
                "since": "2026-10-16T16:52:00Z",
                "expires": expires,
            })
            .to_string(),
        )
        .unwrap();
        path
    }

    /// The panel's whole point: on Saturday morning, long after the event
    /// window that used to carry this was trimmed, the limit is still read.
    #[test]
    fn a_current_record_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "2026-10-19T00:07:00Z");
        let paused = read_from(&path, at(2026, 10, 17, 9, 0)).expect("still current");
        assert_eq!(paused.class, "weekly_limit");
        assert_eq!(paused.state, "BLOCKED");
        assert_eq!(paused.provider, "Claude");
        assert_eq!(paused.resets_at.as_deref(), Some("2026-10-19T00:00:00Z"));
        assert_eq!(paused.resumes_at.as_deref(), Some("2026-10-19T00:02:00Z"));
    }

    /// Silence must never read as current state (spec 2.3): a record left by
    /// a harness that has since stopped expires on its own.
    #[test]
    fn an_expired_or_unreadable_record_is_not_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "2026-10-19T00:07:00Z");
        assert!(
            read_from(&path, at(2026, 10, 19, 0, 8)).is_none(),
            "expired"
        );

        let no_expiry = write(dir.path(), "not a time");
        assert!(read_from(&no_expiry, at(2026, 10, 17, 9, 0)).is_none());

        let junk = dir.path().join("junk.json");
        std::fs::write(&junk, "{").unwrap();
        assert!(read_from(&junk, at(2026, 10, 17, 9, 0)).is_none());
        assert!(read_from(&dir.path().join("missing.json"), at(2026, 10, 17, 9, 0)).is_none());
    }

    /// The pubkey names a file, so only plain hex is ever used as one.
    #[test]
    fn only_a_plain_hex_pubkey_names_a_file() {
        assert!(path(&"ab".repeat(32)).is_some());
        for bad in ["", "../escape", &"z".repeat(64), &"ab".repeat(31)] {
            assert!(path(bad).is_none(), "{bad:?}");
        }
    }
}
