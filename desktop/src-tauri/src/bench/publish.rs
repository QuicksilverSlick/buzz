//! The relay posts of one tick: Recorded and receipt edits, answer and reply
//! notices, card retirement and posting, seeds, then the board. Every step
//! lands in state first, so the next tick resumes where this one stopped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use super::channels::{card_channel, channel_ready, joined, reads_replies};
use super::{
    forget_card, item, live_card, local_hhmm, next_ts, post_as_writer, query, query_writer,
    same_drop, save_both, spend, submit, BenchState, Ctx, Outcome, Posted, BOARD_MAX_AGE_SECS,
    MAX_CARDS, RECOVER_LIMIT,
};
use crate::events;

/// How far past MAX_CARDS a card that preempts (`item::preempts`) or that
/// the owner tapped on the desktop (`bump`) may post.
pub(super) const OVER_CAP: usize = 10;
/// Longer than the desktop waits for a bumped card to post.
pub(super) const BUMP_SECS: u64 = 120;

/// The owner's desktop taps on cards waiting for a slot, `(root, item id)`
/// -> when (`commands::bench_bump`): each posts next, past MAX_CARDS up to
/// OVER_CAP, for BUMP_SECS. In memory, in the desktop process that runs the
/// tick, so no file an agent can write orders a post. It never answers.
static BUMPS: Mutex<BTreeMap<(PathBuf, String), u64>> = Mutex::new(BTreeMap::new());

pub(super) fn bump(root: &Path, id: &str, now: u64) {
    let mut bumps = BUMPS.lock().unwrap_or_else(PoisonError::into_inner);
    bumps.retain(|_, at| now < at.saturating_add(BUMP_SECS));
    bumps.insert((root.to_path_buf(), id.to_string()), now);
}

fn bumped(root: &Path, now: u64) -> Vec<String> {
    let bumps = BUMPS.lock().unwrap_or_else(PoisonError::into_inner);
    bumps
        .iter()
        .filter(|((r, _), at)| r == root && now < at.saturating_add(BUMP_SECS))
        .map(|((_, id), _)| id.clone())
        .collect()
}

/// The open cards: pending, current, unanswered, in a channel ready for them.
fn open(s: &BenchState) -> impl Iterator<Item = &item::Item> {
    s.items.values().filter(|i| {
        i.kind == item::ItemKind::Pending
            && i.validity == item::Validity::Current
            && i.answer.is_none()
            && channel_ready(s, i)
    })
}

/// An open card with a place: posted, or coming back (`Item::repost`).
pub(super) fn placed(i: &item::Item) -> bool {
    i.card.is_some() || i.repost
}

/// Slots in use: each placed open card, and an answered card for
/// ANSWER_SLOT_SECS. Receipts hold none.
pub(super) fn in_use(s: &BenchState, now: u64) -> usize {
    let kept = s
        .items
        .values()
        .filter(|i| {
            i.kind == item::ItemKind::Pending
                && i.card.is_some()
                && i.answer
                    .as_ref()
                    .is_some_and(|a| now.saturating_sub(a.answered_at) <= item::ANSWER_SLOT_SECS)
        })
        .count();
    kept + open(s).filter(|i| placed(i)).count()
}

/// Twice the relay's +-900 s created_at window: past it the relay refuses an
/// event's time before it would answer duplicate, so only a lookup can tell
/// (and sooner, by the relay's clock: `resend_owner`).
const OWNER_RESEND_SECS: u64 = 1800;

/// A kind 9 carrying its `client` marker; the p tags only for whom it pings
/// (a card's answerers or the owner, a notice's route target). `extra` tags
/// go on unchecked: only the Bench's own `df-` tags.
fn message(
    ctx: &Ctx<'_>,
    ch: &str,
    text: &str,
    pings: &[&str],
    thread: Option<&events::ThreadRef>,
    marker: String,
    extra: Vec<nostr::Tag>,
) -> Result<nostr::EventBuilder, String> {
    let ch = uuid::Uuid::parse_str(ch).map_err(|e| e.to_string())?;
    Ok(events::build_message_with_client_tags(
        ch,
        text,
        thread,
        pings,
        &[],
        &[],
        &[],
        &[],
        None,
        &ctx.base,
        &[vec!["client".to_string(), marker]],
    )?
    .tags(extra))
}

/// `message`, from the writer.
#[allow(clippy::too_many_arguments)]
async fn post_message(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    ch: &str,
    text: &str,
    pings: &[&str],
    thread: Option<&events::ThreadRef>,
    marker: String,
    extra: Vec<nostr::Tag>,
    now: u64,
) -> Result<Outcome, String> {
    let builder = message(ctx, ch, text, pings, thread, marker, extra)?;
    post_as_writer(ctx, s, builder, now).await
}

/// An owner's notice that may not have reached the relay (a timeout, a lost
/// reply, a restart): sent again as that same event, never signed afresh.
/// Once its time is too old for the relay, or the card it replies to is
/// gone, a lookup by id in the channel it went to (its h tag, else `ch`)
/// says whether it landed; if not, it never can, and the next tick signs one
/// afresh. Either way the `owner_sent` it was kept in (`slot`) clears, and
/// `routed` lands there if the relay has it. No card is forgotten here: the
/// item's own may not be the card this event replies to.
async fn resend_owner(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    id: &str,
    ends: bool,
    ch: &str,
    sent: item::OwnerSent,
    now: u64,
) -> Result<(), String> {
    let ev = &sent.event;
    let h = ev.tags.iter().find_map(|t| match t.as_slice() {
        [k, v, ..] if k == "h" => Some(v.as_str()),
        _ => None,
    });
    let ch = h.unwrap_or(ch).to_string();
    let mut told = false;
    if now < ev.created_at.as_secs() + OWNER_RESEND_SECS {
        match submit(ctx, ev, &ctx.owner, None).await {
            Ok(_) => told = true,
            // Its card is gone, or its time is past the relay's own clock's
            // window (buzz-relay MAX_TIMESTAMP_DRIFT_SECS, checked before
            // duplicate): whether it landed first, the lookup says.
            Err(e) if e.contains("not found") || e.contains("too far from server time") => {}
            Err(e) => return Err(e),
        }
    }
    if !told {
        let filter = serde_json::json!({
            "ids": [ev.id.to_hex()], "kinds": [9], "#h": [ch],
        });
        told = query(ctx, s, filter, now)
            .await?
            .iter()
            .any(|e| e.id == ev.id);
    }
    let top = !ev
        .tags
        .iter()
        .any(|t| matches!(t.as_slice(), [k, ..] if k == "e"));
    let i = slot(s, id, ends);
    i.owner_sent = None;
    if told {
        i.routed = Some(sent.routed);
        s.board_dirty |= top && ch == s.channel_id;
    }
    Ok(())
}

/// The item an owner's notice is kept on: `id` itself, or (`ends`) the item
/// its re-ask replaced.
fn slot<'a>(s: &'a mut BenchState, id: &str, ends: bool) -> &'a mut item::Item {
    let i = s.items.get_mut(id).expect("routed id");
    match ends {
        true => i.ends.as_deref_mut().expect("re-asked item"),
        false => i,
    }
}

/// A card's thread: under the root it was posted in reply to, if any.
pub(super) fn thread_of(
    card: Option<&Posted>,
    root: Option<&str>,
) -> Result<Option<events::ThreadRef>, String> {
    let id = |h: &str| nostr::EventId::from_hex(h).map_err(|e| e.to_string());
    let Some(c) = card.map(|c| id(&c.event_id)).transpose()? else {
        return Ok(None);
    };
    Ok(Some(events::ThreadRef {
        root_event_id: root.map(id).transpose()?.unwrap_or(c),
        parent_event_id: c,
    }))
}

/// The owner's approval told on the card `id` re-asked (`item::Item::ends`)
/// ends there, in the owner's own message (`item::render_owner_ended`), or
/// (`closed`, the card `id` closed out) is withdrawn there
/// (`item::render_owner_withdrawal`); kept on disk before it goes and only
/// ever sent again as itself, like any owner's notice. One still unsettled
/// there is settled first, as itself; then publish (a3) says whether more
/// is still owed.
async fn end_approval(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    id: &str,
    old: &item::Item,
    closed: bool,
    now: u64,
) -> Result<(), String> {
    let ch = card_channel(s, old);
    if let Some(sent) = old.owner_sent.clone() {
        return resend_owner(ctx, s, id, true, &ch, sent, now).await;
    }
    let told = old.routed.clone().expect("the owner's approval");
    let (text, n) = match closed {
        true => (item::render_owner_withdrawal(old, &told), "withdrawn"),
        false => (item::render_owner_ended(old), "ended"),
    };
    let text = text.ok_or("its card or option has no name")?;
    let thread = thread_of(old.card.as_ref(), old.reply_to.as_deref())?;
    let marker = format!("bench:route:{id}@{n}");
    let ping = [told.to.as_str()];
    let builder = message(ctx, &ch, &text, &ping, thread.as_ref(), marker, Vec::new())?;
    let ev = builder
        .custom_created_at(next_ts(s, now)?)
        .sign_with_keys(&ctx.owner)
        .map_err(|e| e.to_string())?;
    // Once it lands the last notice there is no approval, so none is owed.
    let ended = item::Routed {
        option: None,
        event_id: ev.id.to_hex(),
        at: now,
        owner: false,
        ..told
    };
    slot(s, id, true).owner_sent = Some(item::OwnerSent {
        event: ev.clone(),
        routed: ended.clone(),
    });
    save_both(&ctx.root, s)?;
    let landed = match submit(ctx, &ev, &ctx.owner, None).await {
        // Its card was deleted by hand, so it never landed: it goes
        // top-level next tick.
        Err(e) if thread.is_some() && e.contains("not found") => false,
        r => r.map(|_| true)?,
    };
    let old = slot(s, id, true);
    old.owner_sent = None;
    if landed {
        old.routed = Some(ended);
    } else {
        old.card = None;
    }
    s.board_dirty |= landed && thread.is_none() && ch == s.channel_id;
    Ok(())
}

/// kind 5 with h+e on one of the writer's own events (never 9005).
async fn delete(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    ch: &str,
    event_id: &str,
    now: u64,
) -> Result<Outcome, String> {
    let target = nostr::EventId::from_hex(event_id).map_err(|e| e.to_string())?;
    let ch = uuid::Uuid::parse_str(ch).map_err(|e| e.to_string())?;
    let builder = events::build_delete_compat(ch, target)?;
    match post_as_writer(ctx, s, builder, now).await {
        // A hard-purged target is already gone: nothing left to retire.
        Err(e) if e.contains("not found") => Ok(Outcome::Duplicate),
        r => r,
    }
}

/// kind 40003 with h+e replacing the text of one of the writer's own kind 9s.
async fn edit(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    ch: &str,
    event_id: &str,
    text: &str,
    now: u64,
) -> Result<Outcome, String> {
    let target = nostr::EventId::from_hex(event_id).map_err(|e| e.to_string())?;
    let ch = uuid::Uuid::parse_str(ch).map_err(|e| e.to_string())?;
    let edit_tags = events::MessageEditTags {
        media: &[],
        custom_emoji: &[],
        mentions: &[],
        mention_refs: None,
    };
    let builder = events::build_message_edit(ch, target, text, edit_tags, false)?;
    post_as_writer(ctx, s, builder, now).await
}

/// Local `HH:MM` of `at`, as `YYYY-MM-DD HH:MM` when that is not today.
fn local_when(at: u64, now: u64) -> String {
    let day = |t: u64| {
        chrono::DateTime::from_timestamp(t as i64, 0)
            .map(|t| t.with_timezone(&chrono::Local).date_naive())
    };
    match day(at) {
        Some(d) if day(now) != Some(d) => format!("{d} {}", local_hhmm(at)),
        _ => local_hhmm(at),
    }
}

fn df_answer(parts: [&str; 3]) -> Result<nostr::Tag, String> {
    let [n, by, id] = parts;
    nostr::Tag::parse(["df-answer", n, by, id]).map_err(|e| e.to_string())
}

/// Whether a recovered card has anything on it but the writer's own seeds:
/// a tap, a reply, a route notice, or a Recorded or receipt edit. A cut page
/// holds more than the four seeds, so it counts too.
async fn touched(
    ctx: &Ctx<'_>,
    s: &BenchState,
    ch: &str,
    card: &str,
    now: u64,
) -> Result<bool, String> {
    let filter = serde_json::json!({
        "kinds": [7, 9, 40003], "#h": [ch], "#e": [card], "limit": RECOVER_LIMIT,
    });
    let writer = ctx.writer.public_key();
    Ok(query(ctx, s, filter, now).await?.iter().any(|e| {
        let on_card = e
            .tags
            .iter()
            .any(|t| matches!(t.as_slice(), [k, v, ..] if k == "e" && v == card));
        on_card && !(e.kind.as_u16() == 7 && e.pubkey == writer)
    }))
}

/// A stale card kept for what is on it (`touched`), for good: edited closed
/// from its own text as posted (`item::render_closed`), its thread root (its
/// parent) ending older cards' root-reply windows there. One gone from the
/// relay is only kept.
async fn close(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    ch: &str,
    card: &Posted,
    now: u64,
) -> Result<(), String> {
    let filter = serde_json::json!({ "ids": [card.event_id], "kinds": [9], "#h": [ch] });
    if let Some(e) = query_writer(ctx, s, filter, now).await?.first() {
        let text = item::render_closed(&e.content) + &item::thread_line(ch, &card.event_id);
        match edit(ctx, s, ch, &card.event_id, &text, now).await {
            Err(err) if !err.contains("not found") => return Err(err),
            _ => {}
        }
        let root = item::parent(e).map(|r| (r.to_string(), card.created_at));
        s.kept_at.extend(root);
    }
    s.kept.push(card.event_id.clone());
    Ok(())
}

/// A thread the relay will not take: its root is gone, in another channel or
/// not a thread root.
fn bad_thread(e: &str) -> bool {
    ["not found", "different channel", "thread ancestry"]
        .iter()
        .any(|m| e.contains(m))
}

/// Cards, seeds, then the board, at most `budget` posts. A card is posted
/// once per Drop and card format; it is edited to record an answer and, once
/// closed out, to become a receipt. Only a card nobody engaged with is ever
/// deleted. Every post made once a card's own id is known links its thread.
/// An owner's notice that fails to go again lands in `failed`, for
/// lastError, and the rest goes on.
#[allow(clippy::too_many_arguments)]
pub(super) async fn publish(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
    hhmm: &str,
    routes: &BTreeMap<String, String>,
    owner_set: &[String],
    failed: &mut Vec<String>,
) -> Result<(), String> {
    // (a) The live cards: the open ones (`open`), in `item::rank` order.
    // MAX_CARDS gates new posts only: an open card with a place (`placed`:
    // posted, or retired for a refresh and coming back) keeps it, so (b)
    // never takes down a card someone may be tapping, and (c) reposts it
    // before any newcomer. A newcomer posts while fewer than MAX_CARDS slots
    // are in use (`in_use`); one that preempts (`item::preempts`) or that the
    // owner tapped on the desktop (`bump`), while fewer than MAX_CARDS +
    // OVER_CAP. The rest wait in `s.queued`, which the board and the desktop
    // Bench show.
    //
    // Sized for the factory's open questions today (31 Oct 2026: 23
    // unanswered at once, about 10 answered a day). A new card costs 1 post
    // plus a seed per option, at MAX_POSTS_PER_TICK, so 30 two-option cards
    // take ~23 ticks (~45 s). At ten times that (300 open) the queue holds
    // the rest in rank order behind the alarm, and critical cards and
    // re-asks still go out until OVER_CAP is used up. Not raised: what the
    // phone shows is unmeasured. The fork's mobile/ pages a channel 50
    // top-level events at a time, reactions included, and pages back with
    // them, but the phone may run Block's build: measure it there before
    // raising MAX_CARDS.
    let bumped = bumped(&ctx.root, now);
    let mut used = in_use(s, now);
    let mut ranked: Vec<&item::Item> = open(s).collect();
    ranked.sort_by_key(|i| (!bumped.contains(&i.id), item::rank(i)));
    let (on, off): (Vec<&item::Item>, Vec<&item::Item>) =
        ranked.into_iter().partition(|i| placed(i));
    let mut wanted: Vec<String> = on.into_iter().map(|i| i.id.clone()).collect();
    let mut queued = Vec::new();
    for i in off {
        let cap = if bumped.contains(&i.id) || item::preempts(i) {
            MAX_CARDS + OVER_CAP
        } else {
            MAX_CARDS
        };
        if used < cap {
            used += 1;
            wanted.push(i.id.clone());
        } else {
            queued.push(i.id.clone());
        }
    }
    s.queued = queued;

    // (a2) Record / un-record on the live card, first so a tap is acknowledged
    // within one tick. Diff-driven from what the card text shows (`recorded`,
    // and who it names) vs the answer, so it is a no-op after a restart.
    for id in s.items.keys().cloned().collect::<Vec<_>>() {
        let item = s.items[&id].clone();
        let Some(card) = live_card(&item).filter(|_| item.kind == item::ItemKind::Pending) else {
            continue;
        };
        let want = item.answer.as_ref().map(|a| a.option);
        let want_by = item.answer.as_ref().and_then(|a| a.by_name.clone());
        // A card someone answered or replied on is never reposted: a roster
        // change (`restamp`) rewrites its For line in place, also after the
        // removal took their answer or reply back.
        let restamped = item.stamp.is_some()
            && (item.engaged() || item.was_engaged)
            && card.hash != item::card_hash(&item);
        if item.recorded == want && item.recorded_by == want_by && !restamped {
            continue;
        }
        if !spend(budget, 1) {
            return Ok(());
        }
        let ch = card_channel(s, &item);
        let text = item::render_card(&item, hhmm) + &item::thread_line(&ch, &card.event_id);
        match edit(ctx, s, &ch, &card.event_id, &text, now).await {
            // The edit re-rendered the whole card, so it now shows this card
            // format too: (b) must not retire it for an older one.
            Ok(_) => {
                let i = s.items.get_mut(&id).expect("edited id");
                i.recorded = want;
                i.recorded_by = want_by;
                i.card.as_mut().expect("edited card").hash = item::card_hash(&item);
            }
            // A hand-deleted card: the answer stays and is never reposted; an
            // unanswered one comes back through (c).
            Err(e) if e.contains("not found") => forget_card(s, &id),
            Err(e) => return Err(e),
        }
    }

    // (a2b) A close-out keeps the card: edited to the receipt, with the
    // answer and this drop as its Outcome. A later close-out edits it again.
    for id in s.items.keys().cloned().collect::<Vec<_>>() {
        let item = &s.items[&id];
        let (Some(_), Some(card)) = (&item.request, &item.card) else {
            continue;
        };
        let hash = item::card_hash(item);
        if card.hash == hash {
            continue;
        }
        if !spend(budget, 1) {
            return Ok(());
        }
        let (event_id, ch) = (card.event_id.clone(), card_channel(s, item));
        let text = item::render_card(item, hhmm) + &item::thread_line(&ch, &event_id);
        match edit(ctx, s, &ch, &event_id, &text, now).await {
            Ok(_) => {
                let i = s.items.get_mut(&id).expect("receipt id");
                i.card.as_mut().expect("receipt card").hash = hash;
            }
            Err(e) if e.contains("not found") => forget_card(s, &id),
            Err(e) => return Err(e),
        }
    }

    // (a3) Tell the writer's route target of a settled answer change, and of
    // each new free-text reply on the card, in the card's thread so the
    // board stays newest. `routed` lands only once the relay took the notice,
    // so a failed post is retried next tick.
    for id in s.items.keys().cloned().collect::<Vec<_>>() {
        let item = s.items[&id].clone();
        let ch = card_channel(s, &item);
        let Some(to) = routes
            .get(&item.writer)
            .filter(|to| joined(s, &ch).contains(to))
            .cloned()
        else {
            continue;
        };
        if item
            .stamp
            .iter()
            .flat_map(|st| &st.answerers)
            .any(|a| a.pk == to)
        {
            s.unroutable
                .push(format!("{id}: route target is an answerer on this card"));
            continue;
        }
        // A channel no longer ready is not read either (an untap there is
        // unseen), so its answers wait, like a held card's.
        if now < s.routes_held_until || s.held.contains(&id) || !channel_ready(s, &item) {
            continue;
        }
        // The owner's approval on the card this one re-asked ends there
        // first: this card's notices wait for it. On the card this one
        // closed out (its request is that drop, or neither has a card) it stands
        // once settled, as long as the answer is still the one told.
        if let Some(old) = item.ends.as_deref() {
            let closed = item.kind != item::ItemKind::Pending
                && item
                    .request
                    .as_ref()
                    .map_or(old.card.is_none(), |r| r.hash == old.hash);
            let stands = old.owner_sent.is_none() && !item::route_due(old, u64::MAX, owner_set);
            if old.owner_told() && !(closed && stands) {
                if !spend(budget, 1) {
                    return Ok(());
                }
                if let Err(e) = end_approval(ctx, s, &id, old, closed, now).await {
                    failed.push(format!("{id}: the owner's approval did not end: {e}"));
                }
                continue;
            }
            s.items.get_mut(&id).expect("routed id").ends = None;
        }
        // An owner's notice already signed is settled first, as itself.
        if let Some(sent) = item.owner_sent.clone() {
            if !spend(budget, 1) {
                return Ok(());
            }
            if let Err(e) = resend_owner(ctx, s, &id, false, &ch, sent, now).await {
                failed.push(format!("{id}: the owner's notice did not go again: {e}"));
            }
            continue;
        }
        let thread = thread_of(live_card(&item), item.reply_to.as_deref())?;
        let link = live_card(&item).map_or(String::new(), |c| item::thread_line(&ch, &c.event_id));
        let ping = [to.as_str()];
        if item::route_due(&item, now, owner_set) {
            if !spend(budget, 1) {
                return Ok(());
            }
            let prev = item.routed.as_ref().and_then(|r| r.option);
            let at = item
                .answer
                .as_ref()
                .map_or(item.changed_at, |a| a.answered_at);
            // The owner's own tap goes out as the owner's own message, so the
            // agent acts on the owner's word (`item::owner_route`). One told
            // that way and now withdrawn or not the owner's tap is taken back
            // that way first, as a withdrawal (the other answer follows next
            // tick). Anything else, or text that fails the checks again, is
            // the writer's.
            let mine = item::owner_route(&item, owner_set);
            let taken_back = item
                .routed
                .as_ref()
                .filter(|r| r.owner && mine.is_none())
                .and_then(|r| item::render_owner_withdrawal(&item, r));
            let answer = item.answer.as_ref().filter(|_| taken_back.is_none());
            let want = answer.map(|a| a.option);
            let owners = taken_back.or(mine);
            let as_owner = owners.is_some();
            let text =
                owners.unwrap_or_else(|| item::render_route(&item, prev, &local_hhmm(at)) + &link);
            let n = want.map_or("withdrawn".to_string(), |i| (i + 1).to_string());
            // Who answered, by key, so an agent elsewhere needs no outbox file.
            let extra = match answer {
                Some(a) if item.scoped() => vec![df_answer([&n, &a.by, &a.reaction_id])?],
                _ => Vec::new(),
            };
            let marker = format!("bench:route:{id}@{n}");
            let mut routed = item::Routed {
                option: want,
                to: to.clone(),
                event_id: String::new(),
                at: now,
                owner: as_owner && want.is_some(),
                card: live_card(&item).map(|c| c.event_id.clone()),
            };
            let builder = message(ctx, &ch, &text, &ping, thread.as_ref(), marker, extra)?;
            let posted = if as_owner {
                // On the monotonic clock so the board stays newest, and on
                // disk before it goes (`resend_owner`).
                let ev = builder
                    .custom_created_at(next_ts(s, now)?)
                    .sign_with_keys(&ctx.owner)
                    .map_err(|e| e.to_string())?;
                routed.event_id = ev.id.to_hex();
                let sent = item::OwnerSent {
                    event: ev.clone(),
                    routed: routed.clone(),
                };
                s.items.get_mut(&id).expect("routed id").owner_sent = Some(sent);
                save_both(&ctx.root, s)?;
                submit(ctx, &ev, &ctx.owner, None).await
            } else {
                post_as_writer(ctx, s, builder, now).await
            };
            let outcome = match posted {
                // The card was deleted by hand: the notice goes top-level next
                // tick. The relay refused it, so an owner's never landed.
                Err(e) if thread.is_some() && e.contains("not found") => {
                    s.items.get_mut(&id).expect("routed id").owner_sent = None;
                    forget_card(s, &id);
                    continue;
                }
                r => r?,
            };
            // A top-level notice is now newer than the board.
            s.board_dirty |= thread.is_none() && ch == s.channel_id;
            // A writer's already on the relay is told all the same.
            if let (false, Outcome::Accepted { event_id }) = (as_owner, outcome) {
                routed.event_id = event_id;
            }
            let i = s.items.get_mut(&id).expect("routed id");
            i.owner_sent = None;
            i.routed = Some(routed);
        }
        // The person's Other: no settle, a reply is not undone by a tap.
        let Some(r) = item
            .reply
            .as_ref()
            .filter(|r| item.routed_reply.as_ref() != Some(&r.event_id))
        else {
            continue;
        };
        if !spend(budget, 1) {
            return Ok(());
        }
        let text = item::render_route_reply(&item, &local_when(r.at, now)) + &link;
        let marker = format!("bench:route:{id}@r{}", &r.event_id[..16]);
        let extra = vec![df_answer(["other", &r.by, &r.event_id])?];
        let posted = post_message(
            ctx,
            s,
            &ch,
            &text,
            &ping,
            thread.as_ref(),
            marker,
            extra,
            now,
        );
        match posted.await {
            Err(e) if thread.is_some() && e.contains("not found") => {
                forget_card(s, &id);
                continue;
            }
            r => r?,
        };
        s.board_dirty |= thread.is_none() && ch == s.channel_id;
        s.items.get_mut(&id).expect("routed id").routed_reply = Some(r.event_id.clone());
    }

    // (b) Delete cards that are stale (another Drop, card format or stamp) or
    // no longer asked (superseded, expired, no longer pending), never for
    // MAX_CARDS, unless someone answered or replied on them or they are a
    // receipt: those stay on the relay for good, and so, closed, does one
    // with anything else on it but its seeds. A card in a channel that is
    // not ready, or `held`, waits: its taps are read once it is again.
    let stale: Vec<(String, String, Posted)> = s
        .items
        .values()
        .filter_map(|i| {
            let c = i.card.as_ref()?;
            let stale = c.hash != item::card_hash(i) || !wanted.contains(&i.id);
            let waits = !channel_ready(s, i) || s.held.contains(&i.id);
            let retire = !i.engaged() && i.request.is_none() && stale && !waits;
            retire.then(|| (i.id.clone(), card_channel(s, i), c.clone()))
        })
        .collect();
    for (id, ch, card) in stale {
        // One post per card, its lookups riding on it: the delete, or the edit
        // that closes it.
        if !spend(budget, 1) {
            return Ok(());
        }
        // A reply or tap the item never read or no longer holds (free text on
        // an unscoped card with no route, or one asked again) still makes the
        // card a record: it stays, as `carry_card` keeps one, but closed,
        // since nothing reads it any more.
        if touched(ctx, s, &ch, &card.event_id, now).await? {
            close(ctx, s, &ch, &card, now).await?;
        } else {
            delete(ctx, s, &ch, &card.event_id, now).await?;
        }
        forget_card(s, &id);
    }

    // (c) Post missing cards, adopting a recovered orphan when its hash matches.
    for id in &wanted {
        if s.items[id].card.is_some() {
            continue;
        }
        let ch = card_channel(s, &s.items[id]);
        if let Some(orphan) = s.orphan_cards.get(id).cloned() {
            if orphan.hash == item::card_hash(&s.items[id]) {
                s.items.get_mut(id).expect("wanted id").card = s.orphan_cards.remove(id);
                continue;
            }
            // An answered, replied or receipt card stays on the relay for good.
            if touched(ctx, s, &ch, &orphan.event_id, now).await? {
                s.kept.push(orphan.event_id.clone());
            } else {
                if !spend(budget, 1) {
                    return Ok(());
                }
                delete(ctx, s, &ch, &orphan.event_id, now).await?;
            }
            s.orphan_cards.remove(id);
            let i = s.items.get_mut(id).expect("wanted id");
            i.quiet_repost |= same_drop(&orphan, i);
        }
        if !spend(budget, 1) {
            return Ok(());
        }
        let item = s.items[id].clone();
        let hash = item::card_hash(&item);
        let marker = format!("bench:card:{}@{hash}", item.id);
        let text = item::render_card(&item, hhmm);
        let pings = item::card_pings(&item, &s.owner_pubkey, owner_set);
        let pinged: Vec<&str> = pings.iter().map(String::as_str).collect();
        let thread = item
            .reply_to
            .as_deref()
            .map(nostr::EventId::from_hex)
            .transpose()
            .map_err(|e| e.to_string())?
            .map(|r| events::ThreadRef {
                root_event_id: r,
                parent_event_id: r,
            });
        let posted = post_message(
            ctx,
            s,
            &ch,
            &text,
            &pinged,
            thread.as_ref(),
            marker,
            Vec::new(),
            now,
        );
        match posted.await {
            // Rather than stall every card behind this one, it goes top-level.
            Err(e) if thread.is_some() && bad_thread(&e) => {
                eprintln!("bench: {id}: replyTo refused ({e}); posting top-level");
                s.items.get_mut(id).expect("wanted id").reply_to = None;
            }
            Err(e) => return Err(e),
            Ok(Outcome::Accepted { event_id }) => {
                let created_at = s.last_created_at;
                // Posted while routed: no reply on it predates the route.
                let seen = reads_replies(s, &item, routes);
                let i = s.items.get_mut(id).expect("wanted id");
                i.card = Some(Posted {
                    event_id,
                    created_at,
                    hash,
                    seeded: 0,
                });
                i.routed_seen |= seen;
                i.quiet_repost = false;
                i.repost = false;
                i.pinged.extend(pings);
                s.board_dirty = true;
            }
            Ok(Outcome::Duplicate) => {
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
    let mut text = item::render_board(&all, &s.queued, &s.channel_id, hhmm, now);
    let refused = s
        .channels
        .iter()
        .filter_map(|(ch, c)| Some(format!("- {ch}: {}", c.refused.as_ref()?)));
    item::section(&mut text, "Cards refused", refused);
    // Every limit has an alarm: the cap says what it holds back, and why.
    let n = s.queued.len();
    let why = format!(
        "The Bench posts a new card only while fewer than {MAX_CARDS} slots are in use; a \
         critical card, one asked again or one tapped on the desktop Bench may take it to {}. \
         Cards marked (queued for a slot) post in order as slots free; tap one on the desktop \
         Bench to post it now.",
        MAX_CARDS + OVER_CAP
    );
    item::section(
        &mut text,
        &format!("Card slots: {used} of {MAX_CARDS} in use, {n} queued"),
        (n > 0 || used > MAX_CARDS).then_some(why).into_iter(),
    );
    let hash = item::board_hash(&text);
    let aged = s
        .board
        .as_ref()
        .is_none_or(|b| now.saturating_sub(b.created_at) > BOARD_MAX_AGE_SECS);
    let bench = s.channel_id.clone();
    if s.board_dirty || aged {
        // Both the post and the old board's delete, so the pair never splits
        // the budget (a crash between them leaves two boards, which
        // rebuild_map resolves by keeping the newest).
        if !spend(budget, 2) {
            return Ok(());
        }
        let marker = format!("bench:board@{hash}");
        match post_message(ctx, s, &bench, &text, &[], None, marker, Vec::new(), now).await? {
            Outcome::Accepted { event_id } => {
                let old = s.board.replace(Posted {
                    event_id,
                    created_at: s.last_created_at,
                    hash,
                    seeded: 0,
                });
                s.board_dirty = false;
                if let Some(old) = old {
                    delete(ctx, s, &bench, &old.event_id, now).await?;
                }
            }
            Outcome::Duplicate => s.needs_rebuild = true,
        }
    } else if let Some(board_id) = s
        .board
        .as_ref()
        .filter(|b| b.hash != hash)
        .map(|b| b.event_id.clone())
    {
        if !spend(budget, 1) {
            return Ok(());
        }
        match edit(ctx, s, &bench, &board_id, &text, now).await {
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
