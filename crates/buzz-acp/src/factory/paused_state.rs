//! The paused state, written where the desktop can read it (spec 4.3, M1
//! step 5).
//!
//! The agent panel's "Paused: Claude weekly limit · resets Sun 7 pm" is
//! otherwise derived from the observer event window, which is capped and
//! trimmed and does not survive a restart — so for most of a multi-day
//! outage the panel went blank, which is the one surface M1 step 5 exists to
//! fill. `ManagedAgent.last_error` cannot cover it either: a parked harness
//! keeps running, so it never exits with an error to record.
//!
//! So the harness writes one small file per agent beside its Bench cards,
//! and the desktop reads it. Every record carries `expires`, and a reader
//! treats anything past that as unknown (spec 4.1), so a stale file left by
//! a harness that has since stopped cannot keep claiming an outage.

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use super::{Factory, Health};

/// The folder under the Bench root that holds the paused-state files.
pub(crate) const PAUSED_DIR: &str = "state";

impl Factory {
    /// Save the paused state, or clear it when nothing is paused.
    ///
    /// Called from [`take_report`](Factory::take_report), which is the one
    /// place that knows the state changed and has computed its `expires`, so
    /// the file and the #factory record always say the same thing.
    pub(super) fn save_paused_state(&self, expires: DateTime<Utc>) {
        let Some(path) = self.paused_state_path() else {
            return;
        };
        let Some(failure) = self
            .failure
            .as_ref()
            .filter(|_| matches!(self.health, Health::Blocked | Health::Down))
        else {
            // Moving again: the panel must stop saying it is paused. A
            // missing file is the normal case, so its absence is not an
            // error.
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("paused state not cleared: {e}");
                }
            }
            return;
        };
        let iso = |t: DateTime<Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let record = serde_json::json!({
            "version": 1,
            // The class slug and the provider's own (already scrubbed and
            // truncated) words: the same pair the STATE record carries.
            "class": failure.slug,
            "text": failure.text,
            "provider": self.provider_name(),
            "state": if self.health == Health::Down { "DOWN" } else { "BLOCKED" },
            "resetsAt": failure.resets_at.map(iso),
            "resumesAt": self.resumes.map(iso),
            "since": iso(self.since.unwrap_or(expires)),
            "expires": iso(expires),
        });
        // Written beside its name and renamed into place, so the desktop
        // never reads half a file — the same discipline as a Bench card.
        let written = path
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|()| {
                let part = path.with_extension("json.part");
                std::fs::write(&part, record.to_string())?;
                std::fs::rename(&part, &path)
            });
        match written {
            Ok(()) => tracing::debug!(class = failure.slug, "paused state saved for the desktop"),
            Err(e) => tracing::warn!("paused state not saved: {e}"),
        }
    }

    /// `<bench>/state/<pubkey>.json`, the path the desktop derives too.
    /// `None` without a bench folder, and for a pubkey that is not plain
    /// 64-character hex — a file name must never come from anything that
    /// could carry a path separator.
    fn paused_state_path(&self) -> Option<PathBuf> {
        let pubkey = &self.reporting.pubkey;
        if pubkey.len() != 64 || !pubkey.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(
            self.reporting
                .bench
                .as_ref()?
                .join(PAUSED_DIR)
                .join(format!("{pubkey}.json")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpError;
    use crate::factory::{classify, Reporting};
    use crate::scope::SessionScope;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn scope() -> SessionScope {
        SessionScope::Conversation {
            channel_id: uuid::Uuid::from_u128(7),
        }
    }

    const PUBKEY: &str = "ade9d0e2ade9d0e2ade9d0e2ade9d0e2ade9d0e2ade9d0e2ade9d0e2ade9d0e2";

    fn factory(bench: &std::path::Path) -> Factory {
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting = Reporting {
            agent: "Reset Worker".to_string(),
            short: PUBKEY[..8].to_string(),
            pubkey: PUBKEY.to_string(),
            bench: Some(bench.to_path_buf()),
            channel: None,
        };
        f
    }

    fn read(bench: &std::path::Path) -> Option<serde_json::Value> {
        let path = bench.join(PAUSED_DIR).join(format!("{PUBKEY}.json"));
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    /// The weekly limit of 16 Oct, as the desktop panel reads it two days
    /// later — from a file, not from an event window that has long since
    /// been trimmed.
    #[test]
    fn a_parked_state_is_saved_where_the_desktop_reads_it() {
        let bench = tempfile::tempdir().unwrap();
        let state = bench.path().join(PAUSED_DIR);
        assert!(!state.exists(), "the folder is made on demand");
        let mut f = factory(bench.path());
        let now = at(2026, 10, 16, 16, 52);
        let weekly = classify(
            &AcpError::agent_error(
                -32603,
                "Internal error: You've hit your weekly limit · resets Oct 18, 7pm \
                 (America/Chicago)",
            ),
            now,
        );
        f.plan(0, &scope(), &weekly, now);
        assert!(f.take_report(now).is_some(), "the state changed");

        let saved = read(bench.path()).expect("a paused state for the panel");
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["state"], "BLOCKED");
        assert_eq!(saved["class"], "weekly_limit");
        assert_eq!(saved["provider"], "Claude");
        assert_eq!(
            saved["text"],
            "You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"
        );
        assert_eq!(saved["resetsAt"], "2026-10-19T00:00:00Z");
        assert_eq!(saved["resumesAt"], "2026-10-19T00:02:00Z");
        assert_eq!(saved["since"], "2026-10-16T16:52:00Z");
        // Past `expires` a reader treats it as unknown (spec 4.1), so the
        // record outlives the next try and no longer.
        assert_eq!(saved["expires"], "2026-10-19T00:07:00Z");
        // Nothing but ids, names and the provider's own words.
        let text = saved.to_string();
        for secret in ["Users", "/home/", "sk-", ".claude"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }

    /// A lost sign-in reads as DOWN, and the file goes as soon as the agent
    /// is working again: a panel that keeps saying "Paused" after recovery
    /// is its own kind of lie.
    #[test]
    fn the_saved_state_is_cleared_once_the_agent_recovers() {
        let bench = tempfile::tempdir().unwrap();
        let mut f = factory(bench.path());
        let now = at(2026, 10, 17, 13, 15);
        let lost = classify(
            &AcpError::agent_error(
                -32000,
                "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
            ),
            now,
        );
        f.plan(0, &scope(), &lost, now);
        f.take_report(now);
        assert_eq!(read(bench.path()).expect("saved")["state"], "DOWN");

        // The first real answer is Recovering, which is no longer paused.
        f.on_success(0, now);
        f.take_report(now);
        assert!(
            read(bench.path()).is_none(),
            "the panel stops saying paused"
        );

        // Clearing an already-missing file is not an error.
        f.on_success(0, now);
        f.take_report(now);
    }

    /// A rate limit only slows the work, so the panel says nothing: the file
    /// is for a harness that has genuinely stopped.
    #[test]
    fn a_degraded_state_is_not_saved() {
        let bench = tempfile::tempdir().unwrap();
        let mut f = factory(bench.path());
        let now = at(2026, 10, 17, 13, 15);
        let rate = classify(
            &AcpError::agent_error(-32603, "Internal error: API Error: 429 too many requests"),
            now,
        );
        f.plan(0, &scope(), &rate, now);
        f.take_report(now);
        assert!(read(bench.path()).is_none());
    }

    /// Without a bench folder, or with a pubkey that is not plain hex,
    /// nothing is written and nothing fails. A file name never comes from
    /// something that could carry a path separator.
    #[test]
    fn nothing_is_written_without_a_folder_or_a_plain_pubkey() {
        let now = at(2026, 10, 17, 13, 15);
        let weekly = classify(
            &AcpError::agent_error(-32603, "Internal error: You've hit your weekly limit"),
            now,
        );
        let mut f = Factory::new(None, "claude-agent-acp");
        f.plan(0, &scope(), &weekly, now);
        f.take_report(now);
        assert!(f.paused_state_path().is_none(), "no bench folder");

        let bench = tempfile::tempdir().unwrap();
        for bad in ["", "../escape", "ADE9D0E2/..", &"z".repeat(64)] {
            let mut f = factory(bench.path());
            f.reporting.pubkey = bad.to_string();
            assert!(f.paused_state_path().is_none(), "{bad:?}");
            f.plan(0, &scope(), &weekly, now);
            f.take_report(now);
        }
        assert!(!bench.path().join(PAUSED_DIR).exists());
    }
}
