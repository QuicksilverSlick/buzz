//! Desktop cards: bench_cards' snapshot and bench_answer's relay half,
//! through the fake relay and Bench harness in tests.rs.

use super::answers::{last_query, outbox, reply, tap, ITEM};
use super::channels::{bound_bench, hex, ASK, X};
use super::*;
use crate::bench::commands::{answer_card, cards, plan_answer, AnswerPlan, CARD_CHANGED};

fn plan(add: Option<&'static str>, delete: &[&str]) -> AnswerPlan {
    AnswerPlan {
        add,
        delete: delete.iter().map(|d| d.to_string()).collect(),
    }
}

/// One bench_answer: the posts it made, as events the relay now holds.
async fn answer(b: &Bench, option: Option<u8>) -> Vec<nostr::Event> {
    answer_on(b, ITEM, option).await
}

/// Answered on the card the owner sees: the live one.
async fn answer_on(b: &Bench, id: &str, option: Option<u8>) -> Vec<nostr::Event> {
    let before = b.posts().len();
    let card = b.card(id).event_id;
    answer_card(&b.ctx, &b.s, id, &card, option, b.now)
        .await
        .unwrap();
    b.posts()[before..]
        .iter()
        .map(|(e, tag)| {
            assert_eq!(*tag, None, "owner posts carry no auth tag");
            serde_json::from_value(e.clone()).unwrap()
        })
        .collect()
}

fn kinds(evs: &[nostr::Event]) -> Vec<u16> {
    evs.iter().map(|e| e.kind.as_u16()).collect()
}

#[test]
fn plan_answer_rules() {
    let k = OPTION_EMOJI;
    let r = |id: &str, c: &str| (id.to_string(), c.to_string());
    // Choose on a bare card.
    assert_eq!(plan_answer(&[], Some(1), 2), plan(Some(k[1]), &[]));
    // Switch: the new keycap, then the old one goes.
    let one = [r("a", k[0])];
    assert_eq!(plan_answer(&one, Some(1), 2), plan(Some(k[1]), &["a"]));
    // Re-choose the same: nothing to do.
    assert_eq!(plan_answer(&one, Some(0), 2), AnswerPlan::default());
    // Re-choose one that is already there under another: only the delete.
    let both = [r("a", k[0]), r("b", k[1])];
    assert_eq!(plan_answer(&both, Some(0), 2), plan(None, &["b"]));
    // Clear: every keycap goes.
    assert_eq!(plan_answer(&both, None, 2), plan(None, &["a", "b"]));
    // Foreign content stays: a heart, a keycap past this card's options and
    // an unqualified keycap.
    let noise = [
        r("h", "\u{2764}\u{FE0F}"),
        r("x", k[2]),
        r("u", "1\u{20E3}"),
    ];
    assert_eq!(plan_answer(&noise, Some(0), 2), plan(Some(k[0]), &[]));
    assert_eq!(plan_answer(&noise, None, 2), AnswerPlan::default());
    // Out of range plans nothing, not even the clear; more than four options
    // clamp to the four keycaps.
    assert_eq!(plan_answer(&both, Some(2), 2), AnswerPlan::default());
    assert_eq!(plan_answer(&both, Some(4), 9), AnswerPlan::default());
    assert_eq!(plan_answer(&[], Some(3), 9), plan(Some(k[3]), &[]));
}

#[tokio::test]
async fn bench_answer_posts_owner_keycaps_and_the_tick_records_them() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    let owner = b.owner_hex();
    let stranger = Keys::generate();
    let noise = [
        tap(&b.ctx.owner, &card, "\u{2764}\u{FE0F}", b.now),
        tap(&stranger, &card, OPTION_EMOJI[1], b.now),
        tap(&b.ctx.owner, &"ab".repeat(32), OPTION_EMOJI[1], b.now),
    ];
    let mut relay: Vec<nostr::Event> = noise.to_vec();

    // First answer: exactly one owner kind 7, tagged like build_reaction.
    reply(&b, &relay.iter().collect::<Vec<_>>());
    let posted = answer(&b, Some(1)).await;
    assert_eq!(kinds(&posted), [7]);
    let first = posted[0].clone();
    assert_eq!(first.pubkey.to_hex(), owner);
    assert_eq!(first.content, OPTION_EMOJI[1]);
    assert_eq!(
        first.tags,
        tap(&b.ctx.owner, &card, OPTION_EMOJI[1], 0).tags,
        "tags exactly like events::build_reaction"
    );
    // The owner's own keycaps on this card, in its channel.
    assert_eq!(
        last_query(&b),
        json!([{
            "kinds": [7], "authors": [owner], "#h": [b.channel()],
            "#e": [card], "limit": 500,
        }])
    );
    relay.push(first.clone());
    reply(&b, &relay.iter().collect::<Vec<_>>());
    b.tick().await.unwrap();
    let item = &b.s.items[ITEM];
    assert_eq!(
        item.answer
            .as_ref()
            .map(|a| (a.option, a.reaction_id.clone())),
        Some((1, first.id.to_hex()))
    );
    assert_eq!(item.recorded, Some(1));
    assert_eq!(outbox(&b).unwrap()["option"], json!(2));

    // Re-choosing the same option publishes nothing.
    assert!(answer(&b, Some(1)).await.is_empty());

    // Switch: the new keycap first, then a kind 5 on the old one.
    let posted = answer(&b, Some(0)).await;
    assert_eq!(kinds(&posted), [7, 5]);
    let second = posted[0].clone();
    assert_eq!(second.content, OPTION_EMOJI[0]);
    assert_eq!(
        second.tags,
        tap(&b.ctx.owner, &card, OPTION_EMOJI[0], 0).tags
    );
    let del = serde_json::to_value(&posted[1]).unwrap();
    assert_eq!(posted[1].pubkey.to_hex(), owner);
    assert_eq!(del["tags"], json!([["e", first.id.to_hex()]]));
    relay.retain(|e| e.id != first.id);
    relay.push(second.clone());
    reply(&b, &relay.iter().collect::<Vec<_>>());
    b.tick().await.unwrap();
    assert_eq!(b.s.items[ITEM].answer.as_ref().map(|a| a.option), Some(0));
    assert_eq!(outbox(&b).unwrap()["option"], json!(1));

    // Clear: only the owner's keycap goes; the heart, the stranger's tap and
    // the reaction on another event stay.
    let posted = answer(&b, None).await;
    assert_eq!(kinds(&posted), [5]);
    let del = serde_json::to_value(&posted[0]).unwrap();
    assert_eq!(del["tags"], json!([["e", second.id.to_hex()]]));
    relay.retain(|e| e.id != second.id);
    reply(&b, &relay.iter().collect::<Vec<_>>());
    b.tick().await.unwrap();
    let item = &b.s.items[ITEM];
    assert!(item.answer.is_none() && item.recorded.is_none());
    assert!(outbox(&b).is_none());
}

#[tokio::test]
async fn bench_answer_in_a_project_channel_overrides_craig() {
    let (mut b, p, list) = bound_bench().await;
    let card = b.card(ASK).event_id;
    let owner = b.owner_hex();
    let craig_tap = tap(&p.craig, &card, OPTION_EMOJI[0], b.now);
    let mut relay = vec![list, craig_tap];
    async fn tick_with(b: &mut Bench, relay: &[nostr::Event]) -> Option<(usize, String)> {
        reply(b, &relay.iter().collect::<Vec<_>>());
        b.tick().await.unwrap();
        let a = b.s.items[ASK].answer.as_ref()?;
        Some((a.option, a.by.clone()))
    }
    assert_eq!(tick_with(&mut b, &relay).await, Some((0, hex(&p.craig))));

    // Answering as owner: one owner keycap, planned in X against the
    // owner's own taps only; Craig's tap is never outranked or touched.
    let posted = answer_on(&b, ASK, Some(1)).await;
    assert_eq!(kinds(&posted), [7]);
    assert_eq!(posted[0].created_at.as_secs(), b.now);
    assert_eq!(
        last_query(&b),
        json!([{
            "kinds": [7], "authors": [owner], "#h": [X], "#e": [card], "limit": 500,
        }])
    );
    relay.push(posted[0].clone());
    assert_eq!(tick_with(&mut b, &relay).await, Some((1, owner.clone())));

    // The same option by Craig and the owner: the owner's still stands.
    let owner_1 = posted[0].clone();
    relay.push(tap(&p.craig, &card, OPTION_EMOJI[1], b.now + 5));
    assert_eq!(tick_with(&mut b, &relay).await, Some((1, owner.clone())));

    // Clearing deletes only the owner's keycap; Craig's newest answer stands.
    let posted = answer_on(&b, ASK, None).await;
    assert_eq!(kinds(&posted), [5]);
    let del = serde_json::to_value(&posted[0]).unwrap();
    assert_eq!(del["tags"], json!([["e", owner_1.id.to_hex()]]));
    relay.retain(|e| e.id != owner_1.id);
    assert_eq!(tick_with(&mut b, &relay).await, Some((1, hex(&p.craig))));
}

#[tokio::test]
async fn bench_answer_refuses_what_it_cannot_answer() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let no_options = [("severity", Value::Null), ("options", Value::Null)];
    let status = check(
        &[
            [
                ("id", json!("st")),
                ("kind", json!("status")),
                ("state", json!("done")),
            ]
            .as_slice(),
            &no_options,
        ]
        .concat(),
    )
    .unwrap();
    let uncarded = check(&[("id", json!("uncarded"))]).unwrap();
    // A card that still shows an older Drop of its item.
    let mut stale = check(&[("id", json!("stale"))]).unwrap();
    stale.card = Some(Posted {
        hash: "ffff.2".to_string(),
        ..b.card(ITEM)
    });
    for i in [status, uncarded, stale] {
        b.s.items.insert(i.id.clone(), i);
    }
    let posts = b.posts().len();
    let queries = b.relay.queries.lock().unwrap().len();
    let live = b.card(ITEM).event_id;
    for (id, option, want) in [
        ("orchestrator/nope", Some(0), "no Bench item"),
        ("orchestrator/st", Some(0), "not a pending item"),
        ("orchestrator/uncarded", Some(0), CARD_CHANGED),
        ("orchestrator/stale", None, CARD_CHANGED),
        (ITEM, Some(2), "out of range"),
    ] {
        let err = answer_card(&b.ctx, &b.s, id, &live, option, b.now)
            .await
            .unwrap_err();
        assert!(err.contains(want), "{id}: {err}");
    }
    b.s.owner_pubkey = "bb".repeat(32);
    let err = answer_card(&b.ctx, &b.s, ITEM, &live, Some(0), b.now)
        .await
        .unwrap_err();
    assert!(err.contains("owner identity changed"), "{err}");
    assert_eq!(b.posts().len(), posts);
    assert_eq!(b.relay.queries.lock().unwrap().len(), queries);
}

#[tokio::test]
async fn bench_cards_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let off = serde_json::to_value(cards(&dir.path().join("missing"))).unwrap();
    assert_eq!(off, json!({ "enabled": false }));
    // Enabled before the first tick: no state.json yet.
    let fresh = serde_json::to_value(cards(dir.path())).unwrap();
    assert_eq!(fresh["enabled"], json!(true));
    assert_eq!(fresh["ownerPubkey"], Value::Null);
    assert_eq!(fresh["items"], json!([]));

    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    let owner_tap = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    b.s.writer_pubkey = b.writer_hex();
    b.s.last_error = Some("relay unreachable".to_string());
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let snap = serde_json::to_value(cards(&b.root)).unwrap();
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    let top = keys(&snap);
    assert_eq!(
        top,
        [
            "channelId",
            "enabled",
            "items",
            "lastError",
            "lastTick",
            "ownerPubkey",
            "writerPubkey"
        ]
    );
    assert_eq!(snap["channelId"], json!(b.channel()));
    assert_eq!(snap["writerPubkey"], json!(b.writer_hex()));
    assert_eq!(snap["ownerPubkey"], json!(b.owner_hex()));
    assert_eq!(snap["lastError"], json!("relay unreachable"));
    assert_eq!(snap["lastTick"], json!(NOW));
    let item = &snap["items"][0];
    assert_eq!(
        keys(item),
        [
            "action",
            "answer",
            "answerers",
            "area",
            "backToYou",
            "body",
            "cardEventId",
            "channelId",
            "deadline",
            "decidedBy",
            "id",
            "kind",
            "links",
            "needs",
            "options",
            "order",
            "ownerName",
            "plain",
            "previousAnswer",
            "project",
            "recommend",
            "recommender",
            "recorded",
            "reply",
            "request",
            "scoped",
            "severity",
            "state",
            "title",
            "updatedAt",
            "validity",
            "writer"
        ]
    );
    assert_eq!(item["updatedAt"], json!(NOW));
    assert_eq!(item["id"], json!(ITEM));
    assert_eq!(item["kind"], json!("pending"));
    assert_eq!(item["severity"], json!("critical"));
    assert_eq!(item["validity"], json!("current"));
    assert_eq!(item["options"], json!(["Stay", "Move"]));
    assert_eq!(item["cardEventId"], json!(card));
    assert_eq!(
        item["answer"],
        json!({
            "option": 1, "answeredAt": b.now, "by": b.owner_hex(), "byName": null,
            "byOwner": true, "source": "tap",
        })
    );
    assert_eq!(item["recorded"], json!(1));
    // A legacy card: #bench, owner-only, no Other.
    for (k, v) in [
        ("channelId", Value::Null),
        ("project", Value::Null),
        ("scoped", json!(false)),
        ("needs", json!("you")),
        ("answerers", json!([])),
        ("ownerName", Value::Null),
        ("recommend", Value::Null),
        ("recommender", Value::Null),
        ("reply", Value::Null),
        ("request", Value::Null),
        ("plain", Value::Null),
        ("action", Value::Null),
        ("backToYou", json!(false)),
        ("previousAnswer", Value::Null),
    ] {
        assert_eq!(item[k], v, "{k}");
    }
    // Only a live card is offered for answering.
    b.s.items.get_mut(ITEM).unwrap().hash = "ffff".to_string();
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let snap = serde_json::to_value(cards(&b.root)).unwrap();
    assert_eq!(snap["items"][0]["cardEventId"], Value::Null);
}

#[tokio::test]
async fn bench_cards_snapshot_names_answerers_and_shows_receipts() {
    let (mut b, p, list) = bound_bench().await;
    let card = b.card(ASK).event_id;
    let craig_tap = tap(&p.craig, &card, OPTION_EMOJI[0], b.now);
    reply(&b, &[&list, &craig_tap]);
    b.tick().await.unwrap();
    let find = |snap: &Value| -> Value {
        snap["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == ASK)
            .unwrap()
            .clone()
    };
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let item = find(&serde_json::to_value(cards(&b.root)).unwrap());
    assert_eq!(item["channelId"], json!(X));
    assert_eq!(item["project"], json!("reset"));
    assert_eq!(item["scoped"], json!(true));
    assert_eq!(
        item["answerers"],
        json!([{"pk": hex(&p.craig), "name": "Craig", "role": "decider", "agent": false}])
    );
    assert_eq!(item["ownerName"], json!("Quicksilver"));
    assert_eq!(item["recommend"], json!(2));
    assert_eq!(item["recommender"], json!("Reset Worker"));
    assert_eq!(item["cardEventId"], json!(card));
    assert_eq!(item["answer"]["by"], json!(hex(&p.craig)));
    assert_eq!(item["answer"]["byName"], json!("Craig"));
    assert_eq!(item["answer"]["byOwner"], json!(false));

    // The close-out: the receipt keeps the card and carries the question.
    let close = drop_json(&[
        ("kind", json!("decision")),
        ("decidedBy", json!("claude")),
        ("project", json!("reset")),
        ("severity", Value::Null),
        ("options", Value::Null),
        ("title", json!("Sent the new email")),
    ]);
    b.drop_file("reset-worker", "welcome", &close);
    b.tick().await.unwrap();
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let item = find(&serde_json::to_value(cards(&b.root)).unwrap());
    assert_eq!(item["kind"], json!("decision"));
    assert_eq!(item["title"], json!("Sent the new email"));
    assert_eq!(item["cardEventId"], json!(card));
    assert_eq!(item["channelId"], json!(X));
    let request = &item["request"];
    assert_eq!(request["kind"], json!("pending"));
    assert_eq!(request["title"], json!("Welcome email"));
    assert_eq!(request["cardEventId"], Value::Null);
    assert_eq!(request["answer"]["byName"], json!("Craig"));
    assert_eq!(request["request"], Value::Null);
}
