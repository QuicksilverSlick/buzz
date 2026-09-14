//! The Bench: a read-only owner dashboard as a private #bench channel, written
//! only by the Bench service with its own key. `item` is the trust boundary
//! for drop files; this file is the service: constants and the debug fence,
//! paths, state.json, writer key custody, the monotonic clock, recovery from
//! the writer's own events and inbox intake. The relay half lands next.
#![allow(dead_code)] // until run() is wired (build-order step 4)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use nostr::ToBech32;
use serde::{Deserialize, Serialize};

use crate::secret_store::SecretStore;

mod item;

pub(crate) const RELAY_WS: &str = "wss://greymatterllc.communities.buzz.xyz";
/// Generated once, never changed: the v5 namespace of every #bench id.
const BENCH_NS: uuid::Uuid = uuid::uuid!("3246615b-a3bb-4d70-9497-2d74aad28857");
/// Outside the buzz-desktop* family, so a Buzz reset never touches it.
const KEYRING_SERVICE: &str = if cfg!(debug_assertions) {
    "dreamforge-bench-dev"
} else {
    "dreamforge-bench"
};
const WRITER_KEY: &str = "writer";
const ABOUT: &str = "Dreamforge owner dashboard";
const POLL: Duration = Duration::from_secs(2);
const TAG_TTL_SECS: u64 = 3600;
const BOARD_MAX_AGE_SECS: u64 = 86_400;
const MAX_CARDS: usize = 30;
/// The relay's HTTP budget is 300/min; a 429 gates the whole desktop.
const MAX_POSTS_PER_TICK: usize = 4;
/// The relay clamps created_at at +-900 s.
const MAX_CLOCK_LEAD_SECS: u64 = 600;
const MAX_DROP_BYTES: u64 = 64 * 1024;
const RECOVER_LIMIT: u32 = 500;
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(60);

/// The debug fence: a debug build may only ever reach a loopback relay.
pub(crate) fn relay_ws_from(env: Option<&str>) -> Option<String> {
    if !cfg!(debug_assertions) {
        return Some(RELAY_WS.to_string());
    }
    env.filter(|u| u.starts_with("ws://localhost") || u.starts_with("ws://127.0.0.1"))
        .map(str::to_string)
}

/// `~/.dreamforge/bench` (debug: `~/.dreamforge-dev/bench`): outside the app
/// data an uninstall wipes and outside `~/.buzz` a boot reset removes.
fn root_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("no home directory")?;
    let nest = if cfg!(debug_assertions) {
        ".dreamforge-dev"
    } else {
        ".dreamforge"
    };
    Ok(home.join(nest).join("bench"))
}

/// One #bench per owner. The relay is not in the name, so a debug channel
/// matches the archive guard too.
pub(crate) fn channel_id(owner_hex: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&BENCH_NS, format!("bench:v1:{owner_hex}").as_bytes())
}

pub(crate) fn is_bench_channel(owner_hex: &str, scope_value: &str) -> bool {
    channel_id(owner_hex)
        .to_string()
        .eq_ignore_ascii_case(scope_value.trim())
}

/// A relay event the Bench posted and still owns (a card or the board).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Posted {
    pub event_id: String,
    pub created_at: u64,
    pub hash: String,
    /// Option seeds landed so far.
    #[serde(default)]
    pub seeded: usize,
}

/// How the relay admitted the writer key: learned from its first request.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
enum Admission {
    #[default]
    Unknown,
    Open,
    ViaOwner,
    Denied,
}

/// Everything the service remembers between ticks. Never a secret.
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct BenchState {
    version: u32,
    relay: String,
    writer_pubkey: String,
    owner_pubkey: String,
    channel_id: String,
    admission: Admission,
    profile_published: bool,
    channel_created: bool,
    members_added: Vec<String>,
    /// Floor of the monotonic clock for writer events.
    last_created_at: u64,
    board: Option<Posted>,
    board_dirty: bool,
    needs_rebuild: bool,
    items: BTreeMap<String, item::Item>,
    orphan_cards: BTreeMap<String, Posted>,
    strays: Vec<String>,
    last_tick: String,
    last_error: Option<String>,
}

/// state.json, else the previous snapshot, else fresh (which forces a rebuild
/// from the relay). The bool is `fresh`.
fn load_state(root: &Path) -> (BenchState, bool) {
    for name in ["state.json", "state.prev.json"] {
        let parsed = std::fs::read_to_string(root.join(name))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        if let Some(state) = parsed {
            return (state, false);
        }
    }
    let fresh = BenchState {
        needs_rebuild: true,
        ..Default::default()
    };
    (fresh, true)
}

/// Write state.json only when the snapshot changed, keeping the bytes it
/// replaces in state.prev.json.
fn save_state(root: &Path, s: &BenchState, last_written: &mut String) -> Result<(), String> {
    let text = serde_json::to_string_pretty(s).map_err(|e| format!("serialize state: {e}"))?;
    if text == *last_written {
        return Ok(());
    }
    let path = root.join("state.json");
    match std::fs::copy(&path, root.join("state.prev.json")) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("keep previous state: {e}")),
    }
    crate::managed_agents::atomic_write_json_restricted(&path, text.as_bytes())?;
    *last_written = text;
    Ok(())
}

/// The writer's own key, minted once into the OS keyring under its own
/// service. Any keyring error is fatal for the service: no file fallback,
/// never on AppState.
fn writer_keys() -> Result<nostr::Keys, String> {
    static STORE: OnceLock<SecretStore> = OnceLock::new();
    let store = STORE.get_or_init(|| SecretStore::keyring(KEYRING_SERVICE));
    if let Some(nsec) = store.load(WRITER_KEY)? {
        return nostr::Keys::parse(nsec.trim()).map_err(|e| format!("bench writer key: {e}"));
    }
    let keys = nostr::Keys::generate();
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode nsec: {e}"))?;
    store.store(WRITER_KEY, &nsec)?;
    if !store.verify_stored_raw(WRITER_KEY, &nsec)? {
        return Err("keyring read-back verify failed".to_string());
    }
    Ok(keys)
}

/// A fresh NIP-OA tag for one request: minted per use, never persisted,
/// never inside an event.
fn owner_tag(
    owner: &nostr::Keys,
    writer_pk: &nostr::PublicKey,
    now: u64,
) -> Result<String, String> {
    let conditions = format!("created_at<{}", now + TAG_TTL_SECS);
    buzz_sdk_pkg::nip_oa::compute_auth_tag(owner, writer_pk, &conditions)
        .map_err(|e| format!("owner auth tag: {e}"))
}

/// Strictly increasing created_at for every writer event, so the board is
/// newest by created_at and seeds land >= 1 s apart. The floor only moves
/// when a timestamp is handed out, so deferring never widens the lead.
fn next_ts(s: &mut BenchState, now: u64) -> Result<nostr::Timestamp, String> {
    let next = now.max(s.last_created_at + 1);
    if next - now > MAX_CLOCK_LEAD_SECS {
        return Err("bench clock too far ahead; deferring".to_string());
    }
    s.last_created_at = next;
    Ok(nostr::Timestamp::from(next))
}

pub(crate) struct Rebuilt {
    pub cards: BTreeMap<String, Posted>,
    pub board: Option<Posted>,
    pub stray_ids: Vec<String>,
}

/// Keep the newest post per marker; the loser is a stray. Returns the post to
/// insert when the slot was empty.
fn newest(slot: Option<&mut Posted>, new: Posted, strays: &mut Vec<String>) -> Option<Posted> {
    match slot {
        Some(old) if old.created_at >= new.created_at => {
            strays.push(new.event_id);
            None
        }
        Some(old) => {
            strays.push(std::mem::replace(old, new).event_id);
            None
        }
        None => Some(new),
    }
}

/// Recover cards and board from the writer's own kind-9 events by their
/// `client` marker. Everything else the writer left in #bench is a stray:
/// listed, never deleted. Recovered cards are assumed seeded (a repeated seed
/// is a duplicate, which counts as success anyway).
pub(crate) fn rebuild_map(events: &[nostr::Event], writer_hex: &str) -> Rebuilt {
    let mut r = Rebuilt {
        cards: BTreeMap::new(),
        board: None,
        stray_ids: Vec::new(),
    };
    for e in events {
        if e.kind.as_u16() != 9 || e.pubkey.to_hex() != writer_hex {
            continue;
        }
        let marker = e.tags.iter().find_map(|t| match t.as_slice() {
            [k, m] if k == "client" => m.strip_prefix("bench:"),
            _ => None,
        });
        let posted = |hash: &str| Posted {
            event_id: e.id.to_hex(),
            created_at: e.created_at.as_secs(),
            hash: hash.to_string(),
            seeded: usize::MAX,
        };
        match marker.and_then(|m| m.rsplit_once('@')) {
            Some(("board", hash)) => {
                if let Some(p) = newest(r.board.as_mut(), posted(hash), &mut r.stray_ids) {
                    r.board = Some(p);
                }
            }
            Some((target, hash)) if target.starts_with("card:") => {
                let id = &target["card:".len()..];
                if let Some(p) = newest(r.cards.get_mut(id), posted(hash), &mut r.stray_ids) {
                    r.cards.insert(id.to_string(), p);
                }
            }
            _ => r.stray_ids.push(e.id.to_hex()),
        }
    }
    r
}

/// Drops in `inbox/<writer>/<id>.json`. A file modified < 1 s ago may still be
/// half written and waits a tick; an oversized one is refused unread.
fn scan_inbox(
    root: &Path,
    now: SystemTime,
) -> Vec<(PathBuf, String, String, Result<String, String>)> {
    let mut out = Vec::new();
    let Ok(writers) = std::fs::read_dir(root.join("inbox")) else {
        return out;
    };
    for w in writers.flatten() {
        let writer = w.file_name().to_string_lossy().into_owned();
        if !w.path().is_dir() || !item::valid_name(&writer, item::NAME_MAX) {
            continue;
        }
        for f in std::fs::read_dir(w.path()).into_iter().flatten().flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let stem = match name.strip_suffix(".json") {
                Some(stem) if !stem.ends_with(".rejected") => stem,
                _ => continue,
            };
            let Ok(meta) = f.metadata() else { continue };
            let age = meta.modified().ok().map(|m| now.duration_since(m));
            if matches!(age, Some(Ok(a)) if a < Duration::from_secs(1)) {
                continue;
            }
            let raw = if meta.len() > MAX_DROP_BYTES {
                Err("too large".to_string())
            } else {
                std::fs::read_to_string(f.path()).map_err(|e| e.to_string())
            };
            out.push((f.path(), writer.clone(), stem.to_string(), raw));
        }
    }
    out
}

/// Take every readable drop into state (keeping the card of the item it
/// replaces) or reject it in place as `<stem>.rejected.json`; the writer
/// cleans up its own rejections. Returns whether state changed.
fn intake(s: &mut BenchState, root: &Path, now_iso: &str) -> bool {
    let mut changed = false;
    for (path, writer, stem, raw) in scan_inbox(root, SystemTime::now()) {
        let (reason, raw) = match raw {
            Ok(raw) => match item::validate(&writer, &stem, &raw, now_iso) {
                Ok(item) => {
                    let old = s.items.get(&item.id);
                    if old.is_none_or(|o| o.hash != item.hash) {
                        let card = old.and_then(|o| o.card.clone());
                        s.items.insert(item.id.clone(), item::Item { card, ..item });
                        changed = true;
                    }
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                Err(reason) => (reason, Some(raw)),
            },
            Err(reason) => (reason, None),
        };
        let rejected = path.with_file_name(format!("{stem}.rejected.json"));
        let body = serde_json::json!({ "reason": reason, "drop": raw }).to_string();
        match crate::managed_agents::atomic_write_json_restricted(&rejected, body.as_bytes()) {
            Ok(()) => {
                let _ = std::fs::remove_file(&path);
            }
            // The drop stays for the next tick rather than vanishing unrecorded.
            Err(e) => eprintln!("bench: {e}"),
        }
    }
    changed
}

#[cfg(test)]
mod tests;
