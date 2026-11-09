//! Old posts: cards a newer card replaced, or that stopped being asked, left
//! on the relay. Only a live card is read (`poll_answers`), yet the old post
//! stays in the owner's feed with its pills, and Block's phone shows nothing
//! but text and keycaps. So it says on its first line that a tap there doesn't
//! count and loses its seeds, and a tap on it all the same gets one reply in
//! its thread. That reply is all such a tap ever does: a tap is tied to the
//! card that was shown, so it never answers, never reaches the outbox and
//! never moves to the new card.
//!
//! None is deleted. A phone that lost its socket across the delete keeps
//! showing the post, the relay refuses a tap on it and the phone's code ignores
//! the refusal, so that tap would vanish with nothing said.
//!
//! Nothing here fails a tick: lastError stops the steward kit's approvals, and
//! the live cards are fine. What keeps failing is on the board (`alarm`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::channels::{card_channel, ready};
use super::publish::{edit, post_message, thread_of};
use super::{
    item, post_as_writer, query, query_writer, spend, BenchState, Ctx, Posted, RECOVER_LIMIT,
};
use crate::events;

/// The first line of an old post whose item is still asked.
const REPLACED: &str = "Replaced by a newer card. A tap here doesn't count.";
/// The first line of one whose item is not.
const NOT_ASKED: &str = "No longer asked. A tap here doesn't count.";
/// How long a tap on an old post still gets its reply. After that its first
/// line and its missing pills are all that say so.
const WATCH_SECS: u64 = 86_400;
/// Old posts read for taps at once, newest first. One batched /query per tick
/// while any is watched, however many: +30/min of the relay's 300/min (about
/// 5/s), beside `poll_answers`' up to 120. Sized for three times the busiest day
/// at 1 Nov 2026, 28 drops if each left an old post; ten times that outgrows it.
/// A cut page keeps its newest events, the ones looked for. Past this the oldest
/// go unread and the board says so (`alarm`); raising it only lengthens the one
/// query's `#e`.
pub(super) const WATCH_MAX: usize = 100;
/// An old post that would not update is tried again this soon.
const RETRY_SECS: u64 = 60;
/// A read of the old posts failing this long goes on the board.
const READ_ALARM_SECS: u64 = 60;
/// Lookups one tick spends on old posts, beside its read: +30/min at most, and
/// only while a post is owed work, so neither a run of posts with no seeds nor
/// a failing relay costs a query per post. With the read's 30, the poll's 120
/// and 120 posts, every ceiling at once is the relay's 300. The rest only wait
/// for a later tick: 28 drops a day (1 Nov 2026) would take seconds to drain.
const LOOKUPS_PER_TICK: usize = 1;

/// An old post still owed its edit, its seeds' removal or its one reply, or
/// still watched for a tap. In state.json, so a restart resumes it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Replaced {
    pub item: String,
    pub channel: String,
    pub card: Posted,
    /// The thread root it was posted under, if any.
    pub root: Option<String>,
    /// Bench clock when it was replaced.
    pub at: u64,
    /// The item as the post shows it, until the post is edited.
    pub shown: Option<Box<item::Item>>,
    /// Owed its edit, before its seeds, from its own text on the relay: its
    /// item already holds the newer Drop (`retired`).
    #[serde(default)]
    pub from_post: bool,
    pub seeds_off: bool,
    /// Read for a tap: the newest old post of its item, not told yet.
    pub watch: bool,
    /// The owner's keycaps that are no new tap: the answer it was left behind
    /// with, whatever its clock says, and what its first read found from before.
    pub seen: Vec<String>,
    /// Its first read is done.
    pub read: bool,
    /// The key behind a new tap, until its one reply is on the relay.
    pub tapped: Option<String>,
    /// Set once an update failed, until one goes through: not before this.
    pub retry_at: u64,
}

impl Replaced {
    fn new(item: &str, channel: &str, card: &Posted, root: Option<&str>, now: u64) -> Self {
        Replaced {
            item: item.to_string(),
            channel: channel.to_string(),
            card: card.clone(),
            root: root.map(str::to_string),
            at: now,
            shown: None,
            from_post: false,
            seeds_off: false,
            watch: true,
            seen: Vec::new(),
            read: false,
            tapped: None,
            retry_at: 0,
        }
    }

    fn idle(&self) -> bool {
        self.shown.is_none() && self.seeds_off && self.tapped.is_none()
    }
}

/// Watch `r`, and no older post of its item any more. An older post it ends
/// the watch of still gets its edit, its seeds off and any reply it is owed.
fn add(s: &mut BenchState, r: Replaced) {
    for old in s.replaced.iter_mut().filter(|o| o.item == r.item) {
        old.watch = false;
    }
    s.replaced.push(r);
}

/// A new Drop left the card of the item it replaces behind (`carry_card`): it
/// has no card, the item still in state does. Edited from what it shows, its
/// Recorded line or receipt included.
pub(super) fn left_behind(s: &mut BenchState, new: &item::Item, now: u64) {
    let Some(old) = s.items.get(&new.id).filter(|_| new.card.is_none()) else {
        return;
    };
    let Some(card) = &old.card else { return };
    let asked = old.request.as_deref().unwrap_or(old);
    let ch = card_channel(s, old);
    let r = Replaced {
        shown: Some(Box::new(old.clone())),
        seen: asked.answer.iter().map(|a| a.reaction_id.clone()).collect(),
        ..Replaced::new(&new.id, &ch, card, asked.reply_to.as_deref(), now)
    };
    add(s, r);
}

/// A stale card publish (b) just retired: its item holds the newer Drop, so it
/// is edited from its own text as posted.
pub(super) fn retired(s: &mut BenchState, id: &str, ch: &str, card: &Posted, now: u64) {
    let r = Replaced {
        from_post: true,
        ..Replaced::new(id, ch, card, None, now)
    };
    add(s, r);
}

fn asked(i: &item::Item) -> bool {
    i.kind == item::ItemKind::Pending && i.validity == item::Validity::Current
}

/// An old post's text from what it shows: its first line says a tap doesn't
/// count, then `render_closed`, which adds that a reply doesn't either.
pub(super) fn old_post(s: &BenchState, id: &str, shows: &str) -> String {
    let first = match s.items.get(id).is_some_and(asked) {
        true => REPLACED,
        false => NOT_ASKED,
    };
    format!("{first}\n{}", item::render_closed(shows))
}

/// The time an old post's text shows: its answer's, else when its Drop came in
/// (a receipt's Outcome). The tick that replaced it only changed its first line.
fn shown_at(i: &item::Item, hhmm: &str) -> String {
    let answered = i.answer.as_ref().map(|a| a.hhmm.as_str());
    let came_in = chrono::DateTime::parse_from_rfc3339(&i.updated_at).ok();
    match (answered.filter(|t| !t.is_empty()), came_in) {
        (Some(t), _) => t.to_string(),
        (None, Some(t)) => super::local_hhmm(t.timestamp() as u64),
        (None, None) => hhmm.to_string(),
    }
}

/// The one reply to a tap on an old post, pointing to where a tap counts.
fn told(s: &BenchState, r: &Replaced) -> String {
    let said = "That tap didn't count: this card is no longer read, so nothing was recorded.";
    let Some(i) = s.items.get(&r.item).filter(|i| asked(i)) else {
        return format!("{said} This is no longer asked.");
    };
    match super::live_card(i) {
        Some(c) => format!(
            "{said} The live card is the one that counts:{}",
            item::thread_line(&card_channel(s, i), &c.event_id)
        ),
        None => format!("{said} The newer card isn't up yet; the board shows it when it is."),
    }
}

/// The board's lines when a limit here bites; none, and `section` hides them.
pub(super) fn alarm(s: &BenchState, now: u64) -> Vec<String> {
    let watched = s
        .replaced
        .iter()
        .filter(|r| r.watch && ready(s, &r.channel))
        .count();
    let over = watched.saturating_sub(WATCH_MAX);
    let stuck = s.replaced.iter().filter(|r| r.retry_at != 0).count();
    let mut lines = Vec::new();
    if s.replaced_unread
        .is_some_and(|since| now >= since + READ_ALARM_SECS)
    {
        lines.push(
            "Can't read them for taps right now, so a tap on one gets no reply. Tried again \
             every tick: answer on the live card."
                .to_string(),
        );
    }
    if over > 0 {
        lines.push(format!(
            "Watched for a tap: {watched}. Only the newest {WATCH_MAX} are read, so a tap on any \
             of the other {over} gets no reply. Each says on its first line that a tap there \
             doesn't count: answer on the live card."
        ));
    }
    if stuck > 0 {
        lines.push(format!(
            "Would not update: {stuck}. These may still look live: answer on the live card. \
             Tried again every minute, for {} h from when each was replaced.",
            WATCH_SECS / 3600
        ));
    }
    lines
}

/// Read the watched posts for keycaps from the owner's keys. The answer it was
/// left behind with, and any it had at its first read from before it was
/// replaced, are no new tap (a keycap's time is the tapping device's, so it only
/// decides that first read: `at` is when this tick began, so a tap since the
/// live poll's last read still counts as new); any other keycap, whatever its
/// clock says, is.
async fn read(ctx: &Ctx<'_>, s: &mut BenchState, now: u64, owner_set: &[String]) {
    let watched: Vec<usize> = (0..s.replaced.len())
        .rev()
        .filter(|&i| s.replaced[i].watch && ready(s, &s.replaced[i].channel))
        .take(WATCH_MAX)
        .collect();
    if watched.is_empty() {
        s.replaced_unread = None;
        return;
    }
    let (cards, chans): (Vec<&str>, BTreeSet<&str>) = watched
        .iter()
        .map(|&i| &s.replaced[i])
        .map(|r| (r.card.event_id.as_str(), r.channel.as_str()))
        .unzip();
    let filter = serde_json::json!({
        "kinds": [7], "authors": owner_set, "#h": chans, "#e": cards, "limit": RECOVER_LIMIT,
    });
    let events = match query(ctx, s, filter, now).await {
        Ok(events) => events,
        // Read whole again next tick, so nothing is lost by this one.
        Err(e) => {
            eprintln!("bench: old cards not read: {e}");
            s.replaced_unread.get_or_insert(now);
            return;
        }
    };
    s.replaced_unread = None;
    for i in watched {
        let r = &mut s.replaced[i];
        let first = !std::mem::replace(&mut r.read, true);
        for e in &events {
            let (id, by) = (e.id.to_hex(), e.pubkey.to_hex());
            let tap = e.kind.as_u16() == 7
                && item::reaction_target(e) == Some(r.card.event_id.as_str())
                && item::OPTION_EMOJI.contains(&e.content.as_str())
                && owner_set.contains(&by);
            if !tap || r.seen.contains(&id) {
                continue;
            }
            if first && e.created_at.as_secs() < r.at {
                r.seen.push(id);
            } else {
                r.tapped.get_or_insert(by);
            }
        }
    }
}

/// What the old post `i` is still owed, in order: the edit, its seeds off (the
/// relay's own untap: a kind 5 by the reaction's author, and a removed one
/// never comes back from /query, so this resumes), its one reply. Ok(false):
/// the tick's budget or lookups ran out first. The relay is asked before the
/// reply goes, so a crash after it landed and before state saved never sends
/// it twice. A post no longer there is an Err saying "not found" (`tick`).
async fn finish(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    i: usize,
    budget: &mut usize,
    lookups: &mut usize,
    now: u64,
    hhmm: &str,
) -> Result<bool, String> {
    let r = s.replaced[i].clone();
    let (ch, card) = (&r.channel, &r.card.event_id);
    if r.from_post {
        if *budget == 0 || !spend(lookups, 1) {
            return Ok(false);
        }
        let filter = serde_json::json!({ "ids": [card], "kinds": [9], "#h": [ch] });
        let posted = query_writer(ctx, s, filter, now).await?;
        let e = posted.first().ok_or("not found: no such post")?;
        let text = old_post(s, &r.item, &e.content) + &item::thread_line(ch, card);
        spend(budget, 1);
        edit(ctx, s, ch, card, &text, now).await?;
        // Its thread root ends older cards' root-reply windows there.
        let root = item::parent(e).map(str::to_string);
        s.kept_at
            .extend(root.clone().map(|root| (root, r.card.created_at)));
        let r = &mut s.replaced[i];
        (r.from_post, r.root) = (false, root);
        return Ok(true);
    }
    if let Some(shown) = &r.shown {
        if !spend(budget, 1) {
            return Ok(false);
        }
        let text = item::render_card(shown, &shown_at(shown, hhmm));
        let text = old_post(s, &r.item, &text) + &item::thread_line(ch, card);
        edit(ctx, s, ch, card, &text, now).await?;
        s.replaced[i].shown = None;
    }
    let writer = ctx.writer.public_key().to_hex();
    let mine = |kind: u16| {
        serde_json::json!({
            "kinds": [kind], "authors": [writer], "#h": [ch], "#e": [card],
            "limit": RECOVER_LIMIT,
        })
    };
    if !r.seeds_off {
        if *budget == 0 || !spend(lookups, 1) {
            return Ok(false);
        }
        for seed in query_writer(ctx, s, mine(7), now).await? {
            if seed.kind.as_u16() != 7 || item::reaction_target(&seed) != Some(card.as_str()) {
                continue;
            }
            if !spend(budget, 1) {
                return Ok(false);
            }
            let off = events::build_remove_reaction(seed.id)?;
            match post_as_writer(ctx, s, off, now).await {
                Err(e) if !e.contains("not found") => return Err(e),
                _ => {}
            }
        }
        s.replaced[i].seeds_off = true;
    }
    if let Some(by) = &r.tapped {
        if *budget == 0 || !spend(lookups, 1) {
            return Ok(false);
        }
        let marker = format!("bench:replaced:{}@{}", r.item, &card[..16]);
        let landed = query_writer(ctx, s, mine(9), now).await?.iter().any(|e| {
            e.tags
                .iter()
                .any(|t| matches!(t.as_slice(), [k, m, ..] if k == "client" && *m == marker))
        });
        if !landed {
            if !spend(budget, 1) {
                return Ok(false);
            }
            let text = told(s, &r);
            let thread = thread_of(Some(&r.card), r.root.as_deref())?;
            post_message(
                ctx,
                s,
                ch,
                &text,
                &[by.as_str()],
                thread.as_ref(),
                marker,
                Vec::new(),
                now,
            )
            .await?;
        }
        let r = &mut s.replaced[i];
        r.tapped = None;
        r.watch = false;
    }
    Ok(true)
}

/// One tick's work on the old posts, after publish: the live cards go first,
/// and only what budget they left goes on these.
pub(super) async fn tick(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
    hhmm: &str,
    owner_set: &[String],
) {
    // Past 30 failing the loop runs out of lookups every tick and never reaches
    // the retain below, so one that has failed goes at its window here.
    s.replaced
        .retain(|r| r.retry_at == 0 || now < r.at + WATCH_SECS);
    read(ctx, s, now, owner_set).await;
    let mut lookups = LOOKUPS_PER_TICK;
    for i in 0..s.replaced.len() {
        let r = &s.replaced[i];
        if r.idle() || now < r.retry_at || !ready(s, &r.channel) {
            continue;
        }
        match finish(ctx, s, i, budget, &mut lookups, now, hhmm).await {
            // Not `break`: the retain would drop what is past its window untried.
            Ok(false) => return,
            Ok(true) => s.replaced[i].retry_at = 0,
            // Gone from the relay: nothing is left to edit or to reply under.
            Err(e) if e.contains("not found") => {
                let r = &mut s.replaced[i];
                (r.shown, r.tapped, r.seeds_off, r.watch) = (None, None, true, false);
                r.retry_at = 0;
            }
            Err(e) => {
                eprintln!("bench: old card {}: {e}", s.replaced[i].card.event_id);
                s.replaced[i].retry_at = now + RETRY_SECS;
            }
        }
    }
    // Past its window every post goes, whatever it was still owed: it had its
    // try above, and one that keeps failing must not retry for good.
    s.replaced
        .retain(|r| now < r.at + WATCH_SECS && !(r.idle() && !r.watch));
}
