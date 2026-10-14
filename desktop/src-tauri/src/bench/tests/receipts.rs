//! Receipts: a close-out keeps the card and edits it to add the answer and
//! the outcome. Child of tests.rs so the fake relay and Bench harness are in
//! scope.

use super::answers::{last_query, reply, tap, ITEM};
use super::*;

fn close_out(b: &Bench, title: &str) {
    let drop = drop_json(&[
        ("kind", json!("decision")),
        ("decidedBy", json!("claude")),
        ("severity", Value::Null),
        ("options", Value::Null),
        ("title", json!(title)),
        ("body", json!("Moved the community to the new relay.")),
        (
            "links",
            json!([{"label": "PR", "url": "https://github.com/x/y/pull/1"}]),
        ),
    ]);
    b.drop_file("orchestrator", "relay-choice", &drop);
}

fn content(ev: &Value) -> &str {
    ev["content"].as_str().unwrap()
}

#[tokio::test]
async fn close_out_edits_the_card_into_a_receipt() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    // A second card keeps the poll running once the first is closed.
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
    let before = b.posts().len();
    close_out(&b, "Moved the relay");
    b.tick().await.unwrap();
    // The card's edit, then the board's: nothing is deleted or reposted.
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [40003, 40003]);
    assert_eq!(tag_value(&posts[0].0, "e").unwrap(), card.event_id);
    assert_eq!(tag_value(&posts[0].0, "h").unwrap(), b.channel());
    let item = &b.s.items[ITEM];
    let receipt = content(&posts[0].0);
    assert_eq!(receipt, render_card(item, "14:05"));
    let request = item.request.as_ref().unwrap();
    let at = &request.answer.as_ref().unwrap().hhmm;
    assert!(
        receipt.starts_with("🔴 · dreamforge · orchestrator\n**Relay choice**\n"),
        "{receipt}"
    );
    assert!(
        receipt.ends_with(&format!(
            "1. Stay\n2. Move\n\nAnswered: 2. Move · {at}\n\nOutcome · 14:05\n\
             **Moved the relay**\n\nMoved the community to the new relay.\n\n\
             PR: https://github.com/x/y/pull/1"
        )),
        "{receipt}"
    );
    assert_eq!(item.card.as_ref().unwrap().event_id, card.event_id);
    assert_eq!(item.card.as_ref().unwrap().hash, card_hash(item));
    // The board links the decision to its receipt.
    let board = content(&posts[1].0);
    let link = format!(
        "buzz://message?channel={}&id={}",
        b.channel(),
        card.event_id
    );
    assert!(
        board.contains(&format!(
            "- 🧭 Moved the relay (dreamforge) (claude) · {link}"
        )),
        "{board}"
    );
    // The poll reads the open card only, and nothing more is posted.
    let before = b.posts().len();
    b.settle(2).await;
    assert_eq!(last_query(&b)[0]["#e"], json!([other]));
    assert_eq!(b.posts().len(), before);
    // A relaunch keeps the receipt: no delete, no repost.
    let relay: Vec<nostr::Event> = b
        .posts()
        .into_iter()
        .map(|(e, _)| serde_json::from_value::<nostr::Event>(e).unwrap())
        .filter(|e| e.kind.as_u16() == 9)
        .collect();
    reply(&b, &relay.iter().collect::<Vec<_>>());
    b.s.needs_rebuild = true;
    let before = b.posts().len();
    b.settle(3).await;
    assert!(!b.kinds()[before..].iter().any(|k| matches!(k, 5 | 9)));
    assert_eq!(b.card(ITEM).event_id, card.event_id);
}

#[tokio::test]
async fn unanswered_close_out_is_kept_and_a_second_one_re_edits() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    let before = b.posts().len();
    close_out(&b, "Not needed after all");
    b.tick().await.unwrap();
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [40003, 40003]);
    let receipt = content(&posts[0].0);
    assert!(
        receipt.contains(
            "\nAnswered: no answer recorded\n\nOutcome · 14:05\n**Not needed after all**"
        ),
        "{receipt}"
    );
    // Later ticks neither retire nor re-edit it.
    let before = b.posts().len();
    b.settle(3).await;
    assert_eq!(b.posts().len(), before);
    // A second close-out edits the same card, under the same question.
    close_out(&b, "Moved after all");
    b.tick().await.unwrap();
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [40003, 40003]);
    assert_eq!(tag_value(&posts[0].0, "e").unwrap(), card);
    let receipt = content(&posts[0].0);
    assert!(
        receipt.contains("**Relay choice**") && receipt.contains("**Moved after all**"),
        "{receipt}"
    );
    assert!(!receipt.contains("Not needed"), "{receipt}");
    assert_eq!(
        b.s.items[ITEM].request.as_ref().unwrap().title,
        "Relay choice"
    );
}

#[tokio::test]
async fn receipts_do_not_count_against_max_cards() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    close_out(&b, "Moved the relay");
    b.tick().await.unwrap();
    for i in 0..MAX_CARDS {
        b.drop_file("orchestrator", &format!("c{i:02}"), &drop_json(&[]));
    }
    let receipt = b.card(ITEM).event_id;
    b.settle(40).await;
    let open =
        b.s.items
            .values()
            .filter(|i| i.kind == ItemKind::Pending && i.card.is_some())
            .count();
    assert_eq!(open, MAX_CARDS);
    assert!(b.s.items["orchestrator/c29"].card.is_some());
    assert_eq!(b.card(ITEM).event_id, receipt);
}
