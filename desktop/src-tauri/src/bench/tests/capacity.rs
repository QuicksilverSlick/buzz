//! The MAX_CARDS cap: it gates new posts only, keeps a refreshed card's
//! place, lets critical cards, re-asks and the owner's desktop taps past it
//! up to OVER_CAP, queues the rest in rank order behind an alarm, and frees
//! an answered card's slot after ANSWER_SLOT_SECS. Child of tests.rs.

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

/// ITEM (critical) and a stack of 32 attention holds, as the steward drops
/// them, for its 29 free slots: hold-7 .. hold-35 post, hold-36 .. -38 queue.
async fn full_bench() -> Bench {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let hold = drop_json(&[("severity", json!("attention"))]);
    for n in 7..=38 {
        b.drop_file("orchestrator", &format!("hold-{n}"), &hold);
    }
    b.settle(30).await;
    b
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
    let b = full_bench().await;
    assert_eq!(live(&b), MAX_CARDS);
    // Cut from the top of the stack, never from its bottom (#7, #8, #9).
    assert_eq!(queued(&b), ["hold-36", "hold-37", "hold-38"]);
    assert!(b.s.items["orchestrator/hold-7"].card.is_some());
    // The alarm, and each queued line says so.
    let text = board(&b);
    assert!(
        text.contains("\n\nCard slots: 30 of 30 in use, 3 queued\n"),
        "{text}"
    );
    assert_eq!(text.matches("(queued for a slot)").count(), 4, "{text}");
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let snap = serde_json::to_value(cards(&b.root)).unwrap();
    assert_eq!(snap["maxCards"], json!(MAX_CARDS));
    let flagged: Vec<&Value> = snap["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["waitsFor"] == json!("slot"))
        .map(|i| &i["id"])
        .collect();
    let want = [
        "orchestrator/hold-36",
        "orchestrator/hold-37",
        "orchestrator/hold-38",
    ];
    assert_eq!(flagged, want);
}

#[tokio::test]
async fn critical_cards_and_re_asks_post_past_the_cap_up_to_over_cap() {
    let mut b = full_bench().await;
    let first = b.card(ITEM).event_id;
    let before = b.posts().len();
    // A critical newcomer at a full Bench goes out at once; nothing posted
    // is taken down for it.
    b.drop_file("orchestrator", "urgent", &drop_json(&[]));
    b.settle(2).await;
    assert!(b.s.items["orchestrator/urgent"].card.is_some());
    assert_eq!(live(&b), MAX_CARDS + 1);
    assert_eq!(queued(&b), ["hold-36", "hold-37", "hold-38"]);
    assert!(board(&b).contains("Card slots: 31 of 30 in use, 3 queued"));
    // So does an attention card asked again after the owner answered it.
    let tap7 = tap(
        &b.ctx.owner,
        &b.card("orchestrator/hold-7").event_id,
        OPTION_EMOJI[0],
        b.now,
    );
    reply(&b, &[&tap7]);
    b.tick().await.unwrap();
    let again = drop_json(&[("severity", json!("attention")), ("title", json!("Again"))]);
    b.drop_file("orchestrator", "hold-7", &again);
    b.settle(2).await;
    assert!(b.s.items["orchestrator/hold-7"].back_to_you());
    assert!(b.s.items["orchestrator/hold-7"].card.is_some());
    assert!(!cards_deleted(&b, before, &first));
    // Up to MAX_CARDS + OVER_CAP; past it even a critical card queues, first.
    for n in 0..OVER_CAP {
        b.drop_file("orchestrator", &format!("c{n}"), &drop_json(&[]));
    }
    b.settle(16).await;
    assert_eq!(live(&b), MAX_CARDS + OVER_CAP);
    assert_eq!(queued(&b), ["c9", "hold-36", "hold-37", "hold-38"]);
    assert!(board(&b).contains("Card slots: 40 of 30 in use, 4 queued"));
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let err = bump(&b.root, "orchestrator/hold-36", b.now).unwrap_err();
    assert!(err.contains("the Bench's limit"), "{err}");
    assert!(!cards_deleted(&b, before, &first));
}

#[tokio::test]
async fn a_desktop_tap_posts_a_queued_card_next_for_bump_secs() {
    let mut b = full_bench().await;
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let err = bump(&b.root, "orchestrator/nope", b.now).unwrap_err();
    assert!(err.contains("no Bench item"), "{err}");
    // A posted card needs no bump.
    bump(&b.root, ITEM, b.now).unwrap();
    b.tick().await.unwrap();
    assert_eq!(live(&b), MAX_CARDS);
    // The owner taps the last one: it posts next, over the cap.
    bump(&b.root, "orchestrator/hold-38", b.now).unwrap();
    b.tick().await.unwrap();
    assert!(b.s.items["orchestrator/hold-38"].card.is_some());
    assert_eq!(queued(&b), ["hold-36", "hold-37"]);
    assert_eq!(live(&b), MAX_CARDS + 1);
    // A bump lasts BUMP_SECS; after that it orders nothing.
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    bump(&b.root, "orchestrator/hold-37", b.now).unwrap();
    b.now += publish::BUMP_SECS;
    b.tick().await.unwrap();
    assert_eq!(queued(&b), ["hold-36", "hold-37"]);
}

#[tokio::test]
async fn a_card_retired_for_a_refresh_keeps_its_place() {
    let mut b = full_bench().await;
    // A newcomer that ranks ahead of every hold, queued.
    let attention = drop_json(&[("severity", json!("attention"))]);
    b.drop_file("orchestrator", "a-first", &attention);
    b.settle(2).await;
    assert_eq!(queued(&b), ["a-first", "hold-36", "hold-37", "hold-38"]);
    // Four cards change format at once: (b) spends the tick retiring them,
    // and they come back before the newcomer, which still waits.
    let refreshed = ["hold-32", "hold-33", "hold-34", "hold-35"];
    for id in refreshed {
        let i = b.s.items.get_mut(&format!("orchestrator/{id}")).unwrap();
        let hash = i.hash.clone();
        i.card.as_mut().unwrap().hash = format!("{hash}.old-format");
    }
    b.tick().await.unwrap();
    assert!(b.s.items["orchestrator/hold-32"].card.is_none());
    b.settle(4).await;
    for id in refreshed {
        let i = &b.s.items[&format!("orchestrator/{id}")];
        assert!(i.card.is_some() && !i.repost, "{id}");
    }
    assert_eq!(queued(&b), ["a-first", "hold-36", "hold-37", "hold-38"]);
    assert_eq!(live(&b), MAX_CARDS);
}

#[tokio::test]
async fn an_answered_card_frees_its_slot_after_answer_slot_secs() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let owner_tap = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    let info = drop_json(&[("severity", json!("info"))]);
    for i in 0..MAX_CARDS {
        b.drop_file("orchestrator", &format!("c{i:02}"), &info);
    }
    b.settle(30).await;
    // The answered card holds one slot while the owner may change his mind.
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
    assert!(!board(&b).contains("Card slots"));
    // A tap after that still changes the answer.
    let changed = tap(&b.ctx.owner, &card, OPTION_EMOJI[0], b.now);
    reply(&b, &[&owner_tap, &changed]);
    b.tick().await.unwrap();
    assert_eq!(b.s.items[ITEM].answer.as_ref().map(|a| a.option), Some(0));
}
