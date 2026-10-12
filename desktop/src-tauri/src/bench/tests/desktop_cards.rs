//! Desktop cards: bench_cards' snapshot and bench_answer's relay half,
//! through the fake relay and Bench harness in tests.rs.

use super::answers::{last_query, outbox, reply, tap, ITEM};
use super::*;
use crate::bench::commands::{answer_card, cards, plan_answer, AnswerPlan, ANSWERED_ELSEWHERE};

fn plan(add: Option<&'static str>, delete: &[&str]) -> AnswerPlan {
    AnswerPlan {
        add,
        delete: delete.iter().map(|d| d.to_string()).collect(),
    }
}

/// One bench_answer: the posts it made, as events the relay now holds.
async fn answer(b: &Bench, option: Option<u8>) -> Vec<nostr::Event> {
    let before = b.posts().len();
    answer_card(&b.ctx, &b.s, ITEM, option, b.now)
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
    // The poll's own filter and approvers, narrowed to this card.
    assert_eq!(
        last_query(&b),
        json!([{
            "kinds": [7], "authors": [owner, "cd".repeat(32)], "#h": [b.channel()],
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
async fn bench_answer_follows_latest_wins_across_approver_keys() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    // The phone's own key, which answers through members.txt.
    let phone = Keys::generate();
    std::fs::write(b.root.join("members.txt"), phone.public_key().to_hex()).unwrap();
    let mut relay: Vec<nostr::Event> = Vec::new();
    async fn tick_with(b: &mut Bench, relay: &[nostr::Event]) -> Option<(usize, String)> {
        reply(b, &relay.iter().collect::<Vec<_>>());
        b.tick().await.unwrap();
        let a = b.s.items[ITEM].answer.as_ref()?;
        Some((a.option, a.reaction_id.clone()))
    }
    async fn refused(b: &Bench, option: Option<u8>) -> String {
        let before = b.posts().len();
        let err = answer_card(&b.ctx, &b.s, ITEM, option, b.now)
            .await
            .unwrap_err();
        assert_eq!(b.posts().len(), before, "a refusal publishes nothing");
        err
    }

    // The phone answers 2; undo on the desktop cannot delete its tap.
    let phone_2 = tap(&phone, &card, OPTION_EMOJI[1], b.now);
    relay.push(phone_2.clone());
    assert_eq!(
        tick_with(&mut b, &relay).await,
        Some((1, phone_2.id.to_hex()))
    );
    assert_eq!(refused(&b, None).await, ANSWERED_ELSEWHERE);
    assert_eq!(
        last_query(&b)[0]["authors"],
        json!([b.owner_hex(), phone.public_key().to_hex()])
    );

    // Choosing 1 on the desktop outranks it.
    let posted = answer(&b, Some(0)).await;
    assert_eq!(kinds(&posted), [7]);
    let desk_1 = posted[0].clone();
    relay.push(desk_1.clone());
    assert_eq!(
        tick_with(&mut b, &relay).await,
        Some((0, desk_1.id.to_hex()))
    );
    // Undo here would hand the answer back to the phone's older 2.
    assert_eq!(refused(&b, None).await, ANSWERED_ELSEWHERE);

    // The phone taps 2 again after the owner, its clock 100 s ahead.
    relay.retain(|e| e.id != phone_2.id);
    let phone_2b = tap(&phone, &card, OPTION_EMOJI[1], b.now + 100);
    relay.push(phone_2b.clone());
    assert_eq!(
        tick_with(&mut b, &relay).await,
        Some((1, phone_2b.id.to_hex()))
    );
    // Choosing 1 again: the owner's 1 is there but outranked, so it is
    // deleted first (the relay refuses an active duplicate) and posted anew,
    // stamped past the phone's tap.
    let posted = answer(&b, Some(0)).await;
    assert_eq!(kinds(&posted), [5, 7]);
    let del = serde_json::to_value(&posted[0]).unwrap();
    assert_eq!(del["tags"], json!([["e", desk_1.id.to_hex()]]));
    let desk_1b = posted[1].clone();
    assert_eq!(desk_1b.content, OPTION_EMOJI[0]);
    assert_eq!(
        desk_1b.created_at.as_secs(),
        phone_2b.created_at.as_secs() + 1
    );
    relay.retain(|e| e.id != desk_1.id);
    relay.push(desk_1b.clone());
    assert_eq!(
        tick_with(&mut b, &relay).await,
        Some((0, desk_1b.id.to_hex()))
    );
    assert!(answer(&b, Some(0)).await.is_empty());

    // A tap dated past the clock lead cannot be outranked from here, and the
    // owner's keycap is not deleted trying.
    relay.retain(|e| e.id != phone_2b.id);
    let far = tap(&phone, &card, OPTION_EMOJI[1], b.now + 10_000);
    relay.push(far.clone());
    assert_eq!(tick_with(&mut b, &relay).await, Some((1, far.id.to_hex())));
    assert!(refused(&b, Some(0)).await.contains("dated in the future"));
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
    for (id, option, want) in [
        ("orchestrator/nope", Some(0), "no Bench item"),
        ("orchestrator/st", Some(0), "not a pending item"),
        ("orchestrator/uncarded", Some(0), "no live card"),
        ("orchestrator/stale", None, "no live card"),
        (ITEM, Some(2), "out of range"),
    ] {
        let err = answer_card(&b.ctx, &b.s, id, option, b.now)
            .await
            .unwrap_err();
        assert!(err.contains(want), "{id}: {err}");
    }
    b.s.owner_pubkey = "bb".repeat(32);
    let err = answer_card(&b.ctx, &b.s, ITEM, Some(0), b.now)
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
            "answer",
            "area",
            "body",
            "cardEventId",
            "deadline",
            "decidedBy",
            "id",
            "kind",
            "links",
            "options",
            "order",
            "recorded",
            "severity",
            "state",
            "title",
            "validity",
            "writer"
        ]
    );
    assert_eq!(item["id"], json!(ITEM));
    assert_eq!(item["kind"], json!("pending"));
    assert_eq!(item["severity"], json!("critical"));
    assert_eq!(item["validity"], json!("current"));
    assert_eq!(item["options"], json!(["Stay", "Move"]));
    assert_eq!(item["cardEventId"], json!(card));
    assert_eq!(item["answer"], json!({ "option": 1, "answeredAt": b.now }));
    assert_eq!(item["recorded"], json!(1));
    // Only a live card is offered for answering.
    b.s.items.get_mut(ITEM).unwrap().hash = "ffff".to_string();
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let snap = serde_json::to_value(cards(&b.root)).unwrap();
    assert_eq!(snap["items"][0]["cardEventId"], Value::Null);
}
