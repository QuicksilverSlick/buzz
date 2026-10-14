//! Bench M2: answers by reaction. Child of tests.rs so the fake relay, Bench
//! harness and drop helpers are in scope.

use super::*;

pub(super) fn tap(keys: &Keys, card_hex: &str, emoji: &str, created_at: u64) -> nostr::Event {
    crate::events::build_reaction(nostr::EventId::from_hex(card_hex).unwrap(), emoji)
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn answered(option: usize, answered_at: u64) -> Option<Answer> {
    Some(Answer {
        reaction_id: "ab".repeat(32),
        option,
        answered_at,
        ..Default::default()
    })
}

pub(super) fn reply(b: &Bench, evs: &[&nostr::Event]) {
    *b.relay.query_reply.lock().unwrap() = evs
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
}

pub(super) fn outbox(b: &Bench) -> Option<Value> {
    std::fs::read_to_string(b.root.join("outbox/orchestrator/relay-choice.json"))
        .ok()
        .map(|t| serde_json::from_str(&t).unwrap())
}

pub(super) fn last_query(b: &Bench) -> Value {
    b.relay.queries.lock().unwrap().last().cloned().unwrap()
}

fn text(ev: &Value) -> &str {
    ev["content"].as_str().unwrap()
}

fn board_id(b: &Bench) -> String {
    b.s.board.as_ref().unwrap().event_id.clone()
}

pub(super) const ITEM: &str = "orchestrator/relay-choice";

#[test]
fn pick_answer_rules() {
    let owner = Keys::generate();
    let member = Keys::generate();
    let writer = Keys::generate();
    let card_hex = "ab".repeat(32);
    let mut item = check(&[]).unwrap();
    let no_card = [tap(&owner, &card_hex, OPTION_EMOJI[1], 10)];
    let owner_hex = [owner.public_key().to_hex()];
    assert!(pick_answer(&item, &no_card, &owner_hex, "").is_none());
    item.card = Some(Posted {
        event_id: card_hex.clone(),
        created_at: 1,
        hash: item.hash.clone(),
        seeded: 2,
    });
    // The writer is listed as an approver on purpose: its own seeds must
    // still never count.
    let approvers: Vec<String> = [&owner, &member, &writer]
        .iter()
        .map(|k| k.public_key().to_hex())
        .collect();
    let writer_hex = writer.public_key().to_hex();
    let pick = |evs: &[nostr::Event]| {
        pick_answer(&item, evs, &approvers, &writer_hex).map(|(i, e)| (i, e.id.to_hex()))
    };

    // A broken signature: the content changed after signing.
    let mut forged = serde_json::to_value(tap(&owner, &card_hex, OPTION_EMOJI[0], 10)).unwrap();
    forged["content"] = json!(OPTION_EMOJI[1]);
    let forged: nostr::Event = serde_json::from_value(forged).unwrap();
    let kind9 = nostr::EventBuilder::new(nostr::Kind::Custom(9), OPTION_EMOJI[1])
        .tags([nostr::Tag::event(
            nostr::EventId::from_hex(&card_hex).unwrap(),
        )])
        .sign_with_keys(&owner)
        .unwrap();
    let ignored = [
        tap(&Keys::generate(), &card_hex, OPTION_EMOJI[1], 10),
        tap(&writer, &card_hex, OPTION_EMOJI[0], 10),
        tap(&owner, &"cd".repeat(32), OPTION_EMOJI[1], 10),
        forged,
        tap(&owner, &card_hex, "\u{2764}\u{FE0F}", 10),
        tap(&owner, &card_hex, OPTION_EMOJI[2], 10),
        kind9,
    ];
    for ev in &ignored {
        assert!(pick(std::slice::from_ref(ev)).is_none(), "{ev:?}");
    }

    let owner_tap = tap(&owner, &card_hex, OPTION_EMOJI[1], 10);
    let member_tap = tap(&member, &card_hex, OPTION_EMOJI[0], 20);
    assert_eq!(
        pick(std::slice::from_ref(&owner_tap)),
        Some((1, owner_tap.id.to_hex()))
    );
    assert_eq!(
        pick(std::slice::from_ref(&member_tap)),
        Some((0, member_tap.id.to_hex()))
    );
    // Latest wins, and the noise around it changes nothing.
    let mut all = ignored.to_vec();
    all.extend([member_tap.clone(), owner_tap.clone()]);
    assert_eq!(pick(&all), Some((0, member_tap.id.to_hex())));
    // A tie on created_at is broken by the larger id, so the pick is stable.
    let same = tap(&member, &card_hex, OPTION_EMOJI[0], 10);
    let expect = if same.id.to_hex() > owner_tap.id.to_hex() {
        (0, same.id.to_hex())
    } else {
        (1, owner_tap.id.to_hex())
    };
    assert_eq!(
        pick(&[owner_tap.clone(), same.clone()]),
        Some(expect.clone())
    );
    assert_eq!(pick(&[same, owner_tap]), Some(expect));
}

#[test]
fn render_card_recorded_trailer() {
    let mut item =
        check(&[("options", json!(["Stay", "Move [x](https://evil.example)"]))]).unwrap();
    let plain = render_card(&item, "14:05");
    item.answer = answered(1, 1);
    let card = render_card(&item, "14:05");
    // The Recorded trailer replaces the how-to-answer hint.
    assert_eq!(
        card,
        format!(
            "{}\nRecorded: 2. Move x (https: evil.example) · 14:05",
            plain.replace(&format!("\n{HINT}"), "")
        )
    );
    assert!(
        !card.contains("https://") && !card.contains('['),
        "{card:?}"
    );
    item.answer = None;
    assert_eq!(render_card(&item, "14:05"), plain);
}

#[test]
fn render_board_answered_section() {
    let open = check(&[("id", json!("open"))]).unwrap();
    let mut done = check(&[
        ("id", json!("done")),
        ("options", json!(["Stay", "Move [x](https://evil.example)"])),
    ])
    .unwrap();
    let mut first = check(&[("id", json!("zz")), ("title", json!("Earlier tap"))]).unwrap();
    assert_eq!(
        render_board(&[&open, &done, &first], UUID, "14:05", 0),
        format!(
            "Bench · as of 14:05 · needs you: 3\n\
             {LEGEND}\n\
             \n\
             Needs you\n\
             - 🔴 Relay choice (dreamforge) (no card yet)\n\
             - 🔴 Relay choice (dreamforge) (no card yet)\n\
             - 🔴 Earlier tap (dreamforge) (no card yet)"
        )
    );
    done.answer = answered(1, 100);
    first.answer = answered(0, 50);
    // A kept card is linked, so an undo is one tap away.
    let hex64 = "ab".repeat(32);
    first.card = Some(Posted {
        event_id: hex64.clone(),
        created_at: 1,
        hash: card_hash(&first),
        seeded: 1,
    });
    let all = [&open, &done, &first];
    let board = render_board(&all, UUID, "14:05", 100);
    assert_eq!(
        board,
        format!(
            "Bench · as of 14:05 · needs you: 1 · answered: 2\n\
             {LEGEND}\n\
             \n\
             Needs you\n\
             - 🔴 Relay choice (dreamforge) (no card yet)\n\
             \n\
             Answered, waiting for action (2)\n\
             - 🔴 Earlier tap (dreamforge) → 1. Stay · buzz://message?channel={UUID}&id={hex64}\n\
             - 🔴 Relay choice (dreamforge) → 2. Move x (https: evil.example)"
        )
    );
    assert!(!board.contains("https://") && !board.contains('['));
    // 24 h exactly (for the earliest answer) is not late; one second past
    // the latest one is, and it changes the hash.
    assert_eq!(
        board_hash(&board),
        board_hash(&render_board(&all, UUID, "23:59", 50 + ANSWER_KEEP_SECS))
    );
    let late = render_board(&all, UUID, "14:05", 100 + ANSWER_KEEP_SECS + 1);
    assert!(
        late.ends_with("→ 2. Move x (https: evil.example) 🟠 24h"),
        "{late:?}"
    );
    assert_ne!(board_hash(&board), board_hash(&late));
}

#[test]
fn m1_state_loads() {
    let mut item = check(&[]).unwrap();
    let mut v = serde_json::to_value(&item).unwrap();
    let o = v.as_object_mut().unwrap();
    for field in ["answer", "recorded", "quietRepost", "routed", "changedAt"] {
        assert!(o.remove(field).is_some(), "{field}");
    }
    let loaded: Item = serde_json::from_value(v).unwrap();
    assert!(loaded.answer.is_none() && loaded.recorded.is_none() && !loaded.quiet_repost);
    // An item answered before routing existed is due at once (changedAt 0).
    assert!(loaded.routed.is_none() && loaded.changed_at == 0);
    // And the M2 fields round-trip in camelCase.
    item.answer = answered(1, 7);
    item.recorded = Some(1);
    let v = serde_json::to_value(&item).unwrap();
    assert_eq!(v["answer"]["reactionId"], json!("ab".repeat(32)));
    assert_eq!(v["answer"]["answeredAt"], json!(7));
    assert_eq!(v["recorded"], json!(1));
    let back: Item = serde_json::from_value(v).unwrap();
    assert!(back.answer == item.answer && back.recorded == Some(1));
}

#[tokio::test]
async fn owner_tap_records_answer_and_writes_outbox() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM);
    let owner_tap = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now - 5);
    reply(&b, &[&owner_tap]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert_eq!(
        b.s.items[ITEM].answer,
        Some(Answer {
            reaction_id: owner_tap.id.to_hex(),
            option: 1,
            answered_at: b.now,
            by: b.owner_hex(),
            hhmm: local_hhmm(b.now),
            ..Default::default()
        })
    );
    assert_eq!(b.s.items[ITEM].recorded, Some(1));
    let out = outbox(&b).unwrap();
    assert_eq!(out["item"], json!(ITEM));
    assert_eq!(out["writer"], json!("orchestrator"));
    assert_eq!(out["card"], json!(card.event_id));
    assert_eq!(out["option"], json!(2));
    assert_eq!(out["text"], json!("Move"));
    let reaction: nostr::Event = serde_json::from_value(out["reaction"].clone()).unwrap();
    assert!(reaction.verify().is_ok() && reaction.id == owner_tap.id);
    // Who answered: the owner's own tap.
    assert_eq!(out["answerer"], json!(b.owner_hex()));
    assert_eq!(out["answererRole"], json!("owner"));
    assert_eq!(out["eligible"], json!([]));
    assert_eq!(out["source"], json!("tap"));
    assert_eq!(
        last_query(&b),
        json!([{
            "kinds": [7],
            "authors": [b.owner_hex(), "cd".repeat(32)],
            "#h": [b.channel()],
            "#e": [card.event_id],
            "limit": 500,
        }])
    );
    // The card's Recorded edit lands first, then the board's Answered section.
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [40003, 40003]);
    assert_eq!(tag_value(&posts[0].0, "e").unwrap(), card.event_id);
    assert!(text(&posts[0].0).ends_with("Recorded: 2. Move · 14:05"));
    assert_eq!(tag_value(&posts[1].0, "e").unwrap(), board_id(&b));
    let board = text(&posts[1].0);
    assert!(
        board.contains("Answered, waiting for action (1)")
            && board.contains("needs you: 0 · answered: 1"),
        "{board:?}"
    );
    let queries = b.relay.queries.lock().unwrap().len();
    b.tick().await.unwrap();
    assert_eq!(b.posts().len(), before + 2);
    assert_eq!(b.relay.queries.lock().unwrap().len(), queries + 1);
}

#[tokio::test]
async fn retraction_unrecords_in_place_and_latest_wins() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM);
    let opt1 = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now - 10);
    let opt0 = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[0], b.now - 5);
    let before = b.posts().len();
    reply(&b, &[&opt1, &opt0]);
    b.tick().await.unwrap();
    assert_eq!(b.s.items[ITEM].answer.as_ref().map(|a| a.option), Some(0));
    assert_eq!(outbox(&b).unwrap()["option"], json!(1));
    // Untapping the newest hands the answer to the one that stays.
    reply(&b, &[&opt1]);
    let n = b.posts().len();
    b.tick().await.unwrap();
    assert_eq!(b.s.items[ITEM].answer.as_ref().map(|a| a.option), Some(1));
    let out = outbox(&b).unwrap();
    assert_eq!(out["option"], json!(2));
    assert_eq!(out["reaction"]["id"], json!(opt1.id.to_hex()));
    let posts = b.posts()[n..].to_vec();
    assert_eq!(b.kinds()[n..], [40003, 40003]);
    assert!(text(&posts[0].0).contains("Recorded: 2. Move"));
    assert_eq!(tag_value(&posts[1].0, "e").unwrap(), board_id(&b));
    // Untapping the last one un-records in place.
    reply(&b, &[]);
    let n = b.posts().len();
    b.tick().await.unwrap();
    let item = &b.s.items[ITEM];
    assert!(item.answer.is_none() && item.recorded.is_none());
    assert!(outbox(&b).is_none());
    let posts = b.posts()[n..].to_vec();
    assert_eq!(b.kinds()[n..], [40003, 40003]);
    assert!(!text(&posts[0].0).contains("Recorded"));
    assert_eq!(tag_value(&posts[1].0, "e").unwrap(), board_id(&b));
    assert_eq!(b.card(ITEM).event_id, card.event_id);
    assert!(!b.kinds()[before..].iter().any(|k| matches!(k, 5 | 9)));
}

#[tokio::test]
async fn answered_card_is_never_retired() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM);
    let owner_tap = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    // A same-hash re-drop is a no-op: the answered card stays put.
    let before = b.posts().len();
    b.drop_file("orchestrator", "relay-choice", &drop_json(&[]));
    b.settle(2).await;
    assert!(!b.kinds()[before..].iter().any(|k| matches!(k, 5 | 9)));
    let old_board = board_id(&b);
    b.now += 86_401;
    let before = b.posts().len();
    b.settle(3).await;
    // Only the day-old board is reposted, flagging the wait; the card stays.
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [9, 5]);
    assert_eq!(tag_value(&posts[1].0, "e").unwrap(), old_board);
    assert!(
        text(&posts[0].0).contains("🟠 24h"),
        "{:?}",
        text(&posts[0].0)
    );
    let item = &b.s.items[ITEM];
    assert!(item.recorded == Some(1) && item.answer.is_some());
    assert_eq!(b.card(ITEM).event_id, card.event_id);
    assert!(outbox(&b).is_some());
}

#[tokio::test]
async fn recorded_edit_not_found_drops_card_keeps_answer() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM);
    b.relay.gone.lock().unwrap().insert(card.event_id.clone());
    let owner_tap = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    // The refused edit, then the board repost pair.
    assert_eq!(b.kinds()[before..], [40003, 9, 5]);
    let item = &b.s.items[ITEM];
    assert!(item.card.is_none() && item.answer.is_some() && item.recorded.is_none());
    let before = b.posts().len();
    b.settle(3).await;
    assert_eq!(b.posts().len(), before);
    let posts = b.posts();
    let board = posts.iter().rev().find(|(e, _)| e["kind"] == 9).unwrap();
    assert!(text(&board.0).contains("Answered, waiting for action (1)"));
}

#[tokio::test]
async fn ack_by_redrop_clears_answer_and_keeps_the_old_card() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM);
    let owner_tap = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    assert!(outbox(&b).is_some());
    let before = b.posts().len();
    b.drop_file(
        "orchestrator",
        "relay-choice",
        &drop_json(&[("title", json!("Relay choice, reworded"))]),
    );
    let old_board = board_id(&b);
    b.settle(3).await;
    // A new question after an answer: the answered card stays on the relay as
    // it is, a new one is posted and seeded, the board follows; the tap on
    // the old card counts no more.
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [9, 7, 7, 9, 5]);
    assert_eq!(tag_value(&posts[4].0, "e").unwrap(), old_board);
    let item = &b.s.items[ITEM];
    assert!(item.answer.is_none() && item.recorded.is_none());
    assert!(outbox(&b).is_none());
    assert_ne!(b.card(ITEM).event_id, card.event_id);
    // A relaunch never adopts the old card back, so it is never retired.
    assert_eq!(b.s.kept, [card.event_id]);
}

#[tokio::test]
async fn ignored_reactions_never_answer() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    let (owner, writer) = (b.ctx.owner.clone(), b.ctx.writer.clone());
    let noise = [
        tap(&Keys::generate(), &card, OPTION_EMOJI[1], b.now),
        tap(&writer, &card, OPTION_EMOJI[0], b.now),
        tap(&owner, &card, "\u{2764}\u{FE0F}", b.now),
        tap(&owner, &"ab".repeat(32), OPTION_EMOJI[0], b.now),
        tap(&owner, &card, OPTION_EMOJI[2], b.now),
    ];
    reply(&b, &noise.iter().collect::<Vec<_>>());
    let before = b.posts().len();
    b.tick().await.unwrap();
    let item = &b.s.items[ITEM];
    assert!(item.answer.is_none() && item.recorded.is_none());
    assert!(outbox(&b).is_none());
    assert_eq!(b.posts().len(), before);
}

#[tokio::test]
async fn idle_bench_sends_no_query() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    b.settle(3).await;
    let queries = b.relay.queries.lock().unwrap().clone();
    assert!(!queries.is_empty());
    assert!(
        queries.iter().all(|q| q[0]["kinds"] == json!([9])),
        "{queries:?}"
    );
}

#[tokio::test]
async fn stale_card_never_supplies_an_answer() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    // A second live card, so the poll still runs once the first goes stale.
    b.drop_file(
        "orchestrator",
        "other",
        &drop_json(&[("title", json!("Other"))]),
    );
    b.settle(3).await;
    let other = b.card("orchestrator/other").event_id;
    let card = b.card(ITEM);
    let owner_tap = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    assert!(b.s.items[ITEM].answer.is_some());
    // The ack re-drop landed (intake cleared the answer and the outbox) but
    // the budget ran out before (b) could retire the old card.
    let item = b.s.items.get_mut(ITEM).unwrap();
    item.hash = "ffff".to_string();
    item.answer = None;
    std::fs::remove_file(b.root.join("outbox/orchestrator/relay-choice.json")).unwrap();
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert!(b.s.items[ITEM].answer.is_none());
    assert!(outbox(&b).is_none());
    assert_eq!(last_query(&b)[0]["#e"], json!([other]));
    // (b) retired the stale card and (c) reposted it fresh.
    let retired = b.posts()[before..].iter().any(|(e, _)| {
        e["kind"] == 5 && tag_value(e, "e").as_deref() == Some(card.event_id.as_str())
    });
    assert!(retired);
    assert_ne!(b.card(ITEM).event_id, card.event_id);
}

#[tokio::test]
async fn past_deadline_pending_folds() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    b.drop_file(
        "orchestrator",
        "relay-choice",
        &drop_json(&[("deadline", json!("2020-01-01T00:00:00Z"))]),
    );
    b.settle(4).await;
    let item = &b.s.items[ITEM];
    assert!(item.validity == Validity::Superseded && item.card.is_none());
    let posts = b.posts();
    assert!(!posts
        .iter()
        .any(|(e, _)| { tag_value(e, "client").is_some_and(|c| c.starts_with("bench:card:")) }));
    let board = posts.iter().rev().find(|(e, _)| e["kind"] == 9).unwrap();
    assert!(!board.0["content"].as_str().unwrap().contains("Needs you"));
    // An answered item waits for the writer, deadline or not.
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let owner_tap = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    b.s.items.get_mut(ITEM).unwrap().deadline = Some("2020-01-01T00:00:00Z".to_string());
    b.tick().await.unwrap();
    assert_eq!(b.s.items[ITEM].validity, Validity::Current);
}

#[tokio::test]
async fn retracted_answer_file_is_swept_when_state_forgot() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let owner_tap = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    assert!(outbox(&b).is_some());
    // A crash before save_state (or a remove the consumer's open handle
    // refused) left the file with no answer in state; then the owner untaps.
    b.s.items.get_mut(ITEM).unwrap().answer = None;
    reply(&b, &[]);
    b.tick().await.unwrap();
    assert!(outbox(&b).is_none());
}

#[tokio::test]
async fn truncated_answers_are_reported() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let owner_tap = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &vec![&owner_tap; RECOVER_LIMIT as usize]);
    let err = b.tick().await.unwrap_err();
    assert!(err.contains("answers truncated"), "{err}");
    // A cut page never records an answer.
    assert!(b.s.items[ITEM].answer.is_none() && outbox(&b).is_none());
}

#[tokio::test]
async fn kept_answered_cards_count_against_max_cards() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let owner_tap = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    for i in 0..MAX_CARDS {
        b.drop_file("orchestrator", &format!("c{i:02}"), &drop_json(&[]));
    }
    b.settle(26).await;
    // The kept answered card holds one of the MAX_CARDS slots, so the last
    // pending item (by id) waits for one.
    let live = b.s.items.values().filter(|i| i.card.is_some()).count();
    assert_eq!(live, MAX_CARDS);
    assert!(b.s.items[ITEM].card.is_some());
    assert!(b.s.items["orchestrator/c28"].card.is_some());
    assert!(b.s.items["orchestrator/c29"].card.is_none());
    // A day on it is a receipt only: it stays, and gives its slot back.
    let card = b.card(ITEM).event_id;
    let before = b.posts().len();
    b.now += ANSWER_KEEP_SECS;
    b.settle(4).await;
    assert_eq!(b.card(ITEM).event_id, card);
    assert!(b.s.items["orchestrator/c29"].card.is_some());
    assert!(!b.posts()[before..]
        .iter()
        .any(|(e, _)| e["kind"] == 5 && tag_value(e, "e").as_deref() == Some(card.as_str())));
}

// ── Saying how to answer ────────────────────────────────────────────────

const LEGEND: &str =
    "To answer: tap one number under a card. To undo or change it, tap that number again first.";
const HINT: &str = "Tap one number below to answer.";

#[tokio::test]
async fn legend_is_board_line_2_and_lands_by_edit() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let board = text(&b.posts().last().unwrap().0).to_string();
    assert_eq!(board.lines().nth(1), Some(LEGEND));
    // The board an older build posted: the same text without the legend.
    let mut older: Vec<&str> = board.lines().collect();
    older.remove(1);
    b.s.board.as_mut().unwrap().hash = board_hash(&older.join("\n"));
    let before = b.posts().len();
    b.tick().await.unwrap();
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [40003]);
    assert_eq!(tag_value(&posts[0].0, "e").unwrap(), board_id(&b));
    assert_eq!(text(&posts[0].0), board);
}

#[test]
fn needs_you_lines_are_bullets_not_numbers() {
    let a = check(&[("id", json!("a"))]).unwrap();
    let c = check(&[("id", json!("c")), ("severity", json!("info"))]).unwrap();
    let board = render_board(&[&a, &c], UUID, "14:05", 0);
    let needs: Vec<&str> = board
        .lines()
        .skip_while(|l| *l != "Needs you")
        .skip(1)
        .collect();
    assert_eq!(needs.len(), 2);
    assert!(needs.iter().all(|l| l.starts_with("- ")), "{board}");
    assert!(
        !board
            .lines()
            .any(|l| l.starts_with(|c: char| c.is_ascii_digit())),
        "{board}"
    );
}

#[test]
fn answer_hint_only_on_unanswered_pending_cards() {
    let mut pending = check(&[]).unwrap();
    let card = render_card(&pending, "14:05");
    assert!(
        card.ends_with(&format!("2. Move\n\n{HINT}\nas of 14:05")),
        "{card}"
    );
    pending.answer = answered(0, 1);
    let card = render_card(&pending, "14:05");
    assert!(!card.contains(HINT) && card.ends_with("Recorded: 1. Stay · 14:05"));
    let no_options = [("severity", Value::Null), ("options", Value::Null)];
    let decision = check(
        &[
            [("kind", json!("decision")), ("decidedBy", json!("claude"))].as_slice(),
            &no_options,
        ]
        .concat(),
    )
    .unwrap();
    let status = check(
        &[
            [("kind", json!("status")), ("state", json!("done"))].as_slice(),
            &no_options,
        ]
        .concat(),
    )
    .unwrap();
    for other in [decision, status] {
        assert!(!render_card(&other, "14:05").contains(HINT));
    }
}

#[tokio::test]
async fn older_card_format_is_reposted_once_and_answers_survive() {
    // Before CARD_FORMAT (the bare Drop hash), format 2 (text in a fence)
    // and format 3 (the summary in a fence).
    for older in ["", ".2", ".3"] {
        older_card_format_case(older).await;
    }
}

async fn older_card_format_case(older: &str) {
    const OTHER: &str = "orchestrator/other";
    let mut b = seeded_bench(Gate::Open, &[]).await;
    b.drop_file(
        "orchestrator",
        "other",
        &drop_json(&[("title", json!("Other"))]),
    );
    b.settle(3).await;
    // Both cards as an older build tagged them, one tapped before the upgrade.
    for i in b.s.items.values_mut() {
        i.card.as_mut().unwrap().hash = format!("{}{older}", i.hash);
    }
    let (kept, old) = (b.card(ITEM).event_id, b.card(OTHER).event_id);
    let owner_tap = tap(&b.ctx.owner, &kept, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    let before = b.posts().len();
    b.settle(4).await;
    let answer = b.s.items[ITEM].answer.clone();
    assert!(answer.is_some());
    // The tapped card takes the new text by its Recorded edit; only the
    // unanswered one is refreshed: delete, quiet repost, seeds, board pair.
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [40003, 5, 9, 7, 7, 9, 5]);
    assert_eq!(tag_value(&posts[0].0, "e").unwrap(), kept);
    assert_eq!(tag_value(&posts[1].0, "e").unwrap(), old);
    let new_card = b.card(OTHER);
    assert_eq!(
        tag_value(&posts[2].0, "client").unwrap(),
        format!("bench:card:{OTHER}@{}", card_hash(&b.s.items[OTHER]))
    );
    assert!(text(&posts[0].0).contains("\n**Relay choice**\n"));
    assert!(text(&posts[2].0).contains("\n**Other**\n") && text(&posts[2].0).contains(HINT));
    assert_eq!(tag_value(&posts[2].0, "p"), None);
    assert!(!b.s.items[OTHER].quiet_repost);
    // The answered card, its answer and its outbox file stay.
    assert_eq!(b.card(ITEM).event_id, kept);
    assert!(outbox(&b).is_some());
    // A relaunch rebuilds from the relay's tags and reposts nothing again.
    let board = board_id(&b);
    let mut relay: Vec<nostr::Event> = b
        .posts()
        .into_iter()
        .map(|(e, _)| serde_json::from_value::<nostr::Event>(e).unwrap())
        .filter(|e| [&new_card.event_id, &board].contains(&&e.id.to_hex()))
        .collect();
    relay.push(owner_tap);
    reply(&b, &relay.iter().collect::<Vec<_>>());
    b.s.items.get_mut(OTHER).unwrap().card = None;
    b.s.needs_rebuild = true;
    let before = b.posts().len();
    b.settle(3).await;
    assert_eq!(b.posts().len(), before);
    assert_eq!(b.card(OTHER).event_id, new_card.event_id);
    assert_eq!(b.s.items[ITEM].answer, answer);
    // Undo on the older card un-records it in place, like on any card.
    reply(&b, &[]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert_eq!(b.kinds()[before..], [40003, 40003]);
    assert_eq!(b.card(ITEM).event_id, kept);
    assert!(b.s.items[ITEM].answer.is_none() && outbox(&b).is_none());
}

#[tokio::test]
async fn recorded_answer_on_older_format_card_survives_the_refresh() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM);
    let owner_tap = tap(&b.ctx.owner, &card.event_id, OPTION_EMOJI[1], b.now - 5);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    assert_eq!(b.s.items[ITEM].recorded, Some(1));
    // As an M2 build without CARD_FORMAT saved it: answered, older tag.
    let hash = b.s.items[ITEM].hash.clone();
    b.s.items.get_mut(ITEM).unwrap().card.as_mut().unwrap().hash = hash;
    let before = b.posts().len();
    b.settle(3).await;
    assert!(!b.kinds()[before..].iter().any(|k| *k == 5 || *k == 9));
    assert_eq!(b.card(ITEM).event_id, card.event_id);
    assert!(b.s.items[ITEM].answer.is_some() && outbox(&b).is_some());
}

// ── Card text the phone can read ────────────────────────────────────────

#[test]
fn render_card_title_and_options_are_inert() {
    // Everything either app gives meaning to, an entity, a dropped character
    // that would join "www.", a custom emoji shortcode and a trailing rule.
    let active = "[a](https://x.example) ![i](u) @Honey #general |a|b| *em* _em_ `c` \
                  ~~s~~ <u>u</u> \\(x\\) \\[y\\] $z$ &num;ch w[w]w.x.example :party: ---";
    let mut item = check(&[]).unwrap();
    item.title = format!("# {active}");
    item.options = ["- ", "> ", "1. ", "(x) "]
        .map(|p| format!("{p}{active}"))
        .to_vec();
    for answer in [None, answered(2, 1)] {
        item.answer = answer;
        let card = render_card(&item, "14:05");
        // Title, summary paragraph, then the options.
        let (head, rest) = card.split_once("\n\n").unwrap();
        let (_, tail) = rest.split_once("\n\n").unwrap();
        let lines: Vec<&str> = head.lines().chain(tail.lines()).skip(1).collect();
        assert!(
            lines[0].starts_with("**") && lines[0].ends_with("**"),
            "{card}"
        );
        for (i, n) in ["1. ", "2. ", "3. ", "4. "].iter().enumerate() {
            assert!(lines[i + 1].starts_with(n), "{card}");
        }
        for (i, line) in lines.iter().enumerate().filter(|(_, l)| !l.is_empty()) {
            // No `:name:` the phone resolves against the community palette.
            let parts: Vec<&str> = line.split(':').collect();
            let shortcode = parts.len() > 2
                && parts[1..parts.len() - 1].iter().any(|p| {
                    !p.is_empty()
                        && p.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                });
            assert!(!shortcode, "{line:?}");
            let text = line
                .strip_prefix("**")
                .and_then(|l| l.strip_suffix("**"))
                .or_else(|| {
                    line.get(3..)
                        .filter(|_| line.as_bytes().get(1) == Some(&b'.'))
                })
                .unwrap_or(line);
            assert!(
                !text.contains(|c: char| "[]@#|*_`~<>\\$;/".contains(c)),
                "{text:?}"
            );
            assert!(!text.contains("www.") && !text.ends_with('-'), "{text:?}");
            // The title and the options are plain_text alone.
            if i < 5 {
                let glued = text
                    .as_bytes()
                    .windows(2)
                    .any(|w| w[0] == b':' && w[1] != b' ');
                assert!(!glued, "{text:?}");
            }
            // No nested list, quote, heading or radio button either.
            assert!(
                text.starts_with(|c: char| c.is_ascii_alphanumeric()),
                "{text:?}"
            );
            let lead = text.split(' ').next().unwrap();
            let ordinal = lead
                .strip_suffix(['.', ')'])
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
            assert!(!ordinal, "{text:?}");
        }
    }
}

#[test]
fn render_card_summary_is_its_own_inert_paragraph() {
    // No code box: the summary is a paragraph between blank lines, with no
    // backtick left to open one, so nothing in it reaches the options.
    for body in [
        "x\n```\n[a](https://e.example)\n```",
        "Run this first:\n```bash\ncargo test\n```\nthen pick *one",
        "inline ```x``` and ````y```` [z",
    ] {
        let card = render_card(&check(&[("body", json!(body))]).unwrap(), "14:05");
        assert!(!card.contains('`'), "{card}");
        let paras: Vec<&str> = card.split("\n\n").collect();
        assert_eq!(
            paras[..3],
            [
                "🔴 · dreamforge · orchestrator\n**Relay choice**",
                prose(body).as_str(),
                "1. Stay\n2. Move"
            ]
        );
    }
}

#[test]
fn prose_keeps_plain_text_and_collapses_spaces() {
    let plain = "Block's hosted relay rejects the new event kinds, so Bench-in-Buzz \
                 cannot ship there. Stay and wait for upstream (a week, maybe 2), \
                 or move the community to our own relay?\n\n\
                 Either way - it's 3.5 hours of work!";
    assert_eq!(prose(plain), plain);
    let card = render_card(&check(&[("body", json!(plain))]).unwrap(), "14:05");
    assert!(
        card.contains(&format!("**\n\n{plain}\n\n1. Stay")),
        "{card}"
    );
    // No indent (a code block or nested item) and no trailing hard break.
    assert_eq!(prose("\n  a \t b  \n\n\t c  \n"), "a b\n\nc");
}

#[test]
fn prose_breaks_each_trigger() {
    // gpt_markdown 1.1.6 honours no backslash escape, so each trigger either
    // side gives meaning to is broken by an invisible U+FEFF instead.
    let z = |s: &str| s.replace('°', "\u{FEFF}");
    for (raw, want) in [
        // Emphasis and strikethrough cannot close; `**`, `~~`, `~~~` split.
        ("*em* and **b**", "°*em°* and °*°*b°*°*"),
        ("_em_ and __b__", "°_em°_ and °_°_b°_°_"),
        ("~s~ and ~~s~~", "°~s°~ and °~°~s°~°~"),
        // Code: U+02CB, since a U+FEFF cannot split a code span.
        ("`c` and ```f```", "ˋcˋ and ˋˋˋfˋˋˋ"),
        // Heading, #channel, @mention, email, :emoji:, autolinks, entities.
        ("# h #general", "#° h #°general"),
        ("@Honey a@b.example", "@°Honey a@°b.example"),
        (
            ":party: https://github.com/x",
            ":°party:° https:°//github.com/x",
        ),
        (
            "&num;ch <u>u</u> <https://e.x>",
            "&°num;ch <°u>u<°/u> <°https:°//e.x>",
        ),
        // Links, images, citations, footnotes, LaTeX and escapes.
        ("[a](u) ![i](u) [1] [^n]", "[°a]°(u) ![°i]°(u) [°1]° [°^n]°"),
        ("\\(x\\) \\[y\\] \\*", "\\°(x\\°) \\°[°y\\°]° \\°°*"),
        // A line ending in --- is a rule on the phone; || a desktop spoiler.
        ("wait --- or ||s||", "wait -°-°- or |°|s|°|"),
        // Block marks at a line start.
        (
            "- a\n+ b\n> c\n1. d\n12) e",
            "°- a\n°+ b\n°> c\n°1. d\n°12) e",
        ),
        ("(x) r\n===\n\u{2E3B} x", "°(x) r\n°===\n°\u{2E3B} x"),
        ("| a | b |\n|---|---|", "°| a | b |\n°|-°-°-|-°-°-|"),
    ] {
        let out = prose(raw);
        assert_eq!(out, z(want), "{raw:?}");
        // Both apps show exactly the text that came in.
        let shown = out.replace('\u{FEFF}', "").replace('\u{02CB}', "`");
        assert_eq!(shown, raw);
    }
}

#[test]
fn plain_text_folds_typography_and_spaces_colons() {
    for (raw, plain) in [
        (
            "Don\u{2019}t merge \u{2014} wait for CI",
            "Don't merge - wait for CI",
        ),
        ("Merge \u{201C}now\u{201D}", "Merge \"now\""),
        (
            "it\u{02BC}s \u{2018}a\u{2019} \u{2013} b\u{2026}",
            "it's 'a' - b...",
        ),
        ("\u{2014} later \u{2014}", "later"),
        ("Ship it :party:", "Ship it : party:"),
        ("Note: at 14:05", "Note: at 14: 05"),
    ] {
        assert_eq!(plain_text(raw, usize::MAX), plain, "{raw:?}");
    }
}

#[test]
fn render_card_omits_a_title_that_sanitizes_to_nothing() {
    // `****` is a thematic break on the desktop, so an all-symbol title must
    // leave no bold line behind.
    let mut item = check(&[]).unwrap();
    item.title = "!!!".to_string();
    let card = render_card(&item, "14:05");
    assert!(
        !card
            .lines()
            .any(|l| l.trim() == "****" || l.starts_with("**")),
        "{card}"
    );
}
