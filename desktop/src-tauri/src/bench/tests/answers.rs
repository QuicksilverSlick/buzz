//! Bench M2: answers by reaction. Child of tests.rs so the fake relay, Bench
//! harness and drop helpers are in scope.

use super::*;

fn tap(keys: &Keys, card_hex: &str, emoji: &str, created_at: u64) -> nostr::Event {
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
    })
}

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
    assert_eq!(
        card,
        format!("{plain}\nRecorded: 2. Move x(https:evil.example) · 14:05")
    );
    let tail = card.rsplit_once("\n```").unwrap().1;
    assert!(
        !tail.contains("https://") && !tail.contains('['),
        "{tail:?}"
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
        "Bench · as of 14:05 · 3 need you\n\
         Needs you\n\
         1. 🔴 Relay choice (dreamforge) (no card yet)\n\
         2. 🔴 Relay choice (dreamforge) (no card yet)\n\
         3. 🔴 Earlier tap (dreamforge) (no card yet)"
    );
    done.answer = answered(1, 100);
    first.answer = answered(0, 50);
    let all = [&open, &done, &first];
    let board = render_board(&all, UUID, "14:05", 100);
    assert_eq!(
        board,
        "Bench · as of 14:05 · 1 need you · 2 answered\n\
         Needs you\n\
         1. 🔴 Relay choice (dreamforge) (no card yet)\n\
         Answered, waiting for action (2)\n\
         - 🔴 Earlier tap (dreamforge) → 1. Stay\n\
         - 🔴 Relay choice (dreamforge) → 2. Move x(https:evil.example)"
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
        late.ends_with("→ 2. Move x(https:evil.example) 🟠 24h"),
        "{late:?}"
    );
    assert_ne!(board_hash(&board), board_hash(&late));
}

#[test]
fn m1_state_loads() {
    let mut item = check(&[]).unwrap();
    let mut v = serde_json::to_value(&item).unwrap();
    let o = v.as_object_mut().unwrap();
    assert!(o.remove("answer").is_some() && o.remove("recorded").is_some());
    let loaded: Item = serde_json::from_value(v).unwrap();
    assert!(loaded.answer.is_none() && loaded.recorded.is_none());
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
