//! Old posts: a replaced card says so, loses its seeds and answers a tap once,
//! and a tap there never counts. Child of tests.rs so the fake relay and Bench
//! harness are in scope.

use super::answers::{last_query, outbox, posted, reply, tap, ITEM};
use super::channels::{admins, ask, bind, ASK, X};
use super::receipts::close_out;
use super::*;
use crate::bench::channels::ChannelCheck;
use crate::bench::replaced::{retired, tick as old_posts_tick, Replaced, WATCH_MAX};

const REPLACED: &str = "Replaced by a newer card. A tap here doesn't count.";
const NOT_ASKED: &str = "No longer asked. A tap here doesn't count.";

fn content(ev: &Value) -> &str {
    ev["content"].as_str().unwrap()
}

fn redrop(b: &Bench, title: &str) {
    let drop = drop_json(&[("title", json!(title))]);
    b.drop_file("orchestrator", "relay-choice", &drop);
}

/// The post the Bench last edited `card` to.
pub(super) fn last_edit(b: &Bench, card: &str) -> Value {
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

/// The board's text as last posted or edited.
fn board(b: &Bench) -> String {
    let id = b.s.board.as_ref().unwrap().event_id.clone();
    let posts = b.posts();
    let is_board = |e: &Value| {
        e["id"] == id.as_str() || (e["kind"] == 40003 && tag_value(e, "e").as_deref() == Some(&id))
    };
    let (e, _) = posts.iter().rev().find(|(e, _)| is_board(e)).unwrap();
    content(e).to_string()
}

/// Ticks in which the clock reads `hhmm`.
async fn ticks_at(b: &mut Bench, hhmm: &str, n: usize) {
    for _ in 0..n {
        b.now += 2;
        tick(&b.ctx, &mut b.s, b.now, hhmm, NOW).await.unwrap();
    }
}

/// One run of the old posts' own tick on `budget` posts; what it left of them.
async fn old_tick(b: &mut Bench, mut budget: usize) -> usize {
    let owners = [b.owner_hex()];
    old_posts_tick(&b.ctx, &mut b.s, &mut budget, b.now, "14:05", &owners).await;
    budget
}

/// The scoped card ASK closes out.
fn done_in_x() -> String {
    drop_json(&[
        ("project", json!("reset")),
        ("kind", json!("status")),
        ("state", json!("done")),
        ("severity", Value::Null),
        ("options", Value::Null),
        ("title", json!("Done in X")),
    ])
}

/// An old post already edited closed.
fn closed(s: &mut BenchState, id: &str, ch: &str, card: &Posted, root: Option<&str>, now: u64) {
    retired(s, id, ch, card, now);
    let r = s.replaced.last_mut().unwrap();
    (r.from_post, r.root) = (false, root.map(str::to_string));
}

/// An old post with no history, its id made from `n`, in `ch`.
fn old_card(b: &mut Bench, n: usize, ch: &str) -> Posted {
    let old = Posted {
        event_id: format!("{n:064x}"),
        ..b.card(ITEM)
    };
    closed(
        &mut b.s,
        &format!("orchestrator/old-{n}"),
        ch,
        &old,
        None,
        b.now,
    );
    old
}

/// The /queries made since `from` as the writer, the Bench's own lookups.
fn lookups(b: &Bench, from: usize) -> usize {
    let writer = json!([b.writer_hex()]);
    let queries = b.relay.queries.lock().unwrap();
    queries[from..]
        .iter()
        .filter(|q| q[0]["authors"] == writer)
        .count()
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

/// The registry beside the root, renaming the seeded card's project.
fn rename(b: &Bench, label: &str) {
    let registry = json!({"projects": [{"slug": "dreamforge", "label": label}]});
    let file = b.root.parent().unwrap().join("projects.json");
    std::fs::write(file, registry.to_string()).unwrap();
}

/// A rename never reaches a posted card. The kit rebuilds a tap's card from its
/// record (bench-card.mjs), and its edits (Recorded, old post, receipt) and its
/// recovery from the relay all show the header it was posted with.
#[tokio::test]
async fn a_cards_header_stays_as_posted_through_a_rename() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let first = b.card(ITEM);
    let head = "🔴 · Dreamforge · orchestrator\n";
    assert_eq!(first.place.as_deref(), Some("Dreamforge"));
    rename(&b, "Forge");

    let answer = tap(&b.ctx.owner, &first.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&answer]);
    b.tick().await.unwrap();
    let recorded = last_edit(&b, &first.event_id);
    assert!(content(&recorded).starts_with(head), "{recorded}");
    assert!(
        content(&recorded).contains("\nRecorded: 2. Move"),
        "{recorded}"
    );

    // Asked again: the old post keeps its header, the new card has the new name.
    redrop(&b, "Relay choice, reworded");
    b.settle(3).await;
    assert_eq!(b.s.replaced[0].card.place, first.place);
    let old = last_edit(&b, &first.event_id);
    let replaced = format!("{REPLACED}\n{head}");
    assert!(content(&old).starts_with(&replaced), "{old}");
    let second = b.card(ITEM);
    assert_ne!(second.event_id, first.event_id);
    assert_eq!(second.place.as_deref(), Some("Forge"));
    // The name the kit reads in state.json.
    assert_eq!(serde_json::to_value(&second).unwrap()["place"], "Forge");

    // Lost from state and recovered: its post's header, not today's registry.
    let post = posted(&b, &second.event_id);
    let back = "Back to you\n🔴 · Forge · orchestrator\n";
    assert!(post.content.starts_with(back), "{}", post.content);
    rename(&b, "Forge Two");
    *b.relay.query_reply.lock().unwrap() = vec![serde_json::to_value(&post).unwrap()];
    b.s.items.get_mut(ITEM).unwrap().card = None;
    b.s.needs_rebuild = true;
    b.tick().await.unwrap();
    assert_eq!(b.card(ITEM).event_id, second.event_id);
    assert_eq!(b.card(ITEM).place, second.place);

    // Closed out, the receipt (whose request has no card of its own) keeps it too.
    close_out(&b, "Moved the relay");
    b.tick().await.unwrap();
    let receipt = last_edit(&b, &second.event_id);
    let closed_head = format!("{NOT_ASKED}\n🔴 · Forge · orchestrator\n");
    assert!(content(&receipt).starts_with(&closed_head), "{receipt}");
    assert!(content(&receipt).contains("Moved the relay"), "{receipt}");
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
    let at = b.s.items[ITEM].answer.as_ref().unwrap().hhmm.clone();
    redrop(&b, "Relay choice, reworded");
    ticks_at(&mut b, "23:59", 3).await;

    let edited = last_edit(&b, &old.event_id);
    let text = content(&edited);
    assert!(
        text.starts_with(&format!("{REPLACED}\n🔴 · Dreamforge · orchestrator\n")),
        "{text}"
    );
    // What it showed stays: the question it asked and what was recorded on it,
    // at the time it was recorded, not the time it was replaced.
    assert!(text.contains("**Relay choice**"), "{text}");
    let recorded = format!("\nas of {at}\nRecorded: 2. Move · {at}\n");
    assert!(text.contains(&recorded), "{text}");
    assert!(!text.contains("23:59"), "{text}");
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
        (vec![("validity", json!("superseded"))], NOT_ASKED),
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

/// A receipt stores no time for its Outcome: the old post shows when the
/// close-out came in, not when a new question replaced it.
#[tokio::test]
async fn an_old_receipt_keeps_its_outcome_time() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM);
    let answer = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&answer]);
    b.tick().await.unwrap();
    close_out(&b, "Moved the relay");
    b.settle(2).await;
    redrop(&b, "Relay choice, reworded");
    ticks_at(&mut b, "23:59", 3).await;
    let came_in = chrono::DateTime::parse_from_rfc3339(NOW).unwrap();
    let came_in = local_hhmm(came_in.timestamp() as u64);
    let edited = last_edit(&b, &old.event_id);
    let text = content(&edited);
    assert!(text.starts_with(REPLACED), "{text}");
    assert!(text.contains(&format!("\n\nOutcome · {came_in}")), "{text}");
    assert!(!text.contains("23:59"), "{text}");
}

/// An untouched card stays as any other: a phone that lost its socket across a
/// delete would keep showing it, and the relay would refuse a tap there with
/// nothing said. It says so from its own text, its seeds come off and a tap on
/// it is told, once.
#[tokio::test]
async fn an_untouched_stale_card_is_edited_not_deleted() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM).event_id;
    let on_card = seeds(&b, &old);
    let post = posted(&b, &old);
    let mut relay: Vec<&nostr::Event> = on_card.iter().collect();
    relay.push(&post);
    reply(&b, &relay);
    redrop(&b, "Relay choice, reworded");
    b.settle(4).await;

    let edited = last_edit(&b, &old);
    let text = content(&edited);
    assert!(text.starts_with(&format!("{REPLACED}\n🔴 · ")), "{text}");
    assert!(text.contains("**Relay choice**"), "as asked: {text}");
    assert!(!text.contains("reworded"), "{text}");
    assert!(text.ends_with(&thread_line(&b.channel(), &old)), "{text}");
    let off = removed(&b);
    assert!(!off.contains(&old), "deleted: {off:?}");
    for seed in &on_card {
        assert!(off.contains(&seed.id.to_hex()), "{off:?}");
    }
    assert_eq!(b.s.kept, [old.as_str()]);

    let stray = tap(&b.ctx.owner, &old, OPTION_EMOJI[0], b.now + 1);
    reply(&b, &[&stray]);
    b.settle(3).await;
    assert_eq!(replies(&b).len(), 1);
    assert!(content(&replies(&b)[0]).contains("The live card is the one that counts:"));
}

/// lastError stops the kit's approvals: a lookup that fails, an edit refused or
/// a post gone from the relay is the board's and the retry's, never the tick's.
#[tokio::test]
async fn a_stale_card_that_will_not_update_fails_no_tick() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM).event_id;
    let post = posted(&b, &old);
    reply(&b, &[&post]);
    *b.relay.post_down.lock().unwrap() = Some(old.clone());
    redrop(&b, "Relay choice, reworded");
    b.tick().await.unwrap();
    assert!(b.s.replaced[0].from_post);
    // Saved by a build before this one, the key is absent and state still loads.
    let mut saved = serde_json::to_value(&b.s.replaced[0]).unwrap();
    saved.as_object_mut().unwrap().remove("fromPost");
    assert!(!serde_json::from_value::<Replaced>(saved).unwrap().from_post);
    let retry = b.s.replaced[0].retry_at;
    assert_eq!(retry, b.now + 60);
    b.settle(2).await;
    assert!(
        board(&b).contains("\n\nOld cards\nWould not update: 1."),
        "{}",
        board(&b)
    );
    assert!(b.s.replaced[0].from_post, "not before its retry");
    *b.relay.post_down.lock().unwrap() = None;
    b.now = retry;
    b.settle(4).await;
    assert!(content(&last_edit(&b, &old)).starts_with(REPLACED));
    assert!(!board(&b).contains("Old cards"), "{}", board(&b));

    // One gone from the relay is let go, with nothing to edit.
    let gone = b.card(ITEM).event_id;
    reply(&b, &[]);
    let before = b.posts().len();
    redrop(&b, "Relay choice, reworded again");
    b.settle(3).await;
    assert!(!b.s.replaced.iter().any(|r| r.card.event_id == gone));
    let edited = |e: &Value| e["kind"] == 40003 && tag_value(e, "e") == Some(gone.clone());
    assert!(!b.posts()[before..].iter().any(|(e, _)| edited(e)));
    assert!(b.s.kept.contains(&gone));
}

/// Nothing an old post does brings a new card back: edited, kept and read again
/// through a relaunch, the item keeps the one newer card it was given.
#[tokio::test]
async fn an_old_post_never_brings_a_repost() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM).event_id;
    let post = posted(&b, &old);
    reply(&b, &[&post]);
    redrop(&b, "Relay choice, reworded");
    b.settle(6).await;
    let new = b.card(ITEM).event_id;
    assert_ne!(new, old);
    let board_id = b.s.board.as_ref().unwrap().event_id.clone();
    let (new_post, board_post) = (posted(&b, &new), posted(&b, &board_id));
    let before = b.posts().len();
    b.settle(10).await;
    assert_eq!(b.posts().len(), before, "quiet once it is done");

    // A relaunch reads both posts: the old one is kept, so never adopted.
    reply(&b, &[&post, &new_post, &board_post]);
    b.s.needs_rebuild = true;
    b.settle(10).await;
    assert_eq!(b.card(ITEM).event_id, new);
    assert!(b.s.orphan_cards.is_empty() && b.s.strays.is_empty());
    assert_eq!(b.posts().len(), before, "no repost, no delete");

    // State lost: the newest post is the card, the old one only listed.
    b.s.kept.clear();
    b.s.needs_rebuild = true;
    b.settle(10).await;
    assert_eq!(b.card(ITEM).event_id, new);
    assert_eq!(b.s.strays, [old]);
    assert_eq!(b.posts().len(), before, "no repost, no delete");
}

/// State lost, then the same Drop again: a post edited closed says a tap
/// doesn't count, so a new card goes up. One with its Recorded edit is the card.
#[tokio::test]
async fn lost_state_never_adopts_a_post_edited_closed() {
    for closed in [true, false] {
        let mut b = seeded_bench(Gate::Open, &[]).await;
        let first = b.card(ITEM).event_id;
        let post = posted(&b, &first);
        if closed {
            reply(&b, &[&post]);
            let gone = drop_json(&[("validity", json!("superseded"))]);
            b.drop_file("orchestrator", "relay-choice", &gone);
        } else {
            let owner_tap = tap(&b.ctx.owner, &first, OPTION_EMOJI[0], b.now);
            reply(&b, &[&post, &owner_tap]);
        }
        b.settle(4).await;
        let edit = last_edit(&b, &first);
        assert_eq!(content(&edit).starts_with(NOT_ASKED), closed);
        reply(&b, &[&post, &serde_json::from_value(edit).unwrap()]);
        b.s.items.clear();
        b.s.kept.clear();
        b.s.replaced.clear();
        b.s.needs_rebuild = true;
        b.settle(1).await;
        assert!(b.s.orphan_cards.contains_key(ITEM));
        b.drop_file("orchestrator", "relay-choice", &drop_json(&[]));
        b.settle(3).await;
        assert_eq!(b.card(ITEM).event_id == first, !closed);
        assert_eq!(b.s.kept.contains(&first), closed);
    }
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
        let (mut b, old, answer) = answered_then_replaced().await;
        // Its first read is yet to come, and the live poll never saw this tap.
        b.s.replaced[0].read = false;
        let at = b.s.replaced[0].at;
        let when = if late { at } else { at - 1 };
        let tap2 = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], when);
        reply(&b, &[&answer, &tap2]);
        b.settle(3).await;
        assert_eq!(replies(&b).len(), want, "late: {late}");
        assert!(b.s.items[ITEM].answer.is_none());
        assert!(outbox(&b).is_none());
    }
}

/// A tap the live poll read in the very tick the writer asked again: the old
/// post says it is recorded and the new card comes back from it, so no reply
/// says otherwise.
#[tokio::test]
async fn a_tap_recorded_in_the_tick_of_the_re_ask_is_no_new_tap() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old = b.card(ITEM);
    let first = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&first]);
    b.tick().await.unwrap();
    redrop(&b, "Relay choice, reworded");
    let second = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[1], b.now + 2);
    reply(&b, &[&first, &second]);
    b.settle(3).await;
    assert!(replies(&b).is_empty());
    let edited = last_edit(&b, &old.event_id);
    assert!(content(&edited).contains("\nRecorded: 2. Move · "));
    assert_eq!(b.s.items[ITEM].previous.as_ref().unwrap().option, 1);
}

/// His phone's clock runs ahead and the writer asks again within that lead: the
/// answer it recorded, on the card or in a receipt, is no new tap and is never
/// told it didn't count. A keycap it hasn't seen still is one.
#[tokio::test]
async fn a_recorded_answer_from_a_clock_ahead_is_no_new_tap() {
    for (lead, receipt) in [(5, false), (60, false), (600, false), (600, true)] {
        let mut b = seeded_bench(Gate::Open, &[]).await;
        let old = b.card(ITEM);
        let answer = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[1], b.now + lead);
        reply(&b, &[&answer]);
        b.tick().await.unwrap();
        assert!(b.s.items[ITEM].answer.is_some());
        if receipt {
            close_out(&b, "Moved the relay");
            b.settle(2).await;
        }
        redrop(&b, "Relay choice, reworded");
        b.settle(3).await;
        assert!(replies(&b).is_empty(), "lead {lead}, receipt {receipt}");
        assert!(b.s.replaced[0].watch);
        let stray = tap(&b.ctx.owner, &old.event_id, OPTION_EMOJI[0], b.now);
        reply(&b, &[&answer, &stray]);
        b.settle(1).await;
        assert_eq!(replies(&b).len(), 1, "lead {lead}, receipt {receipt}");
    }
}

/// A tap after the item closed out: nothing is asked, so no live card is named.
#[tokio::test]
async fn a_tap_after_a_close_out_is_told_nothing_is_asked() {
    let (mut b, _old, answer) = answered_then_replaced().await;
    let receipt = b.card(ITEM).event_id;
    close_out(&b, "Moved the relay");
    b.settle(2).await;
    let stray = tap(&b.ctx.owner, &receipt, OPTION_EMOJI[0], b.now);
    reply(&b, &[&answer, &stray]);
    b.settle(2).await;
    let told = replies(&b);
    assert_eq!(told.len(), 1);
    let text = content(&told[0]);
    assert!(text.ends_with("This is no longer asked."), "{text}");
    assert!(!text.contains("live card"), "{text}");
}

/// Close-out receipts are the largest group of dead pills: the first line, the
/// seeds off, the answer it closed with no new tap, its card still the item's
/// (the kit's `findCard` matches `item.card`).
/// The relay refuses a reply whose root is not its thread's, so a tap on a
/// receipt posted in a thread is told under that thread's root.
#[tokio::test]
async fn a_tap_on_a_receipt_in_a_thread_is_told_under_its_root() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    bind(&mut b);
    let list = admins(&b, &b.relay.keys.clone(), &[]);
    reply(&b, &[&list]);
    let root = "ab".repeat(32);
    b.drop_file("reset-worker", "welcome", &ask(&[("replyTo", json!(root))]));
    b.settle(6).await;
    let card = b.card(ASK).event_id;
    b.drop_file("reset-worker", "welcome", &done_in_x());
    b.settle(3).await;
    assert!(b.s.items[ASK].request.is_some());
    let stray = tap(&b.ctx.owner, &card, OPTION_EMOJI[0], b.now + 5);
    reply(&b, &[&list, &stray]);
    b.settle(3).await;
    let told = replies(&b);
    assert_eq!(told.len(), 1);
    let named = |marker: &str| {
        let tags = told[0]["tags"].as_array().unwrap();
        let tag = tags.iter().find(|t| t[0] == "e" && t[3] == marker).unwrap();
        tag[1].as_str().unwrap().to_string()
    };
    assert_eq!(named("root"), root);
    assert_eq!(named("reply"), card);
}

/// Close-out receipts are the largest group of dead pills: the first line, the
/// seeds off, the answer it closed with no new tap, its card still the item's
/// (the kit's `findCard` matches `item.card`).
/// The relay refuses a reply whose root is not its thread's, so a tap on a
/// receipt posted in a thread is told under that thread's root.
#[tokio::test]
async fn a_tap_on_a_receipt_in_a_thread_is_told_under_its_root() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    bind(&mut b);
    let list = admins(&b, &b.relay.keys.clone(), &[]);
    reply(&b, &[&list]);
    let root = "ab".repeat(32);
    b.drop_file("reset-worker", "welcome", &ask(&[("replyTo", json!(root))]));
    b.settle(6).await;
    let card = b.card(ASK).event_id;
    b.drop_file("reset-worker", "welcome", &done_in_x());
    b.settle(3).await;
    assert!(b.s.items[ASK].request.is_some());
    let stray = tap(&b.ctx.owner, &card, OPTION_EMOJI[0], b.now + 5);
    reply(&b, &[&list, &stray]);
    b.settle(3).await;
    let told = replies(&b);
    assert_eq!(told.len(), 1);
    let named = |marker: &str| {
        let tags = told[0]["tags"].as_array().unwrap();
        let tag = tags.iter().find(|t| t[0] == "e" && t[3] == marker).unwrap();
        tag[1].as_str().unwrap().to_string()
    };
    assert_eq!(named("root"), root);
    assert_eq!(named("reply"), card);
}

/// A close-out in a channel that stays refused for more than a day: its edit
/// waits with the channel, unowed, and goes out once the channel is ready.
#[tokio::test]
async fn a_close_out_in_a_channel_not_ready_keeps_its_edit() {
    let (mut b, p, list) = bound_bench().await;
    let card = b.card(ASK).event_id;
    let bad = admins(&b, &b.relay.keys.clone(), &[&hex(&p.craig)]);
    reply(&b, &[&bad]);
    b.now += 601;
    b.settle(2).await;
    assert!(b.s.channels[X].refused.is_some());
    b.drop_file("reset-worker", "welcome", &done_in_x());
    let before = b.posts().len();
    b.settle(2).await;
    assert!(b.s.items[ASK].request.is_some());
    b.now += 86_400;
    b.settle(2).await;
    let edited = |b: &Bench| {
        let on_card = |e: &Value| e["kind"] == 40003 && tag_value(e, "e").as_deref() == Some(&card);
        b.posts()[before..].iter().any(|(e, _)| on_card(e))
    };
    assert!(!edited(&b), "not while the channel is refused");
    reply(&b, &[&list]);
    b.now += 61;
    b.settle(4).await;
    assert_eq!(b.s.channels[X].refused, None);
    assert!(edited(&b), "the close-out's edit never went out");
    let receipt = last_edit(&b, &card);
    assert!(content(&receipt).starts_with(NOT_ASKED), "{receipt}");
    assert!(content(&receipt).contains("Done in X"), "{receipt}");
}

/// A later close-out's edit, failing once near the end of the first one's
/// 24 h, still gets its retry.
#[tokio::test]
async fn a_later_close_out_gets_its_own_window() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    close_out(&b, "First report");
    b.settle(3).await;
    assert!(b.s.replaced[0].shown.is_none() && b.s.replaced[0].watch);
    b.now = b.s.replaced[0].at + 86_400 - 6;
    *b.relay.post_down.lock().unwrap() = Some(card.clone());
    close_out(&b, "Second report");
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert!(b.s.replaced[0].retry_at > 0, "one failed try");
    *b.relay.post_down.lock().unwrap() = None;
    b.now += 60;
    b.settle(4).await;
    assert_eq!(b.s.last_error, None);
    // The fake relay logs a refused write too, so two is the retry.
    let second = b.posts()[before..]
        .iter()
        .filter(|(e, _)| e["kind"] == 40003 && tag_value(e, "e").as_deref() == Some(&card))
        .filter(|(e, _)| content(e).contains("Second report"))
        .count();
    assert_eq!(second, 2);
}

/// A new card format is no close-out: receipts closed days ago are edited
/// again, unwatched, and leave the cap to posts a tap could still come to.
#[tokio::test]
async fn a_format_bump_watches_no_old_receipt() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    close_out(&b, "Moved the relay");
    b.settle(3).await;
    let receipt = b.s.items[ITEM].clone();
    b.s.replaced.clear();
    b.s.receipts_todo = Some(Vec::new());
    for n in 0..=WATCH_MAX {
        let mut i = receipt.clone();
        i.id = format!("orchestrator/r{n}");
        let c = i.card.as_mut().unwrap();
        c.event_id = format!("{n:064x}");
        c.hash = format!("{}.3", i.hash);
        b.s.items.insert(i.id.clone(), i);
    }
    b.tick().await.unwrap();
    assert_eq!(b.s.replaced.len(), WATCH_MAX + 1);
    assert!(b.s.replaced.iter().all(|r| !r.watch));
    b.tick().await.unwrap();
    assert!(!board(&b).contains("Watched for a tap"), "{}", board(&b));
}

#[tokio::test]
async fn a_receipt_says_so_and_loses_its_seeds() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM);
    let on_card = seeds(&b, &card.event_id);
    assert_eq!(on_card.len(), 2);
    // Its clock is ahead, so only `seen` keeps it from counting as new.
    let answer = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now + 500);
    let mut relay: Vec<&nostr::Event> = on_card.iter().collect();
    relay.push(&answer);
    reply(&b, &relay);
    b.tick().await.unwrap();
    close_out(&b, "Moved the relay");
    b.settle(4).await;

    let edited = last_edit(&b, &card.event_id);
    let text = content(&edited);
    assert!(text.starts_with(&format!("{NOT_ASKED}\n🔴 · ")), "{text}");
    assert!(text.contains("\nAnswered: 2. Move"), "{text}");
    assert!(text.contains("**Moved the relay**"), "{text}");
    let off = removed(&b);
    for seed in &on_card {
        assert!(off.contains(&seed.id.to_hex()), "{off:?}");
    }
    assert!(!off.contains(&answer.id.to_hex()) && !off.contains(&card.event_id));
    assert!(replies(&b).is_empty(), "the answer it closed with");
    assert!(b.s.kept.is_empty());
    assert_eq!(b.card(ITEM).event_id, card.event_id);
}

/// Its edit failing used to fail the tick, and lastError stops the kit.
#[tokio::test]
async fn a_receipt_that_will_not_update_fails_no_tick_and_one_gone_is_forgotten() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    close_out(&b, "Moved the relay");
    *b.relay.post_down.lock().unwrap() = Some(card.clone());
    b.tick().await.unwrap();
    assert!(b.s.replaced[0].shown.is_some() && b.s.replaced[0].retry_at > 0);
    // A second close-out while it is owed: one post, one edit, the last words.
    close_out(&b, "Moved after all");
    b.tick().await.unwrap();
    assert_eq!(b.s.replaced.len(), 1);
    *b.relay.post_down.lock().unwrap() = None;
    b.now = b.s.replaced[0].retry_at;
    b.settle(2).await;
    let text = last_edit(&b, &card);
    assert!(content(&text).contains("**Moved after all**"), "{text}");

    // A later close-out on a receipt deleted by hand: the item lets it go.
    b.relay.gone.lock().unwrap().insert(card);
    close_out(&b, "Moved again");
    b.settle(3).await;
    assert!(b.s.items[ITEM].card.is_none());
}

/// Receipts from before this build: one at a time and unwatched, each with its
/// first line and without its seeds; then never again.
#[tokio::test]
async fn old_receipts_are_edited_one_at_a_time_then_never_again() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    b.drop_file(
        "orchestrator",
        "other",
        &drop_json(&[("title", json!("Other"))]),
    );
    b.settle(4).await;
    let (a, z) = (b.card(ITEM).event_id, b.card("orchestrator/other").event_id);
    let on_card = [seeds(&b, &a), seeds(&b, &z)].concat();
    let done = drop_json(&[
        ("kind", json!("status")),
        ("state", json!("done")),
        ("severity", Value::Null),
        ("options", Value::Null),
    ]);
    b.drop_file("orchestrator", "relay-choice", &done);
    b.drop_file("orchestrator", "other", &done);
    b.settle(6).await;
    // As a build before this left them: nothing owed, not listed.
    b.s.replaced.clear();
    b.s.receipts_todo = None;
    reply(&b, &on_card.iter().collect::<Vec<_>>());
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert_eq!(
        b.posts().len(),
        before,
        "listed this tick, edited from the next"
    );
    assert_eq!(b.s.receipts_todo.as_ref().unwrap().len(), 1);
    assert!(b.s.replaced.len() == 1 && !b.s.replaced[0].watch);
    b.settle(10).await;
    assert_eq!(b.s.receipts_todo.as_deref(), Some(&[][..]));
    assert!(b.s.replaced.is_empty(), "let go once done");
    for card in [&a, &z] {
        let text = last_edit(&b, card);
        assert!(content(&text).starts_with(NOT_ASKED), "{text}");
    }
    let off = removed(&b);
    for seed in &on_card {
        let n = off.iter().filter(|id| **id == seed.id.to_hex()).count();
        assert_eq!(n, 1, "{off:?}");
    }
    let before = b.posts().len();
    b.settle(5).await;
    assert_eq!(b.posts().len(), before, "never again");
}

/// One that can't be told yet stays listed; one asked again since has a live card,
/// which is never edited as an old post.
#[tokio::test]
async fn an_old_receipt_waits_for_its_channel_and_one_asked_again_is_skipped() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    close_out(&b, "Moved the relay");
    b.settle(3).await;
    b.s.replaced.clear();
    b.s.receipts_todo = Some(vec![ITEM.to_string()]);
    let in_x = |b: &mut Bench, ch: Option<String>| {
        b.s.items
            .get_mut(ITEM)
            .unwrap()
            .request
            .as_mut()
            .unwrap()
            .channel = ch;
    };
    in_x(&mut b, Some(X.to_string()));
    b.settle(2).await;
    assert!(b.s.replaced.is_empty());
    assert_eq!(b.s.receipts_todo.as_ref().unwrap().len(), 1);
    in_x(&mut b, None);
    redrop(&b, "Relay choice, asked again");
    b.settle(6).await;
    assert_eq!(b.s.receipts_todo.as_deref(), Some(&[][..]));
    let live = b.card(ITEM).event_id;
    let on_live = |e: &Value| e["kind"] == 40003 && tag_value(e, "e").as_ref() == Some(&live);
    assert!(!b.posts().iter().any(|(e, _)| on_live(e)));
}

/// A relay refusing them holds one on the board, not all of them in state.
#[tokio::test]
async fn an_old_receipt_that_will_not_update_holds_the_rest() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    close_out(&b, "Moved the relay");
    b.settle(3).await;
    b.s.replaced.clear();
    b.s.receipts_todo = Some(vec![ITEM.to_string(), ITEM.to_string()]);
    *b.relay.post_down.lock().unwrap() = Some(card);
    b.settle(4).await;
    assert_eq!(b.s.replaced.len(), 1);
    assert_eq!(b.s.receipts_todo.as_ref().unwrap().len(), 1);
    assert!(board(&b).contains("Would not update: 1."), "{}", board(&b));
}

/// A channel that isn't ready: nothing there is read, edited or counted against
/// the cap, and it all resumes when the channel is ready.
#[tokio::test]
async fn an_old_post_in_a_channel_not_ready_waits() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let olds: Vec<String> = (0..=WATCH_MAX)
        .map(|n| old_card(&mut b, n, X).event_id)
        .collect();
    let read = |b: &Bench| {
        let queries = b.relay.queries.lock().unwrap();
        queries.iter().any(|q| olds.iter().any(|id| names(q, id)))
    };
    b.settle(2).await;
    assert!(!read(&b));
    let edited = |e: &Value| tag_value(e, "e").is_some_and(|id| olds.contains(&id));
    assert!(!b.posts().iter().any(|(e, _)| edited(e)));
    assert!(!board(&b).contains("Old cards"), "{}", board(&b));

    let writer = b.writer_hex();
    b.s.joined.insert(X.to_string(), vec![writer]);
    let check = ChannelCheck {
        checked_at: b.now,
        ok_at: b.now,
        refused: None,
    };
    b.s.channels.insert(X.to_string(), check);
    b.settle(2).await;
    assert!(read(&b));
    let watched = format!("Watched for a tap: {}.", WATCH_MAX + 1);
    assert!(board(&b).contains(&watched), "{}", board(&b));
}

/// A post that would not update, then does: a refused removal isn't done, and
/// once one goes through the board stops saying so though it's still watched.
#[tokio::test]
async fn a_watched_post_that_recovers_clears_the_board() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let ch = b.channel();
    let old = old_card(&mut b, 1, &ch);
    let seed = tap(&b.ctx.writer, &old.event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&seed]);
    *b.relay.clock.lock().unwrap() = Some(b.now + 10_000);
    b.tick().await.unwrap();
    let retry = b.s.replaced[0].retry_at;
    assert_eq!(retry, b.now + 60);
    assert!(!b.s.replaced[0].seeds_off);
    *b.relay.clock.lock().unwrap() = None;
    b.tick().await.unwrap();
    assert!(board(&b).contains("Would not update: 1."), "{}", board(&b));
    b.now = retry;
    b.settle(2).await;
    let r = &b.s.replaced[0];
    assert!(r.seeds_off && r.watch);
    assert_eq!(r.retry_at, 0);
    assert!(!board(&b).contains("Old cards"), "{}", board(&b));
}

/// A seed already gone is as good as removed: its post stays watched.
#[tokio::test]
async fn a_seed_gone_from_the_relay_counts_as_removed() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let ch = b.channel();
    let old = old_card(&mut b, 1, &ch);
    let seed = tap(&b.ctx.writer, &old.event_id, OPTION_EMOJI[0], b.now);
    b.relay.gone.lock().unwrap().insert(seed.id.to_hex());
    reply(&b, &[&seed]);
    b.tick().await.unwrap();
    let r = &b.s.replaced[0];
    assert!(r.seeds_off && r.watch);
    assert_eq!(r.retry_at, 0);
}

/// Each removal and each reply is charged to the post budget.
#[tokio::test]
async fn removals_and_replies_stop_at_the_post_budget() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let ch = b.channel();
    let old = old_card(&mut b, 1, &ch);
    let seeds = [0, 1].map(|n| tap(&b.ctx.writer, &old.event_id, OPTION_EMOJI[n], b.now));
    reply(&b, &[&seeds[0], &seeds[1]]);
    assert_eq!(old_tick(&mut b, 1).await, 0);
    assert_eq!(removed(&b).len(), 1);
    assert!(!b.s.replaced[0].seeds_off);
    assert_eq!(old_tick(&mut b, 2).await, 0);
    assert!(b.s.replaced[0].seeds_off);

    let mut b = seeded_bench(Gate::Open, &[]).await;
    let ch = b.channel();
    for n in 1..=2 {
        old_card(&mut b, n, &ch);
    }
    let owner = b.owner_hex();
    for r in &mut b.s.replaced {
        r.seeds_off = true;
        r.tapped = Some(owner.clone());
    }
    assert_eq!(old_tick(&mut b, 1).await, 0);
    assert_eq!(replies(&b).len(), 1);
    assert!(b.s.replaced[1].tapped.is_some());
}

/// One lookup a tick and a minute's retry: with more posts failing than a
/// minute's lookups, every tick runs out. They still go at the end of their
/// window.
#[tokio::test]
async fn posts_failing_past_the_lookups_still_go_at_their_window() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let ch = b.channel();
    let seeds: Vec<_> = (1..=40)
        .map(|n| {
            let old = old_card(&mut b, n, &ch);
            tap(&b.ctx.writer, &old.event_id, OPTION_EMOJI[0], b.now)
        })
        .collect();
    reply(&b, &seeds.iter().collect::<Vec<_>>());
    let end = b.now + 86_400;
    *b.relay.clock.lock().unwrap() = Some(b.now + 10_000);
    for _ in 0..120 {
        b.now += 2;
        old_tick(&mut b, 4).await;
    }
    assert_eq!(b.s.replaced.len(), 40);
    b.now = end;
    // The 30 tried go at once; the 10 waiting behind them go a tick each.
    for _ in 0..10 {
        b.now += 2;
        old_tick(&mut b, 4).await;
    }
    assert!(b.s.replaced.is_empty());
}

/// Out of lookups, a post past its window that hasn't failed waits for its try:
/// a Bench off for a day still closes what it owed.
#[tokio::test]
async fn past_its_window_a_post_still_gets_its_try_when_lookups_run_out() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let ch = b.channel();
    let seeds = [1, 2].map(|n| {
        let old = old_card(&mut b, n, &ch);
        tap(&b.ctx.writer, &old.event_id, OPTION_EMOJI[0], b.now)
    });
    reply(&b, &[&seeds[0], &seeds[1]]);
    b.now += 86_400;
    old_tick(&mut b, 4).await;
    assert_eq!(removed(&b).len(), 1);
    assert_eq!(b.s.replaced.len(), 2);
    old_tick(&mut b, 4).await;
    assert_eq!(removed(&b).len(), 2);
    assert!(b.s.replaced.is_empty());
}

/// With no post left to spend, the lookups before a removal or a reply wait too.
#[tokio::test]
async fn no_budget_no_lookup() {
    for seeds_off in [false, true] {
        let mut b = seeded_bench(Gate::Open, &[]).await;
        let ch = b.channel();
        old_card(&mut b, 1, &ch);
        b.s.replaced[0].seeds_off = seeds_off;
        b.s.replaced[0].tapped = Some(b.owner_hex());
        let from = b.relay.queries.lock().unwrap().len();
        old_tick(&mut b, 0).await;
        assert_eq!(lookups(&b, from), 0, "seeds off: {seeds_off}");
    }
}

/// Old posts with no seeds left spend no post, so their lookups have their own
/// cap: a run of them drains over a few ticks, not one query each in one tick.
#[tokio::test]
async fn lookups_are_capped_per_tick() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let ch = b.channel();
    for n in 1..=3 {
        old_card(&mut b, n, &ch);
    }
    for want in 1..=3 {
        let from = b.relay.queries.lock().unwrap().len();
        assert_eq!(old_tick(&mut b, 4).await, 4);
        assert_eq!(lookups(&b, from), 1);
        let done = b.s.replaced.iter().filter(|r| r.seeds_off).count();
        assert_eq!(done, want);
    }
    // The lookup before each reply counts the same.
    let owner = b.owner_hex();
    for r in &mut b.s.replaced {
        r.tapped = Some(owner.clone());
    }
    let from = b.relay.queries.lock().unwrap().len();
    old_tick(&mut b, 4).await;
    assert_eq!(lookups(&b, from), 1);
}

/// Its lookup is a lookup and its edit a post, like any other.
#[tokio::test]
async fn an_edit_from_its_post_spends_a_post() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let (ch, card) = (b.channel(), b.card(ITEM));
    let post = posted(&b, &card.event_id);
    reply(&b, &[&post]);
    retired(&mut b.s, "orchestrator/old", &ch, &card, b.now);
    assert_eq!(old_tick(&mut b, 4).await, 3);
}

/// A read that keeps failing fails no tick (lastError stops the kit's approvals).
/// Past a minute the board says taps go unanswered, until a read goes through.
#[tokio::test]
async fn a_read_that_keeps_failing_is_on_the_board_until_one_goes_through() {
    const SAYS: &str = "Can't read them for taps right now";
    let (mut b, old, answer) = answered_then_replaced().await;
    reply(&b, &[&answer]);
    b.settle(3).await;
    assert!(b.s.replaced[0].seeds_off);
    *b.relay.e_down.lock().unwrap() = Some(old.event_id.clone());
    b.settle(2).await;
    assert!(!board(&b).contains(SAYS), "not after a failure or two");
    b.now += 60;
    b.settle(2).await;
    assert!(board(&b).contains(SAYS), "{}", board(&b));
    *b.relay.e_down.lock().unwrap() = None;
    b.settle(2).await;
    assert!(!board(&b).contains(SAYS), "{}", board(&b));

    // Nothing left to read says the same, though the read would still fail.
    *b.relay.e_down.lock().unwrap() = Some(old.event_id.clone());
    b.settle(1).await;
    b.now += 60;
    b.settle(1).await;
    assert!(board(&b).contains(SAYS), "{}", board(&b));
    b.s.replaced.clear();
    b.settle(2).await;
    assert!(!board(&b).contains(SAYS), "{}", board(&b));
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
    // Lookups have a cap of their own (`lookups_are_capped_per_tick`).
    for r in &mut b.s.replaced {
        r.seeds_off = true;
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
    old_tick(&mut b, 0).await;
    assert!(replies(&b).is_empty(), "no budget, so it waits");
    old_tick(&mut b, 1).await;
    let told = replies(&b);
    assert_eq!(told.len(), 1);
    assert!(
        content(&told[0]).ends_with("The newer card isn't up yet; the board shows it when it is."),
        "{}",
        content(&told[0])
    );
}
