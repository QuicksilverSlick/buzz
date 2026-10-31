//! The MAX_CARDS cap: it gates new posts only, queues the rest in rank order
//! behind an alarm, frees an answered card's slot after ANSWER_SLOT_SECS,
//! and posts a card the owner bumped next. Child of tests.rs.

use super::answers::{reply, tap, ITEM};
use super::*;
use crate::bench::commands::{bump, cards};

/// The board as the relay last got it: its post, or a later edit of it.
fn board(b: &Bench) -> String {
    let id = json!(b.s.board.as_ref().unwrap().event_id);
    b.posts()
        .iter()
        .rev()
        .find(|(e, _)| {
            e["id"] == id
                || (e["kind"] == 40003 && tag_value(e, "e") == id.as_str().map(String::from))
        })
        .map(|(e, _)| e["content"].as_str().unwrap().to_string())
        .unwrap()
}

/// Whether a kind 5 since `from` took down `first` or a card posted since.
fn cards_deleted(b: &Bench, from: usize, first: &str) -> bool {
    let posts = b.posts()[from..].to_vec();
    let cards: Vec<String> = posts
        .iter()
        .filter(|(e, _)| tag_value(e, "client").is_some_and(|c| c.starts_with("bench:card:")))
        .map(|(e, _)| e["id"].as_str().unwrap().to_string())
        .chain([first.to_string()])
        .collect();
    posts
        .iter()
        .any(|(e, _)| e["kind"] == 5 && tag_value(e, "e").is_some_and(|t| cards.contains(&t)))
}

fn live(b: &Bench) -> usize {
    b.s.items
        .values()
        .filter(|i| i.kind == ItemKind::Pending && i.card.is_some())
        .count()
}

fn queued(b: &Bench) -> Vec<&str> {
    b.s.queued
        .iter()
        .map(|q| &q["orchestrator/".len()..])
        .collect()
}

#[test]
fn rank_reads_digit_runs_as_numbers() {
    let item = |id: &str, severity: &str| {
        validate(
            "steward",
            id,
            &drop_json(&[("severity", json!(severity))]),
            NOW,
        )
        .unwrap()
    };
    let mut items = [
        item("hold-talos-10-ab12", "attention"),
        item("hold-talos-7-ff00", "attention"),
        item("hold-talos-9-0a", "attention"),
        item("hold-buzz-99-1", "attention"),
        item("hold-talos-17-0", "critical"),
    ];
    items.sort_by_key(rank);
    let order: Vec<&str> = items
        .iter()
        .map(|i| &i.id["steward/hold-".len()..])
        .collect();
    assert_eq!(
        order,
        [
            "talos-17-0",
            "buzz-99-1",
            "talos-7-ff00",
            "talos-9-0a",
            "talos-10-ab12"
        ]
    );
}

#[tokio::test]
async fn posted_cards_keep_their_place_and_the_rest_queue_in_order() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let first = b.card(ITEM).event_id;
    let before = b.posts().len();
    // A stack of holds, as the steward drops them: 32 for 29 free slots.
    let hold = drop_json(&[("severity", json!("attention"))]);
    for n in 7..=38 {
        b.drop_file("orchestrator", &format!("hold-{n}"), &hold);
    }
    b.settle(30).await;
    assert_eq!(b.card(ITEM).event_id, first);
    assert_eq!(live(&b), MAX_CARDS);
    // Cut from the top of the stack, never from its bottom (#7, #8, #9).
    assert_eq!(queued(&b), ["hold-36", "hold-37", "hold-38"]);
    assert!(b.s.items["orchestrator/hold-7"].card.is_some());
    let text = board(&b);
    let alarm = "\n\nQueued for a slot (3)\n3 open cards show (no card yet): \
                 the Bench keeps 30 live at once, and each posts as a slot frees.";
    assert!(text.contains(alarm), "{text}");
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let snap = serde_json::to_value(cards(&b.root)).unwrap();
    assert_eq!(snap["maxCards"], json!(MAX_CARDS));
    let flagged: Vec<&Value> = snap["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["queued"] == json!(true))
        .map(|i| &i["id"])
        .collect();
    let want = [
        "orchestrator/hold-36",
        "orchestrator/hold-37",
        "orchestrator/hold-38",
    ];
    assert_eq!(flagged, want);

    // A critical newcomer goes to the front of the line; nothing posted is
    // taken down to make room for it.
    b.drop_file("orchestrator", "urgent", &drop_json(&[]));
    b.settle(2).await;
    assert_eq!(queued(&b), ["urgent", "hold-36", "hold-37", "hold-38"]);
    assert!(!cards_deleted(&b, before, &first));

    // The owner taps the last one on the desktop: it posts next, over the cap.
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let err = bump(&b.root, "orchestrator/nope", b.now).unwrap_err();
    assert!(err.contains("no Bench item"), "{err}");
    bump(&b.root, ITEM, b.now).unwrap();
    assert!(
        !b.root.join("bump.json").exists(),
        "a live card needs no bump"
    );
    bump(&b.root, "orchestrator/hold-38", b.now).unwrap();
    b.tick().await.unwrap();
    assert!(b.s.items["orchestrator/hold-38"].card.is_some());
    assert_eq!(queued(&b), ["urgent", "hold-36", "hold-37"]);
    assert_eq!(live(&b), MAX_CARDS + 1);
    // A bump lasts BUMP_SECS; after that it orders nothing.
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    bump(&b.root, "orchestrator/hold-37", b.now).unwrap();
    b.now += publish::BUMP_SECS;
    b.tick().await.unwrap();
    assert_eq!(queued(&b), ["urgent", "hold-36", "hold-37"]);
    assert!(!cards_deleted(&b, before, &first));
}

#[tokio::test]
async fn an_answered_card_frees_its_slot_after_answer_slot_secs() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let owner_tap = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    for i in 0..MAX_CARDS {
        b.drop_file("orchestrator", &format!("c{i:02}"), &drop_json(&[]));
    }
    b.settle(30).await;
    // The answered card holds one slot while a late tap may still change it.
    assert_eq!(live(&b), MAX_CARDS);
    assert_eq!(queued(&b), ["c29"]);
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let err = bump(&b.root, ITEM, b.now).unwrap_err();
    assert!(err.contains("no longer an open card"), "{err}");
    // Past ANSWER_SLOT_SECS it stays on the relay, still read, slot given back.
    let card = b.card(ITEM).event_id;
    let before = b.posts().len();
    b.now += ANSWER_SLOT_SECS;
    b.settle(4).await;
    assert_eq!(b.card(ITEM).event_id, card);
    assert!(b.s.items[ITEM].answer.is_some());
    assert!(b.s.items["orchestrator/c29"].card.is_some());
    assert!(b.s.queued.is_empty());
    assert!(!cards_deleted(&b, before, &card));
    assert!(!board(&b).contains("Queued for a slot"));
}
