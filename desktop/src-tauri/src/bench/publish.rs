//! The relay posts of one tick: Recorded and receipt edits, answer and reply
//! notices, card retirement and posting, seeds, then the board. Every step
//! lands in state first, so the next tick resumes where this one stopped.

use std::collections::BTreeMap;

use super::channels::{card_channel, channel_ready, joined};
use super::{
    forget_card, item, live_card, local_hhmm, next_ts, post_as_owner, post_as_writer, query,
    same_drop, spend, BenchState, Ctx, Outcome, Posted, BOARD_MAX_AGE_SECS, MAX_CARDS,
    RECOVER_LIMIT,
};
use crate::events;

/// A kind 9 carrying its `client` marker; the p tags only for whom it pings
/// (a card's answerers or the owner, a notice's route target). `extra` tags
/// go on unchecked: only the Bench's own `df-` tags. `as_owner`: signed by
/// the owner's identity (`item::render_owner_route`), still on the
/// monotonic clock so the board stays newest.
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
    as_owner: bool,
) -> Result<Outcome, String> {
    let ch = uuid::Uuid::parse_str(ch).map_err(|e| e.to_string())?;
    let builder = events::build_message_with_client_tags(
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
    .tags(extra);
    if as_owner {
        return post_as_owner(ctx, builder.custom_created_at(next_ts(s, now)?)).await;
    }
    post_as_writer(ctx, s, builder, now).await
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
pub(super) async fn publish(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
    hhmm: &str,
    routes: &BTreeMap<String, String>,
    owner_set: &[String],
) -> Result<(), String> {
    // (a) The live cards: pending + current + unanswered by severity, in a
    // channel ready for them; an answered card counts against MAX_CARDS (the
    // phone's pill window) for ANSWER_KEEP_SECS so late pills land, then
    // stays on the relay as a receipt. Receipts do not count.
    let mut ranked: Vec<(u8, i64, String)> = s
        .items
        .values()
        .filter(|i| {
            i.kind == item::ItemKind::Pending
                && i.validity == item::Validity::Current
                && i.answer.is_none()
                && channel_ready(s, i)
        })
        .map(|i| (item::severity_rank(i.severity), i.order, i.id.clone()))
        .collect();
    ranked.sort();
    let kept = s
        .items
        .values()
        .filter(|i| {
            i.kind == item::ItemKind::Pending
                && i.card.is_some()
                && i.answer
                    .as_ref()
                    .is_some_and(|a| now.saturating_sub(a.answered_at) <= item::ANSWER_KEEP_SECS)
        })
        .count();
    let wanted: Vec<String> = ranked
        .into_iter()
        .take(MAX_CARDS.saturating_sub(kept))
        .map(|r| r.2)
        .collect();

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
    // each new free-text reply on a scoped card, in the card's thread so the
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
        if now < s.routes_held_until || s.held.contains(&id) {
            continue;
        }
        let card = live_card(&item)
            .map(|c| nostr::EventId::from_hex(&c.event_id))
            .transpose()
            .map_err(|e| e.to_string())?;
        let root = item
            .reply_to
            .as_deref()
            .map(nostr::EventId::from_hex)
            .transpose()
            .map_err(|e| e.to_string())?;
        let thread = card.map(|c| events::ThreadRef {
            root_event_id: root.unwrap_or(c),
            parent_event_id: c,
        });
        let link = live_card(&item).map_or(String::new(), |c| item::thread_line(&ch, &c.event_id));
        let ping = [to.as_str()];
        if item::route_due(&item, now) {
            if !spend(budget, 1) {
                return Ok(());
            }
            let prev = item.routed.as_ref().and_then(|r| r.option);
            let at = item
                .answer
                .as_ref()
                .map_or(item.changed_at, |a| a.answered_at);
            // The owner's own answer goes out as the owner's own message, so
            // the agent acts on the owner's word. One told that way and now
            // withdrawn or someone else's is taken back that way first, as a
            // withdrawal (the other answer follows next tick). Anyone else's
            // answer, any other withdrawal, or text that fails the checks
            // again is the writer's.
            let mine = item
                .answer
                .as_ref()
                .is_some_and(|a| owner_set.contains(&a.by));
            let taken_back = item
                .routed
                .as_ref()
                .filter(|r| r.owner && !mine)
                .and_then(|r| item::render_owner_withdrawal(&item, r.option?));
            let answer = item.answer.as_ref().filter(|_| taken_back.is_none());
            let want = answer.map(|a| a.option);
            let owners =
                taken_back.or_else(|| item::render_owner_route(&item, prev).filter(|_| mine));
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
                as_owner,
            );
            let outcome = match posted.await {
                // The card was deleted by hand: the notice goes top-level next tick.
                Err(e) if thread.is_some() && e.contains("not found") => {
                    forget_card(s, &id);
                    continue;
                }
                r => r?,
            };
            // A top-level notice is now newer than the board.
            s.board_dirty |= thread.is_none() && ch == s.channel_id;
            let event_id = match outcome {
                Outcome::Accepted { event_id } => event_id,
                // Already on the relay: told all the same.
                Outcome::Duplicate => String::new(),
            };
            s.items.get_mut(&id).expect("routed id").routed = Some(item::Routed {
                option: want,
                to: to.clone(),
                event_id,
                at: now,
                owner: as_owner && want.is_some(),
            });
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
        let text = item::render_route_reply(&item, &local_hhmm(r.at)) + &link;
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
            false,
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
    // no longer wanted, unless someone answered or replied on them or they
    // are a receipt: those stay on the relay for good. A card in a channel
    // that is not ready, or `held`, waits: its taps are read once it is again.
    let stale: Vec<(String, String, String)> = s
        .items
        .values()
        .filter_map(|i| {
            let c = i.card.as_ref()?;
            let stale = c.hash != item::card_hash(i) || !wanted.contains(&i.id);
            let waits = !channel_ready(s, i) || s.held.contains(&i.id);
            let retire = !i.engaged() && i.request.is_none() && stale && !waits;
            retire.then(|| (i.id.clone(), card_channel(s, i), c.event_id.clone()))
        })
        .collect();
    for (id, ch, event_id) in stale {
        if !spend(budget, 1) {
            return Ok(());
        }
        delete(ctx, s, &ch, &event_id, now).await?;
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
            false,
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
                let i = s.items.get_mut(id).expect("wanted id");
                i.card = Some(Posted {
                    event_id,
                    created_at,
                    hash,
                    seeded: 0,
                });
                i.quiet_repost = false;
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
    let mut text = item::render_board(&all, &s.channel_id, hhmm, now);
    let refused = s
        .channels
        .iter()
        .filter_map(|(ch, c)| Some(format!("- {ch}: {}", c.refused.as_ref()?)));
    item::section(&mut text, "Cards refused", refused);
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
        match post_message(
            ctx,
            s,
            &bench,
            &text,
            &[],
            None,
            marker,
            Vec::new(),
            now,
            false,
        )
        .await?
        {
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
