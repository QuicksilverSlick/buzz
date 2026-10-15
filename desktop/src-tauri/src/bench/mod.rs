//! The Bench: a read-only owner dashboard as a private #bench channel, written
//! only by the Bench service with its own key. `item` is the trust boundary
//! for drop files; this file is the service: constants and the debug fence,
//! paths, state.json, writer key custody, the monotonic clock, recovery from
//! the writer's own events, the reaction poll and the outbox, inbox intake,
//! the pinned relay helpers and the 2 s loop. `publish` posts a tick's cards
//! and board; `channels` covers project channels bound in roles.json.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use nostr::ToBech32;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::events;
use crate::secret_store::SecretStore;

mod channels;
pub(crate) mod commands;
mod item;
mod pick;
mod plain;
mod publish;
mod roles;
mod route;

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
/// Receipts are never deleted, so the writer's kind 9s only grow.
const RECOVER_PAGES: usize = 10;
const REFUSED_TAPS_MAX: usize = 50;
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(60);
const TIMED_OUT: &str = "submit timed out";
/// lastError while roles.json does not read, before its error.
const ROLES_HOLD: &str = "roles.json can't be read, so every card is on hold: ";

/// The debug fence: a debug build may only ever reach a loopback relay. The
/// host is checked parsed, since `ws://localhost@evil.example` starts fine.
pub(crate) fn relay_ws_from(env: Option<&str>) -> Option<String> {
    if !cfg!(debug_assertions) {
        return Some(RELAY_WS.to_string());
    }
    let loopback = |u: &&str| {
        url::Url::parse(u).is_ok_and(|p| {
            p.scheme() == "ws" && matches!(p.host_str(), Some("localhost" | "127.0.0.1"))
        })
    };
    env.filter(loopback).map(str::to_string)
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
    /// A card: the `item::card_hash` its text shows (posted or last edited).
    /// The board: `item::board_hash`.
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
    /// Who the owner added to each bound project channel.
    joined: BTreeMap<String, Vec<String>>,
    /// Each bound channel's join or admin check: cards go only where it passed.
    channels: BTreeMap<String, channels::ChannelCheck>,
    /// Engaged or owner-told cards a new question left behind: on the relay
    /// for good, never adopted again by recover (which would retire them as
    /// stale).
    kept: Vec<String>,
    /// Each kept card posted in a thread, as (its root, its created_at): it
    /// still ends the root-reply window of an older card there.
    kept_at: Vec<(String, u64)>,
    /// Live cards whose owner's page was cut this tick, or every item while
    /// roles.json does not read: their answers are unread, so nothing
    /// expires, restamps, routes, retires or replaces them.
    held: Vec<String>,
    /// Floor of the monotonic clock for writer events.
    last_created_at: u64,
    board: Option<Posted>,
    board_dirty: bool,
    needs_rebuild: bool,
    items: BTreeMap<String, item::Item>,
    orphan_cards: BTreeMap<String, Posted>,
    strays: Vec<String>,
    /// routes.json entries refused this tick, as `<writer>: <reason>`.
    unroutable: Vec<String>,
    /// The last answers that did not count, newest last.
    refused_taps: Vec<RefusedTap>,
    /// No notice before this: an agent never sees events from before it
    /// subscribes, so a launch (rebuild) or a join holds them one settle.
    routes_held_until: u64,
    /// The owner's keys as the last tick resolved them (`owner_keys`).
    owner_keys: Vec<String>,
    last_tick: String,
    last_error: Option<String>,
}

/// A valid answer on a live card from a key that may not answer it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RefusedTap {
    pub item: String,
    pub by: String,
    pub event_id: String,
    /// 1-based.
    pub option: usize,
    pub at: u64,
    pub why: String,
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

/// state.json and state.prev.json alike, before an owner's notice first
/// goes out: whichever one loads next holds it (`item::OwnerSent`), so
/// neither a crash nor the fallback ever signs that notice afresh.
fn save_both(root: &Path, s: &BenchState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(s).map_err(|e| format!("serialize state: {e}"))?;
    for name in ["state.prev.json", "state.json"] {
        crate::managed_agents::atomic_write_json_restricted(&root.join(name), text.as_bytes())?;
    }
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
            // Answer notices: thread replies the Bench never edits or retires.
            Some((target, _)) if target.starts_with("route:") => {}
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

/// What a new Drop keeps of the item it replaces. A close-out (a decision or
/// status) of a carded request keeps the card, which becomes its receipt; a
/// new question after an answer, a reply or the owner's word leaves the old
/// card on the relay as it is and gets a card of its own with nothing told
/// yet, which comes back to whoever answered (`item::Previous`), and the
/// owner's approval told on the old card ends there (`item::Item::ends`);
/// anything else keeps the card for publish to refresh.
fn carry_card(
    old: Option<&item::Item>,
    mut new: item::Item,
    kept: &mut Vec<String>,
    kept_at: &mut Vec<(String, u64)>,
) -> item::Item {
    let Some(o) = old else { return new };
    let asking = new.kind == item::ItemKind::Pending;
    new.ends = o.ends.clone();
    if !asking && o.card.is_some() && (o.kind == item::ItemKind::Pending || o.request.is_some()) {
        let asked = item::Item {
            card: None,
            routed: None,
            owner_sent: None,
            ..o.clone()
        };
        new.request = o.request.clone().or(Some(Box::new(asked)));
        new.card = o.card.clone();
    } else if asking && (o.engaged() || o.request.is_some() || o.owner_told()) {
        let asked = o.request.as_deref().unwrap_or(o);
        if let Some(c) = &o.card {
            kept.push(c.event_id.clone());
            kept_at.extend(asked.reply_to.clone().map(|r| (r, c.created_at)));
        }
        let answered = asked.answer.as_ref().and_then(|a| {
            Some(item::Previous {
                option: a.option,
                label: asked.options.get(a.option)?.clone(),
                by: a.by.clone(),
                by_name: a.by_name.clone(),
                answered_at: a.answered_at,
            })
        });
        new.previous = answered.or_else(|| o.previous.clone());
        if o.owner_told() {
            new.ends = Some(Box::new(o.clone()));
        }
    } else {
        new.card = o.card.clone();
        new.previous = o.previous.clone().filter(|_| asking);
    }
    new
}

/// Take every readable drop into state (keeping the card of the item it
/// replaces, see `carry_card`) or reject it in place as
/// `<stem>.rejected.json`; the writer cleans up its own rejections. A scoped
/// drop waits while roles.json is unreadable, and a drop for a `held` card
/// until its answers are read. Returns whether state changed.
fn intake(
    s: &mut BenchState,
    root: &Path,
    now_iso: &str,
    roles: Option<&roles::Roles>,
    roles_bad: bool,
) -> bool {
    let mut changed = false;
    for (path, writer, stem, raw) in scan_inbox(root, SystemTime::now()) {
        let checked = raw.map(|raw| {
            let item = item::validate(&writer, &stem, &raw, now_iso);
            // Who may scope cards is unknown: the drop waits for roles.json.
            if roles_bad && item.as_ref().is_ok_and(item::Item::scoped) {
                return None;
            }
            let item = item.and_then(|mut i| roles::bind(&mut i, roles).map(|()| i));
            Some((item, raw))
        });
        let (reason, raw) = match checked {
            Ok(None) => continue,
            Ok(Some((Ok(item), _))) if s.held.contains(&item.id) => continue,
            Ok(Some((Ok(item), _))) => {
                let old = s.items.get(&item.id);
                if old.is_none_or(|o| o.hash != item.hash) {
                    // The hash-change re-drop is the writer's ack: the
                    // rebuilt item carries no answer, so the file goes too,
                    // whether or not state still remembers the answer.
                    let _ = std::fs::remove_file(outbox_path(root, &item.id));
                    let item = carry_card(old, item, &mut s.kept, &mut s.kept_at);
                    s.items.insert(item.id.clone(), item);
                    changed = true;
                }
                let _ = std::fs::remove_file(&path);
                continue;
            }
            Ok(Some((Err(reason), raw))) => (reason, Some(raw)),
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

// ── The relay half ──────────────────────────────────────────────────────

/// Everything one tick needs; assembled by `run()`, built directly by tests
/// with a loopback relay so no AppHandle or OS keyring is involved.
pub(crate) struct Ctx<'a> {
    pub state: &'a AppState,
    /// `relay_http_base_url(relay_ws)`.
    pub base: String,
    pub owner: nostr::Keys,
    pub writer: nostr::Keys,
    pub root: PathBuf,
    /// Managed agents on this desktop that admit the writer: the only
    /// possible route targets.
    pub agents: Vec<String>,
    /// Who every managed agent here takes instructions from: an agent a
    /// human on a card can instruct never answers that card.
    pub access: Vec<roles::AgentAccess>,
}

enum Outcome {
    Accepted { event_id: String },
    Duplicate,
}

/// One bounded POST through the pinned NIP-98 funnel. A `duplicate:` reply,
/// whether the relay accepted or rejected, means the event is already there.
async fn submit(
    ctx: &Ctx<'_>,
    ev: &nostr::Event,
    keys: &nostr::Keys,
    tag: Option<&str>,
) -> Result<Outcome, String> {
    let sent = tokio::time::timeout(
        SUBMIT_TIMEOUT,
        crate::relay::submit_signed_event_at_with_keys_tagged(ev, ctx.state, &ctx.base, keys, tag),
    )
    .await
    .map_err(|_| TIMED_OUT.to_string())?;
    match sent {
        Ok(r) if r.message.starts_with("duplicate:") => Ok(Outcome::Duplicate),
        Ok(r) => Ok(Outcome::Accepted {
            event_id: r.event_id,
        }),
        Err(e) if e.contains("duplicate:") => Ok(Outcome::Duplicate),
        Err(e) => Err(e),
    }
}

/// Owner-signed bootstrap events: never a tag, the owner is a member.
async fn post_as_owner(ctx: &Ctx<'_>, builder: nostr::EventBuilder) -> Result<Outcome, String> {
    let ev = builder
        .sign_with_keys(&ctx.owner)
        .map_err(|e| e.to_string())?;
    submit(ctx, &ev, &ctx.owner, None).await
}

/// Writer-signed events on the monotonic clock. The NIP-OA tag is lazy: the
/// relay materializes agent_owner on ANY verified tag, so it is sent only
/// after the relay has refused an untagged request as a non-member.
async fn post_as_writer(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    builder: nostr::EventBuilder,
    now: u64,
) -> Result<Outcome, String> {
    let ev = builder
        .custom_created_at(next_ts(s, now)?)
        .sign_with_keys(&ctx.writer)
        .map_err(|e| e.to_string())?;
    let writer_pk = ctx.writer.public_key();
    loop {
        let tag = match s.admission {
            Admission::ViaOwner => Some(owner_tag(&ctx.owner, &writer_pk, now)?),
            _ => None,
        };
        match submit(ctx, &ev, &ctx.writer, tag.as_deref()).await {
            Err(e) if e.contains("403") && e.contains("must be a relay member") => {
                if s.admission == Admission::ViaOwner {
                    s.admission = Admission::Denied;
                    return Err(format!(
                        "not admitted: add writer pubkey {} in Settings > Members (or enable NIP-OA)",
                        writer_pk.to_hex()
                    ));
                }
                s.admission = Admission::ViaOwner;
            }
            Err(e) => {
                // A lost reply may hide a landed event: re-read the relay.
                s.needs_rebuild |= e == TIMED_OUT;
                return Err(e);
            }
            Ok(outcome) => {
                if tag.is_none() {
                    s.admission = Admission::Open;
                }
                return Ok(outcome);
            }
        }
    }
}

/// One /query as the writer, every event verified: the relay helper only
/// parses JSON.
async fn query(
    ctx: &Ctx<'_>,
    s: &BenchState,
    filter: serde_json::Value,
    now: u64,
) -> Result<Vec<nostr::Event>, String> {
    let tag = match s.admission {
        Admission::ViaOwner => Some(owner_tag(&ctx.owner, &ctx.writer.public_key(), now)?),
        _ => None,
    };
    let events = crate::relay::query_relay_at_with_keys(
        ctx.state,
        &ctx.base,
        &[filter],
        &ctx.writer,
        tag.as_deref(),
    )
    .await?;
    Ok(events.into_iter().filter(|e| e.verify().is_ok()).collect())
}

/// The writer's own events only.
async fn query_writer(
    ctx: &Ctx<'_>,
    s: &BenchState,
    filter: serde_json::Value,
    now: u64,
) -> Result<Vec<nostr::Event>, String> {
    let pk = ctx.writer.public_key();
    Ok(query(ctx, s, filter, now)
        .await?
        .into_iter()
        .filter(|e| e.pubkey == pk)
        .collect())
}

/// members.txt: one 64-hex pubkey per line, `#` comments; how the phone's own
/// key joins private #bench without a UI.
fn read_members(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("members.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            l.split('#')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|l| l.len() == 64 && l.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect()
}

/// The owner's keys: the owner plus members.txt, less every agent key the
/// Bench knows (managed agents, routes.json targets, roles.json writers and
/// agents, its own writer): agents run as this OS user and can write
/// members.txt. Only a separate OS user for agents truly keeps them out.
/// While roles.json is unreadable its agents are unknown, so no members.txt
/// key counts. Err names each key left out, for lastError.
fn owner_keys(
    ctx: &Ctx<'_>,
    owner: &str,
    roles: Option<&roles::Roles>,
    roles_bad: bool,
) -> (Vec<String>, Result<(), String>) {
    let targets: BTreeMap<String, String> = std::fs::read_to_string(ctx.root.join("routes.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let roles_agents = roles.into_iter().flat_map(|r| {
        let writers = r.writers.values().filter_map(|w| w.pubkey.clone());
        let people = r.projects.values().flat_map(|p| &p.people);
        writers.chain(people.filter(|p| p.agent).map(|p| p.pk.clone()))
    });
    let agents: Vec<String> = ctx
        .access
        .iter()
        .map(|a| a.pubkey.clone())
        .chain(ctx.agents.iter().cloned())
        .chain(targets.into_values())
        .chain(roles_agents)
        .chain([ctx.writer.public_key().to_hex()])
        .map(|k| k.trim().to_ascii_lowercase())
        .collect();
    let (out, members): (Vec<String>, Vec<String>) = read_members(&ctx.root)
        .into_iter()
        .filter(|m| m != owner)
        .partition(|m| roles_bad || agents.contains(m));
    let set = std::iter::once(owner.to_string()).chain(members).collect();
    let why = match out.as_slice() {
        [] => Ok(()),
        _ if roles_bad => Err(format!(
            "roles.json is unreadable, so which keys are agents is unknown: no members.txt \
             key counts as yours or joins #bench until it reads: {}",
            out.join(", ")
        )),
        _ => Err(format!(
            "members.txt lists agent keys, which never count as yours and are not joined \
             to #bench as yours: {}",
            out.join(", ")
        )),
    };
    (set, why)
}

/// routes.json: `{"<writer folder>": "<agent hex>"}`, the agent that acts on
/// that writer's answers. The owner's file, not the drop's: a drop could aim
/// writer-signed mentions anywhere, and every target joins private #bench.
/// A target must be a managed agent here that admits the writer, and never
/// the writer, the owner or an approver (an agent must not answer cards).
/// Refused entries come back as `<writer>: <reason>`; no file, no routes.
fn read_routes(
    root: &Path,
    agents: &[String],
    approvers: &[String],
    writer_hex: &str,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut routes = BTreeMap::new();
    let mut unroutable = Vec::new();
    let map = match std::fs::read_to_string(root.join("routes.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (routes, unroutable),
        Err(e) => Err(e.to_string()),
        Ok(text) => {
            serde_json::from_str::<BTreeMap<String, String>>(&text).map_err(|e| e.to_string())
        }
    };
    let map = match map {
        Ok(map) => map,
        Err(e) => return (routes, vec![format!("routes.json: {e}")]),
    };
    for (writer, to) in map {
        let to = to.trim().to_ascii_lowercase();
        let reason = if !item::valid_name(&writer, item::NAME_MAX) {
            "not a writer folder name"
        } else if to.len() != 64 || !to.bytes().all(|b| b.is_ascii_hexdigit()) {
            "not a 64-hex pubkey"
        } else if to == writer_hex || approvers.contains(&to) {
            "the writer, the owner or an approver"
        } else if !agents.contains(&to) {
            "not a managed agent here that admits the Bench writer"
        } else {
            routes.insert(writer, to);
            continue;
        };
        unroutable.push(format!("{writer}: {reason}"));
    }
    (routes, unroutable)
}

/// Agents whose access, as this build enforces it, lets the writer's
/// mentions in: anyone, or an allowlist naming the writer.
fn admitting_agents(
    records: &[crate::managed_agents::ManagedAgentRecord],
    writer_hex: &str,
    owner_only_build: bool,
) -> Vec<String> {
    use crate::managed_agents::{access_policy, RespondTo};
    records
        .iter()
        .filter(
            |r| match access_policy::projected_access_with_policy(r, owner_only_build) {
                (RespondTo::Anyone, _) => true,
                (RespondTo::Allowlist, list) => list
                    .iter()
                    .any(|k| k.trim().eq_ignore_ascii_case(writer_hex)),
                (RespondTo::OwnerOnly, _) => false,
            },
        )
        .map(|r| r.pubkey.to_ascii_lowercase())
        .collect()
}

/// Local `HH:MM` of a Bench clock reading.
fn local_hhmm(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

/// Take `n` posts from the tick's budget, or say no.
fn spend(budget: &mut usize, n: usize) -> bool {
    match budget.checked_sub(n) {
        Some(left) => {
            *budget = left;
            true
        }
        None => false,
    }
}

/// kind 0 canary (writer) -> 9007 (owner) -> 9000 per owner's key
/// (`owner_keys`: members.txt less agent keys) and #bench route target
/// (owner). Each step is retried every tick until its flag is set. The
/// tag-free kind 0 goes first so a refused writer leaves no half-built
/// channel behind. Route targets join to read their notices; they never go
/// into members.txt, so they never answer.
async fn bootstrap(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
    owner_set: &[String],
    targets: &[String],
) -> Result<bool, String> {
    let ch = channel_id(&s.owner_pubkey);
    let writer_hex = ctx.writer.public_key().to_hex();
    if !s.profile_published {
        if !spend(budget, 1) {
            return Ok(false);
        }
        let profile = events::build_profile(Some("Bench"), Some("bench"), None, Some(ABOUT), None)?;
        post_as_writer(ctx, s, profile, now).await?;
        s.profile_published = true;
    }
    if !s.channel_created {
        if !spend(budget, 1) {
            return Ok(false);
        }
        let create =
            events::build_create_channel(ch, "bench", "private", "stream", Some(ABOUT), None)?;
        post_as_owner(ctx, create).await?;
        s.channel_created = true;
    }
    let members = owner_set.iter().filter(|k| **k != s.owner_pubkey);
    let joining = std::iter::once(&writer_hex)
        .chain(members)
        .chain(targets)
        .cloned()
        .collect::<Vec<_>>();
    for hex in joining {
        if s.members_added.contains(&hex) {
            continue;
        }
        if !spend(budget, 1) {
            break;
        }
        post_as_owner(ctx, events::build_add_member(ch, &hex, None)?).await?;
        s.members_added.push(hex);
        s.routes_held_until = now + item::ROUTE_SETTLE_SECS;
        // The relay stamps a member_joined row after the 9000: repost the
        // board so it stays newest.
        s.board_dirty = true;
    }
    Ok(s.members_added.contains(&writer_hex))
}

/// Rebuild cards and board from the writer's own kind-9 events in #bench and
/// the bound channels it joined, newest page first. Deletes nothing:
/// soft-deleted rows never come back from /query, and anything else is
/// listed as a stray for the owner.
async fn recover(ctx: &Ctx<'_>, s: &mut BenchState, now: u64) -> Result<(), String> {
    let writer_hex = ctx.writer.public_key().to_hex();
    let mut chans = vec![s.channel_id.clone()];
    chans.extend(
        s.joined
            .iter()
            .filter(|(ch, m)| **ch != s.channel_id && m.contains(&writer_hex))
            .map(|(ch, _)| ch.clone()),
    );
    let mut events: Vec<nostr::Event> = Vec::new();
    let (mut until, mut cut) = (None, false);
    for _ in 0..RECOVER_PAGES {
        let mut filter = serde_json::json!({
            "kinds": [9], "authors": [writer_hex], "#h": chans, "limit": RECOVER_LIMIT,
        });
        if let Some(u) = until {
            filter["until"] = serde_json::json!(u);
        }
        let page = query_writer(ctx, s, filter, now).await?;
        cut = page.len() as u32 >= RECOVER_LIMIT;
        // Writer timestamps are strictly increasing, so nothing shares the
        // oldest second of a page.
        until = page
            .iter()
            .map(|e| e.created_at.as_secs())
            .min()
            .map(|t| t.saturating_sub(1));
        events.extend(page);
        if !cut {
            break;
        }
    }
    events.retain(|e| !s.kept.contains(&e.id.to_hex()));
    let r = rebuild_map(&events, &writer_hex);
    for (id, posted) in r.cards {
        match s.items.get_mut(&id) {
            // The card we already track keeps its seed count.
            Some(item)
                if item
                    .card
                    .as_ref()
                    .is_some_and(|c| c.event_id == posted.event_id) => {}
            Some(item) => item.card = Some(posted),
            None => {
                s.orphan_cards.insert(id, posted);
            }
        }
    }
    s.board = r.board;
    s.strays = r.stray_ids;
    s.needs_rebuild = false;
    s.routes_held_until = now + item::ROUTE_SETTLE_SECS;
    // The partial map is kept; the Err is how state.json learns of the cut.
    if cut {
        return Err(format!(
            "rebuild truncated (more than {RECOVER_PAGES} pages of {RECOVER_LIMIT} writer kind-9 events)"
        ));
    }
    Ok(())
}

/// `outbox/<writer>/<id>.json`: the item id is `<writer>/<id>`, both
/// valid_name, so no traversal.
fn outbox_path(root: &Path, item_id: &str) -> PathBuf {
    root.join("outbox").join(format!("{item_id}.json"))
}

/// Whether a card shows the item's current Drop, in any card format.
fn same_drop(c: &Posted, i: &item::Item) -> bool {
    c.hash.split('.').next() == Some(i.hash.as_str())
}

/// The card that may carry an answer: present and showing the item's current
/// Drop, in any card format, so a tap on an older build's card counts before
/// publish (b) refreshes it. After an ack re-drop the old card stays until
/// (b) retires it, and it must never re-supply the answer just cleared.
fn live_card(i: &item::Item) -> Option<&Posted> {
    i.card.as_ref().filter(|c| same_drop(c, i))
}

/// Forget a card that is gone from the relay. The owner was already pinged
/// for a card of the same Drop, so its replacement goes out quietly.
fn forget_card(s: &mut BenchState, id: &str) {
    let i = s.items.get_mut(id).expect("carded id");
    let same = i.card.as_ref().is_some_and(|c| same_drop(c, i));
    i.quiet_repost |= same;
    i.card = None;
    i.recorded = None;
    s.board_dirty = true;
}

/// A stamped answerer's roster name and role; None for the owner's own keys.
fn named(
    i: &item::Item,
    pk: &str,
    owner_set: &[String],
) -> (Option<String>, Option<item::AnswerRole>) {
    if owner_set.iter().any(|o| o == pk) {
        return (None, None);
    }
    let a = i
        .stamp
        .iter()
        .flat_map(|s| &s.answerers)
        .find(|a| a.pk == pk);
    (a.map(|a| a.name.clone()), a.map(|a| a.role))
}

/// Re-derive every answer from the kind-7 taps (and, on scoped cards, the
/// kind-9 thread replies) on the live cards, from the owner's keys and the
/// cards' answerers only. The whole set comes back each tick, so a
/// retraction is an event that stopped coming back and nothing is ever
/// double-applied. The outbox file lands before state records the answer: a
/// failed write is retried next tick instead of being lost. Ok(Some) says
/// a page was cut: the cards it covers keep their answers, they are read
/// again next tick, the tick goes on, and it is lastError. A card the
/// owner's cut page covers is also `held` for the rest of the tick.
// ponytail: one /query per 2 s tick, two once a live card has answerers,
// twice that while a card takes its thread root's replies (up to 120/min of
// the 300/min budget); swap for the live #h tail via
// native_relay_client::start when the relay complains.
async fn poll_answers(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    now: u64,
    owner_set: &[String],
    routes: &BTreeMap<String, String>,
) -> Result<Option<String>, String> {
    s.held.clear();
    // A channel not ready for cards is not read either: its answers stand.
    let live: Vec<&item::Item> = s
        .items
        .values()
        .filter(|i| i.kind == item::ItemKind::Pending && live_card(i).is_some())
        .filter(|i| channels::channel_ready(s, i))
        .collect();
    if live.is_empty() {
        return Ok(None);
    }
    let cards: Vec<String> = live
        .iter()
        .filter_map(|i| live_card(i))
        .map(|c| c.event_id.clone())
        .collect();
    // Replies that name only a card's thread root (`item::root_until`) are a
    // read of their own: their whole threads match, so it starts an hour
    // (clock skew) before the oldest card that takes them, nothing older
    // counts on any of them, and a busy thread never cuts the cards' page.
    let rooted: Vec<&item::Item> = live
        .iter()
        .copied()
        .filter(|i| item::root_until(i, s.items.values(), &s.kept, &s.kept_at).is_some())
        .collect();
    let roots: Vec<String> = rooted.iter().filter_map(|i| i.reply_to.clone()).collect();
    let since = rooted
        .iter()
        .filter_map(|i| live_card(i))
        .map(|c| c.created_at.saturating_sub(3600))
        .min();
    let rooted: Vec<String> = rooted.iter().map(|i| i.id.clone()).collect();
    let mut answerers: Vec<String> = live
        .iter()
        .flat_map(|i| i.stamp.iter().flat_map(|st| &st.answerers))
        .map(|a| a.pk.clone())
        .filter(|pk| !owner_set.contains(pk))
        .collect();
    answerers.sort();
    answerers.dedup();
    let chans: std::collections::BTreeSet<String> =
        live.iter().map(|i| channels::card_channel(s, i)).collect();
    let kinds = if live.iter().any(|i| i.scoped()) {
        serde_json::json!([7, 9])
    } else {
        serde_json::json!([7])
    };
    let agents: Vec<String> = live
        .iter()
        .flat_map(|i| i.stamp.iter().flat_map(|st| &st.answerers))
        .filter(|a| a.agent)
        .map(|a| a.pk.clone())
        .chain(ctx.access.iter().map(|a| a.pubkey.clone()))
        .collect();
    let ids: Vec<String> = live.iter().map(|i| i.id.clone()).collect();
    let page = |authors: &[String], e: &[String], since: Option<u64>| {
        let mut f = serde_json::json!({
            "kinds": kinds, "authors": authors, "#h": chans, "#e": e,
            "limit": RECOVER_LIMIT,
        });
        if let Some(t) = since {
            f["since"] = t.into();
        }
        f
    };
    // The owner's keys are read apart from everyone else's, so nobody else's
    // traffic can cut the owner's page. A cut page hides answers: never infer
    // a retraction from it. Each full page, as (the owner's, a thread's).
    let mut events = Vec::new();
    let mut full = Vec::new();
    for (owners, authors) in [(true, owner_set), (false, answerers.as_slice())] {
        for (threads, e, since) in [(false, &cards, None), (true, &roots, since)] {
            if authors.is_empty() || e.is_empty() {
                continue;
            }
            let got = query(ctx, s, page(authors, e, since), now).await?;
            if got.len() as u32 >= RECOVER_LIMIT {
                full.push((owners, threads));
            }
            events.extend(got);
        }
    }
    // Whether a page that `id` is read from was cut.
    let cut = |owners: bool, id: &String| {
        full.iter()
            .any(|&(o, t)| o == owners && (!t || rooted.contains(id)))
    };
    let why = if full.iter().any(|f| f.0) {
        Some(format!(
            "answers truncated (>= {RECOVER_LIMIT} events from your keys on live cards); \
             those cards' answers stand"
        ))
    } else {
        (!full.is_empty()).then(|| {
            format!(
                "answers truncated (>= {RECOVER_LIMIT} events from the cards' answerers); \
                 their answers stand"
            )
        })
    };
    let theirs = |by: &str| !owner_set.iter().any(|o| o == by);
    let writer_hex = ctx.writer.public_key().to_hex();
    let hhmm = local_hhmm(now);
    let mut refused = Vec::new();
    let mut held = Vec::new();
    for id in ids {
        // The owner's page was cut: everything on this card stands, and it
        // is `held` until it is read, since an answer may be taken back or a
        // tap unseen.
        if cut(true, &id) {
            held.push(id);
            continue;
        }
        let until = item::root_until(&s.items[&id], s.items.values(), &s.kept, &s.kept_at);
        let item = s.items.get_mut(&id).expect("live id");
        let card = live_card(item).expect("live card").event_id.clone();
        // Their page was cut, so an answer or reply of theirs here may be
        // hidden: theirs stand as they are. The owner's page is whole, so it
        // still decides once the owner spoke, and an answer of the owner's
        // that stopped coming back was withdrawn.
        let unread = cut(false, &id) && !item::owner_spoke(item, &card, &events, owner_set, until);
        for e in item::refused_taps(item, &events, owner_set, &writer_hex, until) {
            let by = e.pubkey.to_hex();
            let why = if item.needs == item::Needs::You && agents.contains(&by) {
                "agent on a needs-you card"
            } else {
                "not an answerer on this card"
            };
            refused.push(RefusedTap {
                item: id.clone(),
                by,
                event_id: e.id.to_hex(),
                option: item::answer_option(item, &card, e, until).map_or(0, |i| i + 1),
                at: e.created_at.as_secs(),
                why: why.to_string(),
            });
        }
        // The person's Other: kept while it is the newest, so its first-seen
        // time holds; publish (a3) routes it unless it already pings the agent.
        let picked = (!unread)
            .then(|| item::pick_reply(item, &events, owner_set, &writer_hex, until))
            .flatten();
        item.reply = match picked {
            Some(e)
                if item
                    .reply
                    .as_ref()
                    .is_some_and(|r| r.event_id == e.id.to_hex()) =>
            {
                item.reply.take()
            }
            Some(e) => {
                let by = e.pubkey.to_hex();
                let (by_name, role) = named(item, &by, owner_set);
                let pinged = routes.get(&item.writer).is_some_and(|to| {
                    e.tags
                        .iter()
                        .any(|t| matches!(t.as_slice(), [k, p, ..] if k == "p" && p == to))
                });
                if pinged {
                    item.routed_reply = Some(e.id.to_hex());
                }
                Some(item::Reply {
                    event_id: e.id.to_hex(),
                    by,
                    by_name,
                    role,
                    at: now,
                    hhmm: hhmm.clone(),
                })
            }
            None if unread => item
                .reply
                .take()
                .filter(|r| theirs(&r.by) && item::eligible(item, &r.by, owner_set)),
            None => None,
        };
        let path = outbox_path(&ctx.root, &id);
        let picked = (!unread)
            .then(|| item::pick_answer(item, &events, owner_set, &writer_hex, until))
            .flatten();
        match picked {
            // Unless the roster no longer lets them answer (`restamp`).
            None if unread
                && item
                    .answer
                    .as_ref()
                    .is_some_and(|a| theirs(&a.by) && item::eligible(item, &a.by, owner_set)) => {}
            Some((_, e))
                if item
                    .answer
                    .as_ref()
                    .is_some_and(|a| a.reaction_id == e.id.to_hex()) => {}
            Some((i, e)) => {
                let by = e.pubkey.to_hex();
                let (by_name, role) = named(item, &by, owner_set);
                let source = if e.kind.as_u16() == 9 {
                    item::AnswerSource::Reply
                } else {
                    item::AnswerSource::Tap
                };
                let eligible: Vec<&String> = item
                    .stamp
                    .iter()
                    .flat_map(|st| &st.answerers)
                    .map(|a| &a.pk)
                    .collect();
                // "owner" for the owner's own keys.
                let answerer_role =
                    role.map_or(serde_json::json!("owner"), |r| serde_json::json!(r));
                std::fs::create_dir_all(path.parent().expect("outbox dir"))
                    .map_err(|e| format!("create outbox: {e}"))?;
                let body = serde_json::json!({
                    "item": id, "writer": item.writer, "card": card,
                    "option": i + 1, "text": item.options[i], "reaction": e,
                    "answerer": by, "answererRole": answerer_role,
                    "eligible": eligible, "source": source,
                });
                crate::managed_agents::atomic_write_json_restricted(
                    &path,
                    body.to_string().as_bytes(),
                )?;
                item.changed_at = now;
                item.answer = Some(item::Answer {
                    reaction_id: e.id.to_hex(),
                    option: i,
                    answered_at: now,
                    by,
                    by_name,
                    role,
                    source,
                    hhmm: hhmm.clone(),
                });
            }
            // Retried every tick with every error ignored (Windows sharing
            // violations included): a remove that failed, or a crash between
            // the write and save_state, must not leave a withdrawn answer on
            // disk for the writer to act on.
            None => {
                let _ = std::fs::remove_file(&path);
                if item.answer.take().is_some() {
                    item.changed_at = now;
                }
            }
        }
    }
    for t in refused {
        if !s.refused_taps.iter().any(|x| x.event_id == t.event_id) {
            s.refused_taps.push(t);
        }
    }
    let over = s.refused_taps.len().saturating_sub(REFUSED_TAPS_MAX);
    s.refused_taps.drain(..over);
    s.held = held;
    Ok(why)
}

/// A pending item past its deadline folds off the board: Superseded, so
/// publish (b) retires its card. Engaged and `held` items are left alone,
/// and a same-hash re-drop cannot revive one: a new deadline is needed.
fn expire(s: &mut BenchState, now: u64) {
    for i in s.items.values_mut() {
        let due = i
            .deadline
            .as_deref()
            .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
            .is_some_and(|t| t.timestamp().max(0) as u64 <= now);
        if due
            && i.kind == item::ItemKind::Pending
            && i.validity == item::Validity::Current
            && !i.engaged()
            && !s.held.contains(&i.id)
        {
            i.validity = item::Validity::Superseded;
        }
    }
}

/// Re-resolve who may answer each open scoped request. An engaged or `held`
/// one keeps the set it was answered under, less anyone the roster no
/// longer lets answer it (the poll then drops their answer or reply); an
/// unreadable roster leaves it as it is. A changed stamp changes the
/// card_hash, so publish (b) reposts a card nobody engaged with and (a2)
/// re-renders any other, even once its answer or reply was taken back.
/// Agents the writer's p tag cannot wake are reported.
fn restamp(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    roles: Option<&roles::Roles>,
    roles_bad: bool,
    owner_set: &[String],
    routes: &BTreeMap<String, String>,
) {
    let writer_hex = ctx.writer.public_key().to_hex();
    for i in s.items.values_mut() {
        if i.kind != item::ItemKind::Pending || i.validity != item::Validity::Current {
            continue;
        }
        let target = routes.get(&i.writer).map(String::as_str);
        let fresh = roles::stamp(i, roles, owner_set, &ctx.access, target, &writer_hex);
        i.was_engaged |= i.engaged();
        if !i.engaged() && !s.held.contains(&i.id) {
            i.stamp = fresh;
        } else if let (Some(st), Some(fresh), false) = (i.stamp.as_mut(), fresh, roles_bad) {
            // Removals only: nobody is silently added to an answered card.
            st.answerers
                .retain(|a| fresh.answerers.iter().any(|f| f.pk == a.pk));
        }
        for a in i.stamp.iter().flat_map(|st| &st.answerers) {
            if a.agent && !ctx.agents.contains(&a.pk) {
                let name = item::plain_title(&a.name);
                s.unroutable
                    .push(format!("{}: {name} does not admit the Bench writer", i.id));
            }
        }
    }
}

/// One 2 s tick: owner fence, bootstrap, the bound channels' joins and admin
/// check, recovery, the answer poll (before intake, so a query failure aborts
/// the tick before any drop is read), intake, expiry, answerers, publish. An
/// unreadable roles.json (every card on hold), or a cut page of answers,
/// fails the tick only after publish: #bench and new cards carry on.
pub(crate) async fn tick(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    now: u64,
    hhmm: &str,
    now_iso: &str,
) -> Result<(), String> {
    let owner_hex = ctx.owner.public_key().to_hex();
    if s.owner_pubkey.is_empty() {
        s.channel_id = channel_id(&owner_hex).to_string();
        s.owner_pubkey = owner_hex;
    } else if s.owner_pubkey != owner_hex {
        return Err(
            "owner identity changed; refusing to tick (BUZZ_PRIVATE_KEY or an identity \
                    import would create a second #bench)"
                .to_string(),
        );
    }
    s.last_tick = now_iso.to_string();
    let writer_hex = ctx.writer.public_key().to_hex();
    s.writer_pubkey = writer_hex.clone();
    let (roles, roles_err) = match channels::read_roles(&ctx.root) {
        Ok(r) => (r, None),
        Err(e) => (None, Some(format!("{ROLES_HOLD}{e}"))),
    };
    // The owner's keys: the owner plus the phone's own key in members.txt.
    let (owner_set, not_owner) =
        owner_keys(ctx, &s.owner_pubkey, roles.as_ref(), roles_err.is_some());
    s.owner_keys = owner_set.clone();
    let why: Vec<String> = roles_err.iter().cloned().chain(not_owner.err()).collect();
    let done = (!why.is_empty())
        .then(|| why.join("; "))
        .map_or(Ok(()), Err);
    let (routes, unroutable) = read_routes(&ctx.root, &ctx.agents, &owner_set, &writer_hex);
    s.unroutable = unroutable;
    // Route targets join only their own writer's channels: a bound folder's
    // never joins #bench, and while roles.json is unreadable none does.
    let bench_targets: Vec<String> = routes
        .iter()
        .filter(|(w, _)| {
            roles_err.is_none() && roles.as_ref().is_none_or(|r| !r.writers.contains_key(*w))
        })
        .map(|(_, to)| to.clone())
        .collect();
    let mut budget = MAX_POSTS_PER_TICK;
    if !bootstrap(ctx, s, &mut budget, now, &owner_set, &bench_targets).await? {
        return done;
    }
    if let Some(r) = &roles {
        channels::join_bound(ctx, s, &mut budget, now, r, &routes).await;
    }
    channels::check_admins(ctx, s, now, &owner_set).await;
    if s.needs_rebuild {
        recover(ctx, s, now).await?;
    }
    // Whose keys are the owner's is unknown, so no answer may be read as
    // taken back: every card stands as it is, `held`, until roles.json reads.
    let cut = if roles_err.is_some() {
        s.held = s.items.keys().cloned().collect();
        None
    } else {
        poll_answers(ctx, s, now, &owner_set, &routes).await?
    };
    intake(s, &ctx.root, now_iso, roles.as_ref(), roles_err.is_some());
    expire(s, now);
    restamp(
        ctx,
        s,
        roles.as_ref(),
        roles_err.is_some(),
        &owner_set,
        &routes,
    );
    let mut failed = Vec::new();
    publish::publish(
        ctx,
        s,
        &mut budget,
        now,
        hhmm,
        &routes,
        &owner_set,
        &mut failed,
    )
    .await?;
    let failed = (!failed.is_empty()).then(|| failed.join("; "));
    done.and(cut.map_or(Ok(()), Err))
        .and(failed.map_or(Ok(()), Err))
}

/// The service loop. state.json (lastTick / lastError / admission) is the
/// observability surface: release builds have no console.
pub(crate) async fn run(app: tauri::AppHandle) {
    if let Err(e) = serve(app).await {
        eprintln!("bench: {e}");
    }
}

async fn serve(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let env = std::env::var("BUZZ_BENCH_RELAY_URL").ok();
    let Some(relay) = relay_ws_from(env.as_deref()) else {
        return Err(
            "disabled (debug build without BUZZ_BENCH_RELAY_URL=ws://127.0.0.1:...)".to_string(),
        );
    };
    // The first tagged request binds the writer to whichever owner signs it,
    // first write wins on the relay: never a release override identity.
    if !cfg!(debug_assertions) && std::env::var_os("BUZZ_PRIVATE_KEY").is_some() {
        return Err(
            "BUZZ_PRIVATE_KEY is set; refusing to bind the Bench writer to an override identity"
                .to_string(),
        );
    }
    let root = root_dir()?;
    // Opt-in: the owner enables the Bench by creating this folder. Until then
    // an installer that carries this code posts nothing to the relay.
    if !root.is_dir() {
        return Err(format!("disabled (create {} to enable)", root.display()));
    }
    std::fs::create_dir_all(root.join("inbox"))
        .map_err(|e| format!("create {}: {e}", root.display()))?;
    let writer = writer_keys()?;
    let writer_hex = writer.public_key().to_hex();
    // The writer key must never double as a managed agent. The raw store:
    // a pubkey compare has no business pulling agent secrets from the keyring.
    if crate::managed_agents::load_agent_store(&app)?
        .iter()
        .any(|r| r.pubkey == writer_hex)
    {
        return Err("writer key is a managed agent; refusing to start".to_string());
    }
    let (mut s, _fresh) = load_state(&root);
    // Every launch re-reads the relay: a kill between a post and save_state
    // would otherwise leave an unlisted card behind for good.
    s.needs_rebuild = true;
    s.relay = relay.clone();
    s.writer_pubkey = writer_hex.clone();
    s.version = 1;
    let base = crate::relay::relay_http_base_url(&relay);
    let mut last_written = String::new();
    loop {
        tokio::time::sleep(POLL).await;
        let state = app.state::<AppState>();
        let now_iso = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let result = match state.signing_keys() {
            Ok(owner) => {
                // Re-read every tick: an allowlist edit takes effect without a
                // relaunch. A broken store routes nothing (state.json says so),
                // and an agent with no access record counts as instructable by
                // anyone.
                let records = crate::managed_agents::load_agent_store(&app).unwrap_or_default();
                let owner_only = crate::managed_agents::access_policy::owner_only();
                let ctx = Ctx {
                    state: &state,
                    base: base.clone(),
                    owner,
                    writer: writer.clone(),
                    root: root.clone(),
                    agents: admitting_agents(&records, &writer_hex, owner_only),
                    access: roles::agent_access(&records, owner_only),
                };
                let now = nostr::Timestamp::now().as_secs();
                let hhmm = chrono::Local::now().format("%H:%M").to_string();
                tick(&ctx, &mut s, now, &hhmm, &now_iso).await
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => s.last_error = None,
            Err(e) => {
                eprintln!("bench: {e}");
                // One toast per failure edge, re-armed by an Ok tick.
                if s.last_error.is_none() {
                    use tauri_plugin_notification::NotificationExt;
                    let _ = app.notification().builder().title("Bench").body(&e).show();
                }
                s.last_error = Some(e);
            }
        }
        if let Err(e) = save_state(&root, &s, &mut last_written) {
            eprintln!("bench: {e}");
        }
    }
}

#[cfg(test)]
mod tests;
