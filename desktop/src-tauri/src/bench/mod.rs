//! The Bench: a read-only owner dashboard as a private #bench channel, written
//! only by the Bench service with its own key. `item` is the trust boundary
//! for drop files; this file is the service: constants and the debug fence,
//! paths, state.json, writer key custody, the monotonic clock, recovery from
//! the writer's own events, inbox intake, the pinned relay posts and the
//! 2 s loop.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use nostr::ToBech32;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::events;
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
const TIMED_OUT: &str = "submit timed out";

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

/// The writer's own events, verified and filtered to its key: the query
/// helper only parses JSON.
async fn query_writer(
    ctx: &Ctx<'_>,
    s: &BenchState,
    filter: serde_json::Value,
    now: u64,
) -> Result<Vec<nostr::Event>, String> {
    let writer_pk = ctx.writer.public_key();
    let tag = match s.admission {
        Admission::ViaOwner => Some(owner_tag(&ctx.owner, &writer_pk, now)?),
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
    Ok(events
        .into_iter()
        .filter(|e| e.verify().is_ok() && e.pubkey == writer_pk)
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

/// kind 0 canary (writer) -> 9007 (owner) -> 9000 per member (owner). Each
/// step is retried every tick until its flag is set. The tag-free kind 0 goes
/// first so a refused writer leaves no half-built channel behind.
async fn bootstrap(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
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
    for hex in std::iter::once(writer_hex.clone()).chain(read_members(&ctx.root)) {
        if s.members_added.contains(&hex) {
            continue;
        }
        if !spend(budget, 1) {
            break;
        }
        post_as_owner(ctx, events::build_add_member(ch, &hex, None)?).await?;
        s.members_added.push(hex);
        // The relay stamps a member_joined row after the 9000: repost the
        // board so it stays newest.
        s.board_dirty = true;
    }
    Ok(s.members_added.contains(&writer_hex))
}

/// Rebuild cards and board from the writer's own kind-9 events. Deletes
/// nothing: soft-deleted rows never come back from /query, and anything else
/// is listed as a stray for the owner.
async fn recover(ctx: &Ctx<'_>, s: &mut BenchState, now: u64) -> Result<(), String> {
    let writer_hex = ctx.writer.public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [9], "authors": [writer_hex], "#h": [s.channel_id], "limit": RECOVER_LIMIT,
    });
    let events = query_writer(ctx, s, filter, now).await?;
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
    // The partial map is kept; the Err is how state.json learns of the cut.
    if events.len() as u32 >= RECOVER_LIMIT {
        return Err(format!(
            "rebuild truncated (>= {RECOVER_LIMIT} writer kind-9 events in #bench)"
        ));
    }
    Ok(())
}

/// A kind 9 in #bench carrying its `client` marker; the p tag only when the
/// card pings the owner.
async fn post_message(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    text: &str,
    ping: bool,
    marker: String,
    now: u64,
) -> Result<Outcome, String> {
    let owner_hex = s.owner_pubkey.clone();
    let owner_p = [owner_hex.as_str()];
    let mentions: &[&str] = if ping { &owner_p } else { &[] };
    let builder = events::build_message_with_client_tags(
        channel_id(&owner_hex),
        text,
        None,
        mentions,
        &[],
        &[],
        &[],
        &[],
        None,
        &ctx.base,
        &[vec!["client".to_string(), marker]],
    )?;
    post_as_writer(ctx, s, builder, now).await
}

/// kind 5 with h+e on one of the writer's own events (never 9005).
async fn delete(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    event_id: &str,
    now: u64,
) -> Result<Outcome, String> {
    let target = nostr::EventId::from_hex(event_id).map_err(|e| e.to_string())?;
    let builder = events::build_delete_compat(channel_id(&s.owner_pubkey), target)?;
    match post_as_writer(ctx, s, builder, now).await {
        // A hard-purged target is already gone: nothing left to retire.
        Err(e) if e.contains("not found") => Ok(Outcome::Duplicate),
        r => r,
    }
}

/// Cards, seeds, then the board, at most `budget` posts; the next tick
/// resumes where this one stopped because every step lands in state first.
/// Cards are always delete + repost; only the board is ever edited.
async fn publish(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
    hhmm: &str,
) -> Result<(), String> {
    // (a) The live cards: pending + current by severity, at most MAX_CARDS.
    let mut ranked: Vec<(u8, i64, String)> = s
        .items
        .values()
        .filter(|i| i.kind == item::ItemKind::Pending && i.validity == item::Validity::Current)
        .map(|i| (item::severity_rank(i.severity), i.order, i.id.clone()))
        .collect();
    ranked.sort();
    let wanted: Vec<String> = ranked.into_iter().take(MAX_CARDS).map(|r| r.2).collect();

    // (b) Delete cards that are stale or no longer wanted.
    let stale: Vec<(String, String)> = s
        .items
        .values()
        .filter_map(|i| {
            let c = i.card.as_ref()?;
            (c.hash != i.hash || !wanted.contains(&i.id))
                .then(|| (i.id.clone(), c.event_id.clone()))
        })
        .collect();
    for (id, event_id) in stale {
        if !spend(budget, 1) {
            return Ok(());
        }
        delete(ctx, s, &event_id, now).await?;
        s.items.get_mut(&id).expect("stale card id").card = None;
        s.board_dirty = true;
    }

    // (c) Post missing cards, adopting a recovered orphan when its hash matches.
    for id in &wanted {
        if s.items[id].card.is_some() {
            continue;
        }
        if let Some(orphan) = s.orphan_cards.get(id).cloned() {
            if orphan.hash == s.items[id].hash {
                s.items.get_mut(id).expect("wanted id").card = s.orphan_cards.remove(id);
                continue;
            }
            if !spend(budget, 1) {
                return Ok(());
            }
            delete(ctx, s, &orphan.event_id, now).await?;
            s.orphan_cards.remove(id);
        }
        if !spend(budget, 1) {
            return Ok(());
        }
        let item = s.items[id].clone();
        let marker = format!("bench:card:{}@{}", item.id, item.hash);
        let text = item::render_card(&item, hhmm);
        match post_message(ctx, s, &text, item::ping_owner(&item), marker, now).await? {
            Outcome::Accepted { event_id } => {
                s.items.get_mut(id).expect("wanted id").card = Some(Posted {
                    event_id,
                    created_at: s.last_created_at,
                    hash: item.hash,
                    seeded: 0,
                });
                s.board_dirty = true;
            }
            Outcome::Duplicate => {
                s.needs_rebuild = true;
                return Ok(());
            }
        }
    }

    // (d) Seed one keycap reaction per option, in order, >= 1 s apart.
    let ids: Vec<String> = s.items.keys().cloned().collect();
    for id in ids {
        loop {
            let item = &s.items[&id];
            let Some(card) = &item.card else { break };
            if card.seeded >= item.options.len().min(item::OPTION_EMOJI.len()) {
                break;
            }
            if !spend(budget, 1) {
                return Ok(());
            }
            let target = nostr::EventId::from_hex(&card.event_id).map_err(|e| e.to_string())?;
            let seed = events::build_reaction(target, item::OPTION_EMOJI[card.seeded])?;
            post_as_writer(ctx, s, seed, now).await?;
            let card = s
                .items
                .get_mut(&id)
                .and_then(|i| i.card.as_mut())
                .expect("seeded card");
            card.seeded += 1;
        }
    }

    // (e) The board: reposted after any card change or daily, edited in place
    // for a text-only change so it keeps its id and stays newest.
    let all: Vec<&item::Item> = s.items.values().collect();
    let text = item::render_board(&all, &s.channel_id, hhmm);
    let hash = item::board_hash(&text);
    let aged = s
        .board
        .as_ref()
        .is_none_or(|b| now.saturating_sub(b.created_at) > BOARD_MAX_AGE_SECS);
    if s.board_dirty || aged {
        // Both the post and the old board's delete, so the pair never splits
        // the budget (a crash between them leaves two boards, which
        // rebuild_map resolves by keeping the newest).
        if !spend(budget, 2) {
            return Ok(());
        }
        match post_message(ctx, s, &text, false, format!("bench:board@{hash}"), now).await? {
            Outcome::Accepted { event_id } => {
                let old = s.board.replace(Posted {
                    event_id,
                    created_at: s.last_created_at,
                    hash,
                    seeded: 0,
                });
                s.board_dirty = false;
                if let Some(old) = old {
                    delete(ctx, s, &old.event_id, now).await?;
                }
            }
            Outcome::Duplicate => s.needs_rebuild = true,
        }
    } else if let Some(board) = s.board.as_ref().filter(|b| b.hash != hash) {
        if !spend(budget, 1) {
            return Ok(());
        }
        let target = nostr::EventId::from_hex(&board.event_id).map_err(|e| e.to_string())?;
        let edit_tags = events::MessageEditTags {
            media: &[],
            custom_emoji: &[],
            mentions: &[],
            mention_refs: None,
        };
        let edit = events::build_message_edit(
            channel_id(&s.owner_pubkey),
            target,
            &text,
            edit_tags,
            false,
        )?;
        match post_as_writer(ctx, s, edit, now).await {
            Ok(_) => s.board.as_mut().expect("board").hash = hash,
            // The board was deleted by hand: repost it next tick instead of
            // retrying the edit every 2 s until the daily repost.
            Err(e) if e.contains("not found") => {
                s.board = None;
                s.board_dirty = true;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One 2 s tick: owner fence, bootstrap, recovery, intake, publish.
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
    let mut budget = MAX_POSTS_PER_TICK;
    if !bootstrap(ctx, s, &mut budget, now).await? {
        return Ok(());
    }
    if s.needs_rebuild {
        recover(ctx, s, now).await?;
    }
    intake(s, &ctx.root, now_iso);
    publish(ctx, s, &mut budget, now, hhmm).await
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
    s.writer_pubkey = writer_hex;
    s.version = 1;
    let base = crate::relay::relay_http_base_url(&relay);
    let mut last_written = String::new();
    loop {
        tokio::time::sleep(POLL).await;
        let state = app.state::<AppState>();
        let now_iso = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let result = match state.signing_keys() {
            Ok(owner) => {
                let ctx = Ctx {
                    state: &state,
                    base: base.clone(),
                    owner,
                    writer: writer.clone(),
                    root: root.clone(),
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
