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

use super::{Class, Factory, Health};

/// The class a saved `class=` slug came from, and the slug back as the
/// `'static` string a [`Failure`](super::Failure) carries. `None` for
/// anything this version does not name, which is not resumed at all: a
/// record it cannot reproduce faithfully is worse than none.
fn class_of(slug: &str) -> Option<(Class, &'static str)> {
    Some(match slug {
        "weekly_limit" => (Class::UsageLimit, "weekly_limit"),
        "session_limit" => (Class::UsageLimit, "session_limit"),
        "usage_limit" => (Class::UsageLimit, "usage_limit"),
        "spend_cap" => (Class::SpendCap, "spend_cap"),
        "rate_limit" => (Class::RateLimit, "rate_limit"),
        "sign_in" => (Class::SignIn, "sign_in"),
        "error" => (Class::Other, "error"),
        _ => return None,
    })
}

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
            // The incident, and whether the owner has already been texted
            // about it — ids and a boolean, nothing a person wrote. A
            // harness that restarts mid-outage reads these back so the
            // all-clear it owes still goes out (see
            // [`resume_from_saved_state`](Factory::resume_from_saved_state)).
            "incident": self.incident,
            "alerted": self.alerted.is_some(),
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

    /// Pick the outage back up where a restart left it.
    ///
    /// The owner's own fix for a lost sign-in is to sign in and restart the
    /// agent, so the restart lands *between* the Critical and the recovery —
    /// and `alerted` lived only in memory, so the all-clear was dropped:
    /// the owner's last word was that it was broken, and nothing ever said
    /// it came back. That is the exact failure the all-clear was added to
    /// prevent.
    ///
    /// So a harness that finds its own unexpired paused state comes up in
    /// it: the state, the failure behind it, when it began, its incident,
    /// and whether the owner was already texted. Nothing has fixed itself
    /// while the harness was down, and the panel and #factory are already
    /// saying this — so saying it too is what makes the next transition out
    /// of it a real one, with the all-clear on the end.
    ///
    /// A file past its `expires` says nothing (spec 4.1), and a state this
    /// version cannot name faithfully is ignored rather than guessed at. A
    /// Degraded incident leaves no paused state by design — the file is for
    /// a harness that has genuinely stopped — so a hand-off's text is the
    /// one case a restart still leaves without its all-clear.
    pub(crate) fn resume_from_saved_state(&mut self, now: DateTime<Utc>) {
        let Some(path) = self.paused_state_path() else {
            return;
        };
        let Some(saved) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .filter(|saved| saved["version"] == 1)
        else {
            return;
        };
        let at = |key: &str| {
            saved[key]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc))
        };
        // Past its expiry it is not evidence of anything any more.
        let Some(expires) = at("expires").filter(|expires| now < *expires) else {
            return;
        };
        let health = match saved["state"].as_str() {
            Some("DOWN") => Health::Down,
            Some("BLOCKED") => Health::Blocked,
            _ => return,
        };
        let Some((class, slug)) = saved["class"].as_str().and_then(class_of) else {
            return;
        };
        let Some(text) = saved["text"].as_str() else {
            return;
        };
        self.failure = Some(super::Failure {
            class,
            slug,
            text: super::truncate(text, super::TEXT_MAX),
            resets_at: at("resetsAt"),
        });
        self.health = health;
        self.since = at("since").or(Some(now));
        self.resumes = at("resumesAt");
        // The record this file was written beside is already in #factory and
        // still current, so nothing is re-posted now — but the next change
        // out of this state is due, which is what clears the file and stops
        // the panel saying "paused" after the agent is working again.
        self.reported = Some((health, expires));
        self.incident = saved["incident"]
            .as_str()
            .map(str::to_string)
            .or_else(|| Some(now.format("inc-%Y%m%d-%H%M%S").to_string()));
        if saved["alerted"] == true {
            // Already texted, so the owner is owed the all-clear and not a
            // second opening text for the state they already heard about.
            self.alerted = self.incident.clone();
            self.alerted_worst = Some(super::alert::severity(health));
        }
        tracing::warn!(
            class = slug,
            "resuming the outage this harness was in when it stopped"
        );
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

    /// The owner's own fix for a lost sign-in is to sign in and restart the
    /// agent — so the restart lands between the Critical and the recovery,
    /// which is where the all-clear used to be lost: `alerted` was memory
    /// only, so the new harness had nothing to be clear about and said
    /// nothing. The owner's last word stayed "it is broken".
    #[test]
    fn the_all_clear_survives_a_restart_mid_outage() {
        let bench = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 18, 20, 32);
        let lost = classify(
            &AcpError::agent_error(
                -32000,
                "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
            ),
            now,
        );
        let mut before = factory(bench.path());
        before.plan(0, &scope(), &lost, now);
        before.take_report(now);
        assert_eq!(before.take_alerts().len(), 1, "the owner was texted");
        let saved = read(bench.path()).expect("saved");
        assert_eq!(saved["alerted"], true);
        assert!(saved["incident"].as_str().is_some(), "{saved}");

        // The harness is restarted. It comes up in the outage it was in.
        let mut after = factory(bench.path());
        assert_eq!(after.health, Health::Operational, "before it reads it");
        after.resume_from_saved_state(now + chrono::Duration::minutes(2));
        assert_eq!(after.health, Health::Down);
        assert_eq!(after.presence(), "away");
        assert_eq!(after.failure.as_ref().map(|f| f.slug), Some("sign_in"));
        assert!(after.take_alerts().is_empty(), "no second opening text");

        // The owner signed it in before the restart, so it answers again —
        // and this time the all-clear goes out.
        let later = now + chrono::Duration::minutes(5);
        for _ in 0..crate::factory::ANSWERS_TO_RECOVER {
            after.on_success(0, later);
        }
        assert_eq!(after.health, Health::Operational);
        assert_eq!(
            after.take_alerts(),
            ["Buzz: all clear — Reset Worker is working normally again."]
        );
        after.take_report(later);
        assert!(
            read(bench.path()).is_none(),
            "and the panel stops saying it"
        );
    }

    /// What is not resumed: a state past its own expiry (spec 4.1), a class
    /// this version cannot name faithfully, and a file that is not there.
    /// None of them is an error, and none of them takes a working harness
    /// down.
    #[test]
    fn a_stale_or_unreadable_saved_state_is_not_resumed() {
        let bench = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 18, 20, 32);
        let weekly = classify(
            &AcpError::agent_error(-32603, "Internal error: You've hit your weekly limit"),
            now,
        );
        let mut before = factory(bench.path());
        before.plan(0, &scope(), &weekly, now);
        before.take_report(now);
        let path = bench.path().join(PAUSED_DIR).join(format!("{PUBKEY}.json"));

        // Long past its expiry: evidence of nothing.
        let mut after = factory(bench.path());
        after.resume_from_saved_state(now + chrono::Duration::days(2));
        assert_eq!(after.health, Health::Operational);
        assert!(after.failure.is_none());

        // A class this version does not know, and junk.
        for bad in [
            serde_json::json!({"version": 1, "state": "BLOCKED", "class": "from_the_future",
                               "text": "x", "expires": "2026-10-18T21:00:00Z"})
            .to_string(),
            serde_json::json!({"version": 2, "state": "BLOCKED", "class": "weekly_limit",
                               "text": "x", "expires": "2026-10-18T21:00:00Z"})
            .to_string(),
            "{".to_string(),
            String::new(),
        ] {
            std::fs::write(&path, &bad).unwrap();
            let mut after = factory(bench.path());
            after.resume_from_saved_state(now);
            assert_eq!(after.health, Health::Operational, "{bad}");
            assert!(after.failure.is_none(), "{bad}");
        }

        // And no file at all is the normal case.
        std::fs::remove_file(&path).unwrap();
        let mut after = factory(bench.path());
        after.resume_from_saved_state(now);
        assert_eq!(after.health, Health::Operational);
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
