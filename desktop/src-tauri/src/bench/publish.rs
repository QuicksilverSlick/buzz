//! The relay posts of one tick: cards, seeds and the board, through the
//! writer's kind 9, 5 and 40003 helpers.

use std::collections::BTreeMap;

use super::{
    channel_id, forget_card, item, live_card, local_hhmm, post_as_writer, same_drop, spend,
    BenchState, Ctx, Outcome, Posted, BOARD_MAX_AGE_SECS, MAX_CARDS,
};
use crate::events;

/// A kind 9 in #bench carrying its `client` marker; the p tag only for whom
/// it pings (the owner for a card, the route target for a notice).
async fn post_message(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    text: &str,
    ping: Option<&str>,
    thread: Option<&events::ThreadRef>,
    marker: String,
    now: u64,
) -> Result<Outcome, String> {
    let builder = events::build_message_with_client_tags(
        channel_id(&s.owner_pubkey),
        text,
        thread,
        ping.as_slice(),
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

/// kind 40003 with h+e replacing the text of one of the writer's own kind 9s.
async fn edit(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    event_id: &str,
    text: &str,
    now: u64,
) -> Result<Outcome, String> {
    let target = nostr::EventId::from_hex(event_id).map_err(|e| e.to_string())?;
    let edit_tags = events::MessageEditTags {
        media: &[],
        custom_emoji: &[],
        mentions: &[],
        mention_refs: None,
    };
    let builder =
        events::build_message_edit(channel_id(&s.owner_pubkey), target, text, edit_tags, false)?;
    post_as_writer(ctx, s, builder, now).await
}

/// Cards, seeds, then the board, at most `budget` posts; the next tick
/// resumes where this one stopped because every step lands in state first.
/// Cards are always delete + repost; only the board is ever edited, and the
/// one-shot Recorded edit on an answered card.
pub(super) async fn publish(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
    hhmm: &str,
    routes: &BTreeMap<String, String>,
) -> Result<(), String> {
    // (a) The live cards: pending + current + unanswered by severity; kept
    // answered cards count against MAX_CARDS (the phone's pill window).
    let mut ranked: Vec<(u8, i64, String)> = s
        .items
        .values()
        .filter(|i| {
            i.kind == item::ItemKind::Pending
                && i.validity == item::Validity::Current
                && i.answer.is_none()
        })
        .map(|i| (item::severity_rank(i.severity), i.order, i.id.clone()))
        .collect();
    ranked.sort();
    let kept = s
        .items
        .values()
        .filter(|i| i.answer.is_some() && i.card.is_some())
        .count();
    let wanted: Vec<String> = ranked
        .into_iter()
        .take(MAX_CARDS.saturating_sub(kept))
        .map(|r| r.2)
        .collect();

    // (a2) Record / un-record on the live card, first so a tap is acknowledged
    // within one tick. Diff-driven from what the card text shows (`recorded`)
    // vs the answer, so it is a no-op after a restart.
    for id in s.items.keys().cloned().collect::<Vec<_>>() {
        let item = s.items[&id].clone();
        let Some(card) = live_card(&item) else {
            continue;
        };
        let want = item.answer.as_ref().map(|a| a.option);
        if item.recorded == want {
            continue;
        }
        if !spend(budget, 1) {
            return Ok(());
        }
        let text = item::render_card(&item, hhmm);
        match edit(ctx, s, &card.event_id, &text, now).await {
            // The edit re-rendered the whole card, so it now shows this card
            // format too: (b) must not retire it for an older one.
            Ok(_) => {
                let i = s.items.get_mut(&id).expect("edited id");
                i.recorded = want;
                i.card.as_mut().expect("edited card").hash = item::card_hash(&item);
            }
            // A hand-deleted card: the answer stays and is never reposted; an
            // unanswered one comes back through (c).
            Err(e) if e.contains("not found") => forget_card(s, &id),
            Err(e) => return Err(e),
        }
    }

    // (a3) Tell the writer's route target of a settled answer change, in the
    // card's thread so the board stays newest. `routed` lands only once the
    // relay took the notice, so a failed post is retried next tick.
    for id in s.items.keys().cloned().collect::<Vec<_>>() {
        let item = s.items[&id].clone();
        let Some(to) = routes
            .get(&item.writer)
            .filter(|to| s.members_added.contains(to))
        else {
            continue;
        };
        if now < s.routes_held_until || !item::route_due(&item, now) {
            continue;
        }
        if !spend(budget, 1) {
            return Ok(());
        }
        let want = item.answer.as_ref().map(|a| a.option);
        let prev = item.routed.as_ref().and_then(|r| r.option);
        let at = item
            .answer
            .as_ref()
            .map_or(item.changed_at, |a| a.answered_at);
        let text = item::render_route(&item, prev, &local_hhmm(at));
        let n = want.map_or("withdrawn".to_string(), |i| (i + 1).to_string());
        let thread = live_card(&item)
            .map(|c| nostr::EventId::from_hex(&c.event_id))
            .transpose()
            .map_err(|e| e.to_string())?
            .map(|e| events::ThreadRef {
                root_event_id: e,
                parent_event_id: e,
            });
        let marker = format!("bench:route:{id}@{n}");
        let outcome = match post_message(
            ctx,
            s,
            &text,
            Some(to.as_str()),
            thread.as_ref(),
            marker,
            now,
        )
        .await
        {
            // The card was deleted by hand: the notice goes top-level next tick.
            Err(e) if thread.is_some() && e.contains("not found") => {
                forget_card(s, &id);
                continue;
            }
            r => r?,
        };
        // A top-level notice is now newer than the board.
        s.board_dirty |= thread.is_none();
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
        });
    }

    // (b) Delete cards that are stale (another Drop or card format) or no
    // longer wanted. An answered card is exempt for ANSWER_KEEP_SECS so late
    // pills land, then retired here; it is never refreshed for format alone.
    let stale: Vec<(String, String)> = s
        .items
        .values()
        .filter_map(|i| {
            let c = i.card.as_ref()?;
            let retire = match &i.answer {
                Some(a) => now.saturating_sub(a.answered_at) > item::ANSWER_KEEP_SECS,
                None => c.hash != item::card_hash(i) || !wanted.contains(&i.id),
            };
            retire.then(|| (i.id.clone(), c.event_id.clone()))
        })
        .collect();
    for (id, event_id) in stale {
        if !spend(budget, 1) {
            return Ok(());
        }
        delete(ctx, s, &event_id, now).await?;
        forget_card(s, &id);
    }

    // (c) Post missing cards, adopting a recovered orphan when its hash matches.
    for id in &wanted {
        if s.items[id].card.is_some() {
            continue;
        }
        if let Some(orphan) = s.orphan_cards.get(id).cloned() {
            if orphan.hash == item::card_hash(&s.items[id]) {
                s.items.get_mut(id).expect("wanted id").card = s.orphan_cards.remove(id);
                continue;
            }
            if !spend(budget, 1) {
                return Ok(());
            }
            delete(ctx, s, &orphan.event_id, now).await?;
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
        let owner = s.owner_pubkey.clone();
        let ping = (item::ping_owner(&item) && !item.quiet_repost).then_some(owner.as_str());
        match post_message(ctx, s, &text, ping, None, marker, now).await? {
            Outcome::Accepted { event_id } => {
                let created_at = s.last_created_at;
                let i = s.items.get_mut(id).expect("wanted id");
                i.card = Some(Posted {
                    event_id,
                    created_at,
                    hash,
                    seeded: 0,
                });
                i.quiet_repost = false;
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
    let text = item::render_board(&all, &s.channel_id, hhmm, now);
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
        let marker = format!("bench:board@{hash}");
        match post_message(ctx, s, &text, None, None, marker, now).await? {
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
    } else if let Some(board_id) = s
        .board
        .as_ref()
        .filter(|b| b.hash != hash)
        .map(|b| b.event_id.clone())
    {
        if !spend(budget, 1) {
            return Ok(());
        }
        match edit(ctx, s, &board_id, &text, now).await {
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
