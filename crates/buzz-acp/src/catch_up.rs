//! Mentions sent while the harness was down.
//!
//! A cold start used to subscribe from its own start time, so an @mention
//! sent during a reboot, an app restart or a crash sat on the relay and was
//! never asked for again. The harness now replays a bounded window of
//! mentions on startup and keeps one small file per agent and relay naming
//! the mentions whose turn finished or that the author gate refused, so a
//! restart never answers one twice or reopens a refusal.
//!
//! The file sits beside the Bench root (`~/.dreamforge/agent-state`), is
//! written beside its name and renamed into place, and carries a `floor`:
//! the first boot with this ledger. Nothing older than that is replayed, so
//! the upgrade itself cannot re-answer mentions handled before it. Each kind
//! has its own floor, the first boot that listened for it, so a release that
//! adds a kind does not replay a day of it as new work.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Extra time a finished id is kept past the window, so a mention sent just
/// inside the window is still known as done when it is replayed.
const PRUNE_MARGIN_SECS: u64 = 3600;

/// The kinds a ledger saved before kinds were recorded listened for: the
/// default mention kinds of that release.
const UNRECORDED_KINDS: [u32; 4] = [
    buzz_core::kind::KIND_STREAM_MESSAGE,
    buzz_core::kind::KIND_STREAM_MESSAGE_EDIT,
    buzz_core::kind::KIND_WORKFLOW_APPROVAL_REQUESTED,
    buzz_core::kind::KIND_STREAM_REMINDER,
];

/// How long a refused mention waits in memory before the ledger is saved for
/// it, so a noisy sender cannot force a write per message.
pub(crate) const REFUSED_SAVE_DELAY: Duration = Duration::from_secs(10);

/// The catch-up ledger, or nothing when catch-up is off.
#[derive(Debug, Default)]
pub(crate) struct CatchUp(Option<Arc<Mutex<Ledger>>>);

#[derive(Debug)]
struct Ledger {
    path: PathBuf,
    window: u64,
    floor: u64,
    /// Finished event id (hex) and its `created_at`.
    done: HashMap<String, u64>,
    /// Each kind listened for, and the first boot that listened for it.
    kinds: HashMap<u32, u64>,
    /// Ids added since the last save, with a save already scheduled.
    unsaved: bool,
}

impl CatchUp {
    /// Load the ledger for this agent and relay, or start one whose floor is
    /// `now` when it is missing or unreadable. `window == 0` turns catch-up
    /// off and touches nothing on disk.
    pub(crate) fn open(
        dir: &Path,
        pubkey_hex: &str,
        relay_url: &str,
        window: u64,
        now: u64,
    ) -> Self {
        if window == 0 {
            return Self::default();
        }
        let path = dir.join(format!("{pubkey_hex}-{}.json", relay_slug(relay_url)));
        let saved = std::fs::read_to_string(&path).ok().map(|raw| {
            serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .filter(|v| v["version"] == 1)
        });
        let ledger = match saved {
            Some(Some(saved)) => Ledger {
                path,
                window,
                floor: saved["floor"].as_u64().unwrap_or(now),
                done: saved["done"]
                    .as_object()
                    .map(|done| {
                        done.iter()
                            .filter_map(|(id, ts)| Some((id.clone(), ts.as_u64()?)))
                            .collect()
                    })
                    .unwrap_or_default(),
                kinds: match saved["kinds"].as_object() {
                    Some(kinds) => kinds
                        .iter()
                        .filter_map(|(kind, ts)| Some((kind.parse().ok()?, ts.as_u64()?)))
                        .collect(),
                    None => {
                        let floor = saved["floor"].as_u64().unwrap_or(now);
                        UNRECORDED_KINDS.iter().map(|&kind| (kind, floor)).collect()
                    }
                },
                unsaved: false,
            },
            missing_or_corrupt => {
                // Replaying from an unknown point could re-answer anything,
                // so a fresh ledger starts at now and replays nothing.
                if missing_or_corrupt.is_some() {
                    tracing::warn!(path = %path.display(), "catch-up ledger unreadable — starting a new one; nothing sent before now is replayed");
                } else {
                    tracing::info!(path = %path.display(), "starting the catch-up ledger; mentions sent before now are not replayed");
                }
                let mut ledger = Ledger {
                    path,
                    window,
                    floor: now,
                    done: HashMap::new(),
                    kinds: HashMap::new(),
                    unsaved: false,
                };
                ledger.save();
                ledger
            }
        };
        Self(Some(Arc::new(Mutex::new(ledger))))
    }

    /// Where a cold start replays mentions from: the window, never before
    /// the floor. `None` when catch-up is off.
    pub(crate) fn since(&self, now: u64) -> Option<u64> {
        self.0.as_ref().map(|ledger| {
            let ledger = lock(ledger);
            now.saturating_sub(ledger.window).max(ledger.floor)
        })
    }

    /// Record that this boot listens for `kinds`. A kind not heard before
    /// gets `now` as its floor.
    pub(crate) fn listen_for(&self, kinds: &[u32], now: u64) {
        let Some(ledger) = self.0.as_ref() else {
            return;
        };
        let mut ledger = lock(ledger);
        let before = ledger.kinds.len();
        for &kind in kinds {
            ledger.kinds.entry(kind).or_insert(now);
        }
        if ledger.kinds.len() > before {
            ledger.save();
        }
    }

    /// Whether the harness already listened for `kind` when an event was
    /// sent at `sent`. A replayed event of a kind first heard later is not
    /// backlog: nothing would have answered it then. A kind never recorded
    /// is not held back.
    pub(crate) fn heard_when_sent(&self, kind: u32, sent: u64) -> bool {
        self.0.as_ref().is_none_or(|ledger| {
            lock(ledger)
                .kinds
                .get(&kind)
                .is_none_or(|&floor| floor <= sent)
        })
    }

    /// Whether this event's turn already finished, before or since a restart.
    pub(crate) fn is_done(&self, event_id_hex: &str) -> bool {
        self.0
            .as_ref()
            .is_some_and(|ledger| lock(ledger).done.contains_key(event_id_hex))
    }

    /// Mark these `(event id, created_at)` pairs finished and save. A failed
    /// write only warns: the cost is a possible repeat answer after a restart.
    pub(crate) fn record(&self, finished: &[(String, u64)], now: u64) {
        let Some(ledger) = self.0.as_ref().filter(|_| !finished.is_empty()) else {
            return;
        };
        let mut ledger = lock(ledger);
        ledger.done.extend(finished.iter().cloned());
        // Measured from the newest mention, not the local clock alone: a clock
        // that jumps ahead must not prune what the corrected clock replays.
        let newest = ledger.done.values().copied().max().unwrap_or(now);
        let keep_from = now
            .min(newest)
            .saturating_sub(ledger.window + PRUNE_MARGIN_SECS);
        ledger.done.retain(|_, ts| *ts >= keep_from);
        ledger.save();
    }

    /// Mark a mention the author gate refused as done, so the policy it was
    /// sent under decides it: a respond-to change, which restarts the agent,
    /// must not answer a day of refused requests. Saved with the next turn, or
    /// [`REFUSED_SAVE_DELAY`] from now, whichever comes first. Needs a Tokio
    /// runtime.
    pub(crate) fn note_refused(&self, event_id_hex: String, created_at: u64) {
        let Some(ledger) = self.0.as_ref() else {
            return;
        };
        {
            let mut guard = lock(ledger);
            guard.done.insert(event_id_hex, created_at);
            if std::mem::replace(&mut guard.unsaved, true) {
                return;
            }
        }
        let ledger = Arc::clone(ledger);
        tokio::spawn(async move {
            tokio::time::sleep(REFUSED_SAVE_DELAY).await;
            let mut ledger = lock(&ledger);
            if ledger.unsaved {
                ledger.save();
            }
        });
    }
}

impl Ledger {
    fn save(&mut self) {
        self.unsaved = false;
        let record = serde_json::json!({
            "version": 1,
            "floor": self.floor,
            "done": self.done,
            "kinds": self.kinds,
        });
        let written = self
            .path
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|()| {
                let part = self.path.with_extension("json.part");
                let mut file = std::fs::File::create(&part)?;
                file.write_all(record.to_string().as_bytes())?;
                // On disk before the rename, or a power cut can leave an empty
                // ledger that resets the floor.
                file.sync_all()?;
                drop(file);
                std::fs::rename(&part, &self.path)
            });
        if let Err(e) = written {
            tracing::warn!(path = %self.path.display(), "catch-up ledger not saved: {e}");
        }
    }
}

fn lock(ledger: &Mutex<Ledger>) -> std::sync::MutexGuard<'_, Ledger> {
    ledger
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `wss://relay.example.com:443/x` → `relay.example.com-443`: the relay's
/// host as a file-name part, so each relay keeps its own ledger and nothing
/// in the URL can carry a path separator into the name.
fn relay_slug(relay_url: &str) -> String {
    let rest = relay_url
        .split_once("://")
        .map_or(relay_url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Never a credential in a file name.
    let host = authority.rsplit('@').next().unwrap_or_default();
    host.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PK: &str = "ab";
    const RELAY: &str = "wss://relay.example.com";
    const NOW: u64 = 1_700_000_000;
    const DAY: u64 = 86_400;

    fn file(dir: &Path, relay: &str) -> PathBuf {
        dir.join(format!("{PK}-{}.json", relay_slug(relay)))
    }

    #[test]
    fn first_open_writes_floor_and_replays_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        assert_eq!(catch_up.since(NOW), Some(NOW));
        assert!(
            file(dir.path(), RELAY).exists(),
            "the floor must survive the next restart"
        );
    }

    #[test]
    fn since_is_window_bounded_then_floor_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        // Down for an hour: replay back to the floor, not a full day.
        assert_eq!(catch_up.since(NOW + 3600), Some(NOW));
        // Down for three days: replay only the last day.
        assert_eq!(catch_up.since(NOW + 3 * DAY), Some(NOW + 2 * DAY));
    }

    #[test]
    fn recorded_ids_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        catch_up.record(&[("e1".into(), NOW + 10)], NOW + 20);
        drop(catch_up);

        let reopened = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW + 600);
        assert!(
            reopened.is_done("e1"),
            "a finished mention must not be answered again"
        );
        assert!(!reopened.is_done("e2"));
        assert_eq!(
            reopened.since(NOW + 600),
            Some(NOW),
            "the floor is kept, not reset"
        );
    }

    #[test]
    fn prune_keeps_window_plus_margin() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        let later = NOW + 3 * DAY;
        catch_up.record(
            &[
                ("fresh".into(), later),
                ("old".into(), later - DAY - PRUNE_MARGIN_SECS - 1),
            ],
            later,
        );
        catch_up.record(&[("edge".into(), later - DAY - PRUNE_MARGIN_SECS)], later);
        assert!(!catch_up.is_done("old"));
        assert!(catch_up.is_done("edge"));
        assert!(catch_up.is_done("fresh"));
    }

    #[test]
    fn clock_jumping_ahead_does_not_prune_recent_mentions() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        catch_up.record(&[("e1".into(), NOW + 10)], NOW + 20);
        // The PC boots a week ahead and one turn finishes.
        catch_up.record(&[("e2".into(), NOW + 30)], NOW + 7 * DAY);
        drop(catch_up);
        // NTP corrects the clock; the restart must not answer e1 again.
        let reopened = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW + 60);
        assert!(reopened.is_done("e1"));
        assert!(reopened.is_done("e2"));
    }

    #[tokio::test(start_paused = true)]
    async fn refused_mention_is_saved_after_a_delay_or_with_the_next_turn() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        let saved_ids = || {
            let raw = std::fs::read_to_string(file(dir.path(), RELAY)).unwrap();
            let saved: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let mut ids: Vec<String> = saved["done"].as_object().unwrap().keys().cloned().collect();
            ids.sort();
            ids
        };

        catch_up.note_refused("r1".into(), NOW + 1);
        catch_up.note_refused("r2".into(), NOW + 2);
        assert!(catch_up.is_done("r1"), "refused now, so not answered later");
        assert!(
            saved_ids().is_empty(),
            "a noisy sender does not force a write per message"
        );
        tokio::time::sleep(REFUSED_SAVE_DELAY + Duration::from_secs(1)).await;
        assert_eq!(
            saved_ids(),
            ["r1", "r2"],
            "one delayed save covers the burst"
        );

        catch_up.note_refused("r3".into(), NOW + 3);
        catch_up.record(&[("e1".into(), NOW + 4)], NOW + 5);
        assert_eq!(
            saved_ids(),
            ["e1", "r1", "r2", "r3"],
            "the next turn saves it too"
        );
    }

    /// A release that adds a kind must not replay a day of it as new work.
    /// A ledger saved before kinds were recorded keeps replaying its old
    /// kinds; a new kind replays only from the boot that first heard it.
    #[test]
    fn a_new_kind_replays_only_from_the_boot_that_first_heard_it() {
        const FORUM_COMMENT: u32 = buzz_core::kind::KIND_FORUM_COMMENT;
        let dir = tempfile::tempdir().unwrap();
        let unrecorded = serde_json::json!({ "version": 1, "floor": NOW - 2 * DAY, "done": {} });
        std::fs::write(file(dir.path(), RELAY), unrecorded.to_string()).unwrap();
        let hour_ago = NOW - 3600;

        let upgraded = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        upgraded.listen_for(&[9, FORUM_COMMENT], NOW);
        assert!(upgraded.heard_when_sent(9, hour_ago));
        assert!(!upgraded.heard_when_sent(FORUM_COMMENT, hour_ago));
        drop(upgraded);

        let restarted = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW + 7200);
        restarted.listen_for(&[9, FORUM_COMMENT], NOW + 7200);
        assert!(
            !restarted.heard_when_sent(FORUM_COMMENT, hour_ago),
            "the next restart remembers when the kind was first heard"
        );
        assert!(restarted.heard_when_sent(FORUM_COMMENT, NOW + 60));
    }

    #[test]
    fn corrupt_file_resets_floor_instead_of_replaying() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(file(dir.path(), RELAY), "{not json").unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        assert_eq!(catch_up.since(NOW), Some(NOW));
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file(dir.path(), RELAY)).unwrap())
                .unwrap();
        assert_eq!(saved["floor"], NOW);
    }

    #[test]
    fn window_zero_is_disabled_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, 0, NOW);
        catch_up.record(&[("e1".into(), NOW)], NOW);
        assert_eq!(catch_up.since(NOW), None);
        assert!(!catch_up.is_done("e1"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn no_part_file_left_after_record() {
        let dir = tempfile::tempdir().unwrap();
        let catch_up = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        catch_up.record(&[("e1".into(), NOW)], NOW);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![format!("{PK}-relay.example.com.json")]);
    }

    #[test]
    fn separate_files_per_relay() {
        let dir = tempfile::tempdir().unwrap();
        let a = CatchUp::open(dir.path(), PK, RELAY, DAY, NOW);
        a.record(&[("e1".into(), NOW)], NOW);
        let b = CatchUp::open(dir.path(), PK, "ws://localhost:3000/", DAY, NOW);
        assert!(!b.is_done("e1"));
        assert!(file(dir.path(), "ws://localhost:3000").exists());
        assert_eq!(relay_slug("ws://localhost:3000/"), "localhost-3000");
        assert_eq!(relay_slug("wss://a\\b/../c"), "a-b");
        assert_eq!(
            relay_slug("wss://user:secret@relay.example.com"),
            "relay.example.com"
        );
    }
}
