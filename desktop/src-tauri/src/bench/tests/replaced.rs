//! Old posts: a replaced card says so, loses its seeds and answers a tap once,
//! and a tap there never counts. Child of tests.rs so the fake relay and Bench
//! harness are in scope.

use super::answers::{last_query, outbox, posted, reply, tap, ITEM};
use super::*;
use crate::bench::replaced::{closed, WATCH_MAX};

const REPLACED: &str = "Replaced by a newer card. A tap here doesn't count.";

fn content(ev: &Value) -> &str {
    ev["content"].as_str().unwrap()
}

fn redrop(b: &Bench, title: &str) {
    let drop = drop_json(&[("title", json!(title))]);
    b.drop_file("orchestrator", "relay-choice", &drop);
}

/// The post the Bench last edited `card` to.
fn last_edit(b: &Bench, card: &str) -> Value {
    b.posts()
        .into_iter()
        .rev()
        .find(|(e, _)| e["kind"] == 40003 && tag_value(e, "e").as_deref() == Some(card))
        .unwrap()
        .0
}

/// The writer's seeds on `card`, as the relay hands them back.
fn seeds(b: &Bench, card: &str) -> Vec<nostr::Event> {
    b.posts()
        .into_iter()
        .filter(|(e, _)| e["kind"] == 7 && tag_value(e, "e").as_deref() == Some(card))
        .map(|(e, _)| serde_json::from_value(e).unwrap())
        .collect()
}

/// Every event a kind 5 of the Bench's named.
fn removed(b: &Bench) -> Vec<String> {
    b.posts()
        .iter()
        .filter(|(e, _)| e["kind"] == 5)
        .filter_map(|(e, _)| tag_value(e, "e"))
        .collect()
}

/// Every reply the Bench made to a tap on an old post.
fn replies(b: &Bench) -> Vec<Value> {
    b.posts()
        .into_iter()
        .map(|(e, _)| e)
        .filter(|e| e["kind"] == 9)
        .filter(|e| tag_value(e, "client").is_some_and(|m| m.starts_with("bench:replaced:")))
        .collect()
}

/// Whether a /query names `id` in its `#e`.
fn names(q: &Value, id: &str) -> bool {
    q[0]["#e"]
        .as_array()
        .is_some_and(|ids| ids.contains(&json!(id)))
}

/// An owner's answer on the seeded card, then a new question: the old card is
/// left behind. Returns it and that answer's tap, all on the fake relay.
async fn answered_then_replaced() -> (Bench, Posted, nostr::Event) {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM);
    let answer = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&answer]);
    b.tick().await.unwrap();
    assert!(b.s.items[ITEM].answer.is_some());
    redrop(&b, "Relay choice, reworded");
    b.settle(3).await;
    assert_ne!(b.card(ITEM).event_id, old.event_id);
    (b, old, answer)
}

#[tokio::test]
async fn a_re_ask_edits_the_old_post_first_line_and_takes_its_seeds_off() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM);
    let on_card = seeds(&b, &old.event_id);
    assert_eq!(on_card.len(), 2);
    let answer = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[1], b.now);
    let mut relay: Vec<&nostr::Event> = on_card.iter().collect();
    relay.push(&answer);
    reply(&b, &relay);
    b.tick().await.unwrap();
    redrop(&b, "Relay choice, reworded");
    b.settle(3).await;

    let edited = last_edit(&b, &old.event_id);
    let text = content(&edited);
    assert!(
        text.starts_with(&format!("{REPLACED}\n🔴 · Dreamforge · orchestrator\n")),
        "{text}"
    );
    // What it showed stays: the question it asked and what was recorded on it.
    assert!(text.contains("**Relay choice**"), "{text}");
    assert!(text.contains("\nRecorded: 2. Move · "), "{text}");
    assert!(text.contains("\n\nClosed: a tap or reply here is no longer read."));
    assert!(!text.contains("Tap one number"), "{text}");
    assert!(text.ends_with(&thread_line(&b.channel(), &old.event_id)));

    // Its seeds go, once each; the owner's answer is his to take back.
    b.settle(3).await;
    let off = removed(&b);
    for seed in &on_card {
        let n = off.iter().filter(|id| **id == seed.id.to_hex()).count();
        assert_eq!(n, 1, "{off:?}");
    }
    assert!(!off.contains(&answer.id.to_hex()));
    // The answer it already had is no new tap.
    assert!(replies(&b).is_empty());
    assert!(b.s.replaced[0].watch);
}

#[tokio::test]
async fn a_touched_stale_card_says_so_first_too() {
    for (drop, first) in [
        (vec![("options", json!(["Stay", "Move away"]))], REPLACED),
        (
            vec![("validity", json!("superseded"))],
            "No longer asked. A tap here doesn't count.",
        ),
    ] {
        let mut b = seeded_bench(Gate::Open, &[]).await;
        let old = b.card(ITEM);
        let heart = tap(&b.ctx.owner, &old.event_id, "\u{2764}\u{FE0F}", b.now);
        reply(&b, &[&heart, &posted(&b, &old.event_id)]);
        b.tick().await.unwrap();
        assert!(!b.s.items[ITEM].engaged());
        b.drop_file("orchestrator", "relay-choice", &drop_json(&drop));
        b.settle(3).await;
        let edited = last_edit(&b, &old.event_id);
        let text = content(&edited);
        assert!(text.starts_with(&format!("{first}\n🔴 · ")), "{text}");
        assert!(text.contains("**Relay choice**"), "{text}");
        assert!(!text.contains("Tap one number"), "{text}");
        assert_eq!(b.s.kept, [old.event_id.as_str()]);
        assert_eq!(b.s.replaced[0].card.event_id, old.event_id);
        // A heart is no keycap, so it is told nothing.
        assert!(replies(&b).is_empty());
    }
}

/// `pending_reword_deletes_and_reposts_never_edits` pins the post; a deleted
/// card has nothing left to say so on, and the relay refuses a tap on it.
#[tokio::test]
async fn an_untouched_stale_card_is_only_deleted() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM).event_id;
    redrop(&b, "Relay choice, reworded");
    b.settle(3).await;
    assert!(removed(&b).contains(&old));
    assert!(b.s.replaced.is_empty());
    assert!(b.s.kept.is_empty());
}

#[tokio::test]
async fn a_tap_on_an_old_post_gets_one_reply_and_counts_for_nothing() {
    let (mut b, old, answer) = answered_then_replaced().await;
    let new = b.card(ITEM);
    let stray = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &stray]);
    b.settle(1).await;

    let told = replies(&b);
    assert_eq!(told.len(), 1);
    let reply_to = &told[0];
    // Writer-signed, in the old card's thread, and it reaches who tapped.
    assert_eq!(reply_to["pubkey"], b.writer_hex());
    assert_eq!(tag_value(reply_to, "e").unwrap(), old.event_id);
    assert_eq!(tag_value(reply_to, "p").unwrap(), b.owner_hex());
    assert_eq!(tag_value(reply_to, "h").unwrap(), b.channel());
    let text = content(reply_to);
    assert!(text.starts_with("That tap didn't count"), "{text}");
    assert!(
        text.ends_with(&thread_line(&b.channel(), &new.event_id)),
        "{text}"
    );

    // Never an answer, on either card, and nothing for the writer or the kit.
    let item = &b.s.items[ITEM];
    assert!(item.answer.is_none() && item.reply.is_none() && item.recorded.is_none());
    assert_eq!(item.card.as_ref().unwrap().event_id, new.event_id);
    assert!(outbox(&b).is_none());

    // Once per old card, and it is read no more.
    b.settle(5).await;
    assert_eq!(replies(&b).len(), 1);
    assert!(b.s.replaced.is_empty());
    assert_ne!(last_query(&b)[0]["#e"], json!([old.event_id]));
    // It is no stray to a relaunch, which never adopts the old card back.
    let mut relay = vec![posted(&b, &new.event_id), posted(&b, &old.event_id)];
    relay.push(serde_json::from_value(reply_to.clone()).unwrap());
    reply(&b, &relay.iter().collect::<Vec<_>>());
    b.s.needs_rebuild = true;
    b.tick().await.unwrap();
    assert!(b.s.strays.is_empty(), "{:?}", b.s.strays);
    assert_eq!(b.card(ITEM).event_id, new.event_id);
}

/// The live card's flow as if the old post were not there: its poll names the
/// live card alone, and its tap lands whole in the outbox.
#[tokio::test]
async fn the_live_card_is_read_as_ever_beside_a_watched_old_post() {
    let (mut b, old, answer) = answered_then_replaced().await;
    let new = b.card(ITEM);
    assert!(b.s.replaced[0].watch);
    let live = tap(&b.ctx.owner, &new.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &live]);
    let before = b.relay.queries.lock().unwrap().len();
    b.tick().await.unwrap();

    let queries = b.relay.queries.lock().unwrap()[before..].to_vec();
    let polls: Vec<&Value> = queries.iter().filter(|q| names(q, &new.event_id)).collect();
    assert_eq!(polls.len(), 1, "{queries:?}");
    assert_eq!(polls[0][0]["#e"], json!([new.event_id]));
    assert_eq!(polls[0][0]["kinds"], json!([7]));
    assert_eq!(
        polls[0][0]["authors"],
        json!([b.owner_hex(), "cd".repeat(32)])
    );
    let watch = queries.iter().find(|q| names(q, &old.event_id)).unwrap();
    assert_eq!(watch[0]["#e"], json!([old.event_id]));

    let picked = b.s.items[ITEM].answer.clone().unwrap();
    assert_eq!(picked.option, 0);
    assert_eq!(picked.reaction_id, live.id.to_hex());
    let file = outbox(&b).unwrap();
    assert_eq!(file["card"], new.event_id);
    assert_eq!(file["option"], 1);
    assert_eq!(file["reaction"]["id"], live.id.to_hex());
    assert_eq!(file["answerer"], b.owner_hex());
    assert_eq!(file["source"], "tap");
    assert!(replies(&b).is_empty());
}

#[tokio::test]
async fn no_reply_twice_across_a_restart() {
    let (mut b, old, answer) = answered_then_replaced().await;
    let stray = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &stray]);
    // Killed once the reply landed, before state.json learned of it.
    let before = serde_json::to_string(&b.s).unwrap();
    b.settle(1).await;
    assert_eq!(replies(&b).len(), 1);
    b.s = serde_json::from_str(&before).unwrap();
    assert!(b.s.replaced[0].tapped.is_none());
    let landed: nostr::Event = serde_json::from_value(replies(&b)[0].clone()).unwrap();
    reply(&b, &[&answer, &stray, &landed]);
    b.settle(3).await;
    assert_eq!(replies(&b).len(), 1, "the relay says it went");
    assert!(b.s.replaced.is_empty());

    // Saved and loaded again: nothing is owed or watched any more.
    let mut written = String::new();
    save_state(&b.root, &b.s, &mut written).unwrap();
    b.s = load_state(&b.root).0;
    b.settle(3).await;
    assert_eq!(replies(&b).len(), 1);
}

/// A restart between the tap and its reply: state.json holds what is owed.
#[tokio::test]
async fn a_reply_owed_survives_a_restart() {
    let (mut b, old, answer) = answered_then_replaced().await;
    b.s.replaced[0].tapped = Some(b.owner_hex());
    let mut written = String::new();
    save_state(&b.root, &b.s, &mut written).unwrap();
    b.s = load_state(&b.root).0;
    assert_eq!(b.s.replaced[0].card.event_id, old.event_id);
    reply(&b, &[&answer]);
    b.settle(2).await;
    assert_eq!(replies(&b).len(), 1);
}

#[tokio::test]
async fn only_keycaps_that_are_new_are_a_tap() {
    let (mut b, old, answer) = answered_then_replaced().await;
    let writer = b.ctx.writer.clone();
    let other = Keys::generate();
    let heart = tap(&b.ctx.owner, &old.event_id, "\u{2764}\u{FE0F}", b.now);
    let others = tap(&other, &old.event_id, OPTION_EMOJI[0], b.now);
    let own_seed = tap(&writer, &old.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &heart, &others, &own_seed]);
    b.settle(2).await;
    assert!(replies(&b).is_empty());
    // A phone's clock runs behind: past the first read, its time decides nothing.
    let behind = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now - 300);
    reply(&b, &[&answer, &behind]);
    b.settle(1).await;
    assert_eq!(replies(&b).len(), 1);
}

/// At its first read a post may hold a tap older than `at` (no new tap) or one
/// made since the live poll last read it (new, the owner may well think it counted).
#[tokio::test]
async fn at_the_first_read_a_tap_from_before_the_replacement_is_no_new_tap() {
    for (late, want) in [(false, 0), (true, 1)] {
        let mut b = seeded_bench(Gate::Open, &[]).await;
        let old = b.card(ITEM);
        let answer = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now);
        reply(&b, &[&answer]);
        b.tick().await.unwrap();
        redrop(&b, "Relay choice, reworded");
        // The tick that takes this drop reads as of the tick's clock.
        let at = b.now + 2;
        let when = if late { at } else { at - 1 };
        let tap2 = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[1], when);
        reply(&b, &[&answer, &tap2]);
        b.settle(3).await;
        assert_eq!(replies(&b).len(), want, "late: {late}");
        assert!(b.s.items[ITEM].answer.is_none());
        assert!(outbox(&b).is_none());
    }
}

#[tokio::test]
async fn only_the_newest_old_post_of_an_item_is_watched() {
    let (mut b, first, answer) = answered_then_replaced().await;
    let second = b.card(ITEM);
    let again = tap(&b.ctx.owner, &second.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&answer, &again]);
    b.tick().await.unwrap();
    assert!(b.s.items[ITEM].answer.is_some());
    redrop(&b, "Relay choice, reworded again");
    b.settle(3).await;
    assert_ne!(b.card(ITEM).event_id, second.event_id);
    // Each said so, and only the newer is still read.
    for id in [&first.event_id, &second.event_id] {
        let edited = last_edit(&b, id);
        assert!(content(&edited).starts_with(REPLACED));
    }
    assert_eq!(last_query(&b)[0]["#e"], json!([second.event_id]));
    assert_eq!(b.s.replaced.len(), 1);
    let on_first = tap(&b.ctx.owner, &first.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &again, &on_first]);
    b.settle(2).await;
    assert!(replies(&b).is_empty());
    let on_second = tap(&b.ctx.owner, &second.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &again, &on_second]);
    b.settle(2).await;
    let told = replies(&b);
    assert_eq!(told.len(), 1);
    assert_eq!(tag_value(&told[0], "e").unwrap(), second.event_id);
}

#[tokio::test]
async fn past_its_window_an_old_post_is_read_once_more_and_let_go() {
    let (mut b, old, answer) = answered_then_replaced().await;
    let at = b.s.replaced[0].at;
    b.now = at + 86_396;
    b.tick().await.unwrap();
    assert_eq!(b.s.replaced.len(), 1);
    let late = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &late]);
    b.tick().await.unwrap();
    assert_eq!(b.now, at + 86_400);
    assert_eq!(replies(&b).len(), 1, "a tap read as it ends is told");
    assert!(b.s.replaced.is_empty());
    b.settle(1).await;
    assert_eq!(replies(&b).len(), 1);

    let (mut b, old, answer) = answered_then_replaced().await;
    b.now += 86_400;
    reply(&b, &[&answer]);
    b.tick().await.unwrap();
    assert!(b.s.replaced.is_empty());
    let queries = b.relay.queries.lock().unwrap().len();
    b.settle(1).await;
    let after = b.relay.queries.lock().unwrap()[queries..].to_vec();
    assert!(!after.iter().any(|q| names(q, &old.event_id)));
}

#[tokio::test]
async fn past_the_cap_the_oldest_go_unread_and_the_board_says_so() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let (ch, card) = (b.channel(), b.card(ITEM));
    let n = WATCH_MAX + 2;
    let hex = |i: usize| format!("{i:064x}");
    for i in 0..n {
        let old = Posted {
            event_id: hex(i),
            ..card.clone()
        };
        closed(
            &mut b.s,
            &format!("orchestrator/old-{i}"),
            &ch,
            &old,
            None,
            b.now,
        );
    }
    b.settle(2).await;
    // The poll names the one live card; this is the read of the old ones.
    let queries = b.relay.queries.lock().unwrap().clone();
    let many = |q: &&Value| q[0]["#e"].as_array().is_some_and(|ids| ids.len() > 1);
    let read = queries.iter().rev().find(many).unwrap();
    assert_eq!(read[0]["authors"], json!([b.owner_hex(), "cd".repeat(32)]));
    let cards = read[0]["#e"].as_array().unwrap();
    assert_eq!(cards.len(), WATCH_MAX);
    assert!(cards.contains(&json!(hex(n - 1))));
    assert!(!cards.contains(&json!(hex(0))) && !cards.contains(&json!(hex(1))));
    assert_eq!(read[0]["limit"], json!(RECOVER_LIMIT));
    let board = b.posts().iter().rev().find_map(|(e, _)| {
        let on_board = tag_value(e, "e") == b.s.board.as_ref().map(|c| c.event_id.clone());
        (e["kind"] == 40003 && on_board).then(|| content(e).to_string())
    });
    let board = board.unwrap();
    assert!(
        board.contains(&format!(
            "\n\nOld cards\nWatched for a tap: {n}. Only the newest {WATCH_MAX} are read, so \
             a tap on any of the other 2 gets no reply."
        )),
        "{board}"
    );
    // A tap past the cap gets no reply; one inside it does.
    let past = tap(&b.ctx.owner, &hex(0), OPTION_EMOJI[0], b.now);
    let inside = tap(&b.ctx.owner, &hex(n - 1), OPTION_EMOJI[0], b.now);
    reply(&b, &[&past]);
    b.settle(1).await;
    assert!(replies(&b).is_empty());
    reply(&b, &[&past, &inside]);
    b.settle(2).await;
    let told = replies(&b);
    assert_eq!(told.len(), 1);
    assert_eq!(tag_value(&told[0], "e").unwrap(), hex(n - 1));
}

/// lastError stops the kit's approvals, so an old post never fails a tick.
#[tokio::test]
async fn a_failing_update_fails_no_tick_and_the_board_says_so() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let (ch, card) = (b.channel(), b.card(ITEM));
    let old = Posted {
        event_id: "ab".repeat(32),
        ..card
    };
    closed(&mut b.s, "orchestrator/old", &ch, &old, None, b.now);
    b.s.replaced[0].seeds_off = true;
    b.s.replaced[0].tapped = Some(b.owner_hex());
    *b.relay.clock.lock().unwrap() = Some(b.now + 10_000);
    b.tick().await.unwrap();
    // The fake relay logs what it refuses: one try, and the tick is fine.
    assert_eq!(replies(&b).len(), 1);
    let retry = b.s.replaced[0].retry_at;
    assert_eq!(retry, b.now + 60);
    *b.relay.clock.lock().unwrap() = None;
    b.tick().await.unwrap();
    assert!(b.posts().iter().rev().any(|(e, _)| {
        e["kind"] == 40003 && content(e).contains("\n\nOld cards\nWould not update: 1.")
    }));
    assert_eq!(replies(&b).len(), 1, "not before its retry");
    b.now = retry;
    b.settle(2).await;
    assert_eq!(replies(&b).len(), 2);
    assert!(b.s.replaced.is_empty());
    let board = b
        .posts()
        .iter()
        .rev()
        .find(|(e, _)| e["kind"] == 40003)
        .cloned();
    assert!(!content(&board.unwrap().0).contains("Old cards"));
}

#[tokio::test]
async fn an_old_post_gone_from_the_relay_is_let_go() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM);
    let answer = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&answer]);
    b.tick().await.unwrap();
    b.relay.gone.lock().unwrap().insert(old.event_id.clone());
    redrop(&b, "Relay choice, reworded");
    b.settle(3).await;
    // Its edit was refused, so nothing more is tried, and no tick failed.
    assert!(b.s.replaced.is_empty());
    assert_eq!(b.s.kept, [old.event_id]);
}

#[tokio::test]
async fn a_reply_with_no_live_card_says_where_things_stand() {
    // Still asked, its newer card waiting for a slot.
    let (mut b, old, answer) = answered_then_replaced().await;
    let stray = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &stray]);
    b.s.items.get_mut(ITEM).unwrap().card = None;
    b.s.queued = vec![ITEM.to_string()];
    b.s.replaced[0].seeds_off = true;
    let mut budget = 0;
    crate::bench::replaced::tick(
        &b.ctx,
        &mut b.s,
        &mut budget,
        b.now,
        "14:05",
        &[b.ctx.owner.public_key().to_hex()],
    )
    .await;
    assert!(replies(&b).is_empty(), "no budget, so it waits");
    let mut budget = 1;
    crate::bench::replaced::tick(
        &b.ctx,
        &mut b.s,
        &mut budget,
        b.now,
        "14:05",
        &[b.ctx.owner.public_key().to_hex()],
    )
    .await;
    let told = replies(&b);
    assert_eq!(told.len(), 1);
    assert!(
        content(&told[0]).ends_with("The newer card isn't up yet; the board shows it when it is."),
        "{}",
        content(&told[0])
    );
}
