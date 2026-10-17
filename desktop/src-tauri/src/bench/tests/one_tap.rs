//! Spec section 9: the owner's tap passed on as the owner's own message, the
//! card that comes back, the plain-language layout and its action line, the
//! thread link, and a busy thread's read. Child of tests.rs so the fake
//! relay and Bench harness are in scope.

use super::answers::{reply, tap, ITEM};
use super::channels::{
    admins, ask, bind, bound_bench, content, hex, notices, tags, thread_reply, ASK, X,
};
use super::owner_signed::carded;
use super::*;
use crate::bench::commands::cards;

const MERGE: &str =
    "gh pr merge 711 --repo QuicksilverSlick/Metabolic-reset --squash --delete-branch";

fn full_plain() -> Value {
    json!({
        "whatChanges": "Merges PR #711 into **main**",
        "askedBy": "Reset Worker",
        "risk": "low",
        "undo": "easy",
        "reach": "Every member",
        "ifYouWait": "The fix waits a day",
        "details": "The long story, folded on the desktop.",
    })
}

// ── the drop fields ─────────────────────────────────────────────────────

#[test]
fn plain_and_action_pass_the_drop_checks() {
    let rec = ("recommend", json!(1));
    let mut plain = full_plain();
    plain["confidence"] = json!("high");
    plain["why"] = json!("CI is green");
    let item = check(&[
        ("plain", plain.clone()),
        ("action", json!(MERGE)),
        rec.clone(),
    ])
    .unwrap();
    // Kept as written, member for member.
    assert_eq!(serde_json::to_value(&item.plain).unwrap(), plain);
    assert_eq!(item.action.as_deref(), Some(MERGE));
    // Each joins the hash when set.
    let bare = check(std::slice::from_ref(&rec)).unwrap().hash;
    let only_plain = check(&[("plain", json!({"risk": "low"})), rec.clone()]).unwrap();
    let only_action = check(&[("action", json!(MERGE)), rec.clone()]).unwrap();
    assert!(bare != only_plain.hash && bare != only_action.hash);
    assert_ne!(only_plain.hash, only_action.hash);
    // Caps are inclusive.
    let at_caps = json!({"whatChanges": "a".repeat(300), "details": "a".repeat(3000)});
    assert!(check(&[("plain", at_caps)]).is_ok());
    assert!(check(&[("action", json!("a".repeat(300))), rec.clone()]).is_ok());

    let plain = |v: Value| ("plain", v);
    let action = |a: &str| ("action", json!(a));
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("unknown member", vec![plain(json!({"headline": "Merge?"}))]),
        ("bad risk", vec![plain(json!({"risk": "severe"}))]),
        ("bad undo", vec![plain(json!({"undo": "never"}))]),
        (
            "bad confidence",
            vec![plain(json!({"confidence": "sure"})), rec.clone()],
        ),
        (
            "whatChanges past 300",
            vec![plain(json!({"whatChanges": "a".repeat(301)}))],
        ),
        (
            "details past 3000",
            vec![plain(json!({"details": "a".repeat(3001)}))],
        ),
        (
            "banned text",
            vec![plain(json!({"reach": "see nostr:npub1x"}))],
        ),
        (
            "format character",
            vec![plain(json!({"askedBy": "Craig\u{202E}"}))],
        ),
        ("why without recommend", vec![plain(json!({"why": "x"}))]),
        (
            "confidence without recommend",
            vec![plain(json!({"confidence": "high"}))],
        ),
        ("action without recommend", vec![action(MERGE)]),
        (
            "action newline",
            vec![action("gh pr merge 1\nrm x"), rec.clone()],
        ),
        ("action backtick", vec![action("gh `x`"), rec.clone()]),
        (
            "action link",
            vec![action("curl https://evil.example/x"), rec.clone()],
        ),
        ("action colon", vec![action("a: b"), rec.clone()]),
        ("action mention", vec![action("ping @x"), rec.clone()]),
        ("action channel", vec![action("see #general"), rec.clone()]),
        (
            "action past 300",
            vec![action(&"a".repeat(301)), rec.clone()],
        ),
        (
            "action trailing dash",
            vec![action("gh pr merge 1 -"), rec.clone()],
        ),
        ("action padded", vec![action(" gh pr merge 1"), rec.clone()]),
        ("action empty", vec![action(""), rec.clone()]),
    ];
    for (name, overrides) in cases {
        assert!(check(&overrides).is_err(), "{name} should be rejected");
    }
}

// ── the card text ───────────────────────────────────────────────────────

const HEAD: &str = "🔴 · Dreamforge · orchestrator\n**Relay choice**\n\nStay or move?\n";
const OPTIONS: &str = "\n1. Stay\n2. Move\n";

#[test]
fn plain_lines_follow_the_summary_in_order() {
    // Without the new fields: today's text, byte for byte.
    let today = format!("{HEAD}{OPTIONS}\nTap one number below to answer.\nas of 14:05");
    assert_eq!(render_card(&check(&[]).unwrap(), "14:05"), today);
    // Every line, words only; details stay off the phone.
    let item = check(&[("plain", full_plain())]).unwrap();
    assert_eq!(
        render_card(&item, "14:05"),
        format!(
            "{HEAD}\n\
             **What changes:** Merges PR 711 into main\n\
             \n\
             **Asked by:** Reset Worker\n\
             \n\
             **Risk:** Low · **Undo:** Easy · **Reach:** Every member\n\
             \n\
             **If you wait:** The fix waits a day\n\
             {OPTIONS}\nTap one number below to answer.\nas of 14:05"
        )
    );
    // Only what is set; a value that sanitizes to nothing has no line.
    let item = check(&[("plain", json!({"undo": "impossible", "askedBy": "**"}))]).unwrap();
    assert_eq!(
        render_card(&item, "14:05"),
        format!(
            "{HEAD}\n**Undo:** Impossible\n{OPTIONS}\nTap one number below to answer.\nas of 14:05"
        )
    );
    // The recommendation's confidence and reason, then what one tap approves.
    let item = check(&[
        ("recommend", json!(2)),
        (
            "plain",
            json!({"confidence": "high", "why": "Moving costs a day"}),
        ),
        ("action", json!(MERGE)),
    ])
    .unwrap();
    assert!(render_card(&item, "14:05").ends_with(&format!(
        "\n\n**Recommended:** 2 (orchestrator) (high confidence) - Moving costs a day\n\
         If you choose Move: {MERGE}\n{OPTIONS}\nFor: the owner. Tap one number below to answer.\n\
         as of 14:05\nOr reply in this thread."
    )));
}

fn previous(by: &str) -> Option<Previous> {
    Some(Previous {
        option: 1,
        label: "Move".to_string(),
        by: by.to_string(),
        by_name: None,
        answered_at: 1,
    })
}

#[test]
fn a_card_that_came_back_says_so_first() {
    let mut item = check(&[]).unwrap();
    item.previous = previous(&"aa".repeat(32));
    assert!(render_card(&item, "14:05").starts_with(&format!("Back to you\n{HEAD}")));
    assert!(item.back_to_you());
    // Not twice when the writer already says so.
    item.title = "**Back to you:** Relay choice".to_string();
    assert!(render_card(&item, "14:05").starts_with("🔴 · "));
    // Answered again, it waits no more.
    item.title = "Relay choice".to_string();
    item.answer = Some(Answer::default());
    assert!(!item.back_to_you());
    assert!(render_card(&item, "14:05").starts_with("🔴 · "));
}

#[test]
fn whoever_answered_last_is_pinged_once_while_they_may_answer() {
    let (owner, phone, craig) = ("ff".repeat(32), "cd".repeat(32), "c1".repeat(32));
    let set = [owner.clone(), phone.clone()];
    let mut item = check(&[]).unwrap();
    // The phone's own key answered: it is pinged with the owner.
    item.previous = previous(&phone);
    assert_eq!(
        card_pings(&item, &owner, &set),
        [owner.clone(), phone.clone()]
    );
    item.pinged = vec![phone.clone()];
    assert_eq!(
        card_pings(&item, &owner, &set),
        std::slice::from_ref(&owner)
    );
    // An answer from before step 1 was the owner's.
    item.previous = previous("");
    item.pinged.clear();
    assert_eq!(
        card_pings(&item, &owner, &set),
        std::slice::from_ref(&owner)
    );
    // Someone the roster no longer lets answer is not.
    item.previous = previous(&craig);
    assert_eq!(
        card_pings(&item, &owner, &set),
        std::slice::from_ref(&owner)
    );
    item.project = Some("p".to_string());
    item.stamp = Some(Stamp {
        answerers: vec![Answerer {
            pk: craig.clone(),
            name: "Craig".to_string(),
            role: AnswerRole::Decider,
            agent: false,
        }],
        ..Default::default()
    });
    assert_eq!(
        card_pings(&item, &owner, &set),
        std::slice::from_ref(&craig)
    );
}

#[test]
fn the_owners_notice_is_fixed_wording() {
    let mut item = carded(check(&[("recommend", json!(2)), ("action", json!(MERGE))]).unwrap());
    item.answer = Some(Answer {
        option: 1,
        ..Default::default()
    });
    let chose = |who: &str, label: &str| {
        format!(
            "{who} chose: {label} on card abababababab. \
             I approve the action that option describes."
        )
    };
    assert_eq!(
        render_owner_route(&item).unwrap(),
        format!("{}\nAction: {MERGE}", chose("The owner", "Move"))
    );
    // Not the recommended option: nothing more.
    item.answer.as_mut().unwrap().option = 0;
    assert_eq!(
        render_owner_route(&item).unwrap(),
        chose("The owner", "Stay")
    );
    // Named as roles.json names the owner, and the option as the card shows it.
    item.stamp = Some(Stamp {
        owner_name: "Quicksilver".to_string(),
        ..Default::default()
    });
    item.options[0] = "Stay - for now".to_string();
    assert_eq!(
        render_owner_route(&item).unwrap(),
        chose("Quicksilver", "Stay - for now")
    );
    // Anything the drop checks would refuse now, or that would not read
    // exactly as both apps showed it: the writer's notice instead.
    type Spoil = fn(&mut Item);
    let bad: [(&str, Spoil); 6] = [
        ("option", |i| i.options[0] = "nsec1abc".to_string()),
        ("empty option", |i| i.options[0] = "***".to_string()),
        ("action", |i| i.action = Some("rm `x`".to_string())),
        ("em dash", |i| {
            i.options[0] = "Stay \u{2014} for now".to_string()
        }),
        ("Greek", |i| i.options[0] = "St\u{3b1}y".to_string()),
        ("spaced", |i| i.options[0] = "Stay  for now".to_string()),
    ];
    for (name, spoil) in bad {
        let mut spoiled = item.clone();
        spoil(&mut spoiled);
        assert_eq!(render_owner_route(&spoiled), None, "{name}");
    }
    // No live card to name: the writer's notice.
    item.card = None;
    assert_eq!(render_owner_route(&item), None);
}

#[test]
fn the_thread_line_passes_the_link_rules() {
    let id = "ab".repeat(32);
    assert_eq!(
        thread_line(X, &id),
        format!("\nOpen thread: buzz://message?channel={X}&id={id}")
    );
    assert_eq!(thread_line("not-a-channel", &id), "");
    assert_eq!(thread_line(X, "ab"), "");
}

// ── through the fake relay ──────────────────────────────────────────────

/// A card in X that merges on its recommended option.
fn merge_card(b: &Bench) {
    let drop = ask(&[
        ("title", json!("Merge PR 711")),
        ("options", json!(["Merge PR", "Hold"])),
        ("recommend", json!(1)),
        ("action", json!(MERGE)),
    ]);
    b.drop_file("reset-worker", "merge", &drop);
}

#[tokio::test]
async fn the_owners_tap_goes_out_as_the_owners_own_message() {
    let (mut b, p, list) = bound_bench().await;
    merge_card(&b);
    b.settle(6).await;
    let id = "reset-worker/merge";
    let card = b.card(id).event_id;
    let posts = b.posts();
    let posted = &posts
        .iter()
        .find(|(e, _)| e["id"] == card.as_str())
        .unwrap()
        .0;
    assert!(
        content(posted).contains(&format!(
            "\n\n**Recommended:** 1 (Reset Worker)\nIf you choose Merge PR: {MERGE}\n\n1. Merge PR\n"
        )),
        "{}",
        content(posted)
    );
    let merge = tap(&b.ctx.owner, &card, OPTION_EMOJI[0], b.now);
    reply(&b, &[&list, &merge]);
    b.tick().await.unwrap();
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.settle(2).await;
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0]["pubkey"], json!(b.owner_hex()));
    assert_eq!(
        content(&n[0]),
        format!(
            "Quicksilver chose: Merge PR on card {}. \
             I approve the action that option describes.\nAction: {MERGE}",
            &card[..12]
        )
    );
    assert_eq!(tags(&n[0], "p"), [json!(["p", hex(&p.agent)])]);
    assert_eq!(tags(&n[0], "e"), [json!(["e", card, "", "reply"])]);
    assert_eq!(tag_value(&n[0], "h").as_deref(), Some(X));

    // The owner's key in the card's thread, yet never the owner's Other.
    let notice: nostr::Event = serde_json::from_value(n[0].clone()).unwrap();
    reply(&b, &[&list, &merge, &notice]);
    b.settle(2).await;
    let item = &b.s.items[id];
    assert!(item.reply.is_none());
    assert_eq!(item.answer.as_ref().unwrap().reaction_id, merge.id.to_hex());

    // Hold, which is not the recommended option, approves no command, and
    // takes back the merge it replaces.
    let hold = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now);
    reply(&b, &[&list, &notice, &hold]);
    b.tick().await.unwrap();
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.settle(2).await;
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0]["pubkey"], json!(b.owner_hex()));
    assert_eq!(
        content(&n[0]),
        format!(
            "Quicksilver chose: Hold on card {}. I approve the action that option describes.\n\
             This replaces the earlier answer: Merge PR. Do not act on that one; undo what was \
             done for it if it conflicts.",
            &card[..12]
        )
    );
}

#[tokio::test]
async fn text_that_fails_the_checks_goes_out_as_the_writers() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let agent = Keys::generate();
    b.ctx.agents = vec![hex(&agent)];
    let routes = json!({ "orchestrator": hex(&agent) });
    std::fs::write(b.root.join("routes.json"), routes.to_string()).unwrap();
    // Changed in state after the drop checks passed it.
    b.s.items.get_mut(ITEM).unwrap().options[1] = "Move nostr:npub1x".to_string();
    let mv = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&mv]);
    b.tick().await.unwrap();
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.settle(3).await;
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0]["pubkey"], json!(b.writer_hex()));
    assert!(content(&n[0]).starts_with("Bench answer · orchestrator/relay-choice\n"));
}

#[tokio::test]
async fn an_answered_card_asked_again_comes_back_to_whoever_answered() {
    let (mut b, p, list) = bound_bench().await;
    let old = b.card(ASK).event_id;
    let hold = tap(&b.ctx.owner, &old, OPTION_EMOJI[1], b.now);
    reply(&b, &[&list, &hold]);
    b.tick().await.unwrap();
    let answered_at = b.s.items[ASK].answer.as_ref().unwrap().answered_at;
    let again = ask(&[("body", json!("Craig wants it shorter still."))]);
    b.drop_file("reset-worker", "welcome", &again);
    let before = b.posts().len();
    b.settle(3).await;
    let new = b.card(ASK).event_id;
    assert_ne!(new, old);
    // The answered card stays as the receipt of that answer: never deleted,
    // never adopted again.
    assert!(!b.posts()[before..]
        .iter()
        .any(|(e, _)| e["kind"] == 5 && tag_value(e, "e").as_deref() == Some(old.as_str())));
    assert!(b.s.kept.contains(&old));
    // A new card in the same place, saying so first, pinging Craig as an
    // answerer and the owner, who answered last time. It has no id of its
    // own yet, so no thread link.
    let posts = b.posts();
    let posted = &posts
        .iter()
        .find(|(e, _)| e["id"] == new.as_str())
        .unwrap()
        .0;
    assert_eq!(tag_value(posted, "h").as_deref(), Some(X));
    assert!(
        content(posted)
            .starts_with("Back to you\n🟠 · 28 Day Reset · reset-worker\n**Welcome email**\n"),
        "{}",
        content(posted)
    );
    assert!(!content(posted).contains("Open thread"));
    assert_eq!(
        tags(posted, "p"),
        [json!(["p", hex(&p.craig)]), json!(["p", b.owner_hex()])]
    );
    let item = &b.s.items[ASK];
    assert_eq!(
        item.previous,
        Some(Previous {
            option: 1,
            label: "Keep the old one".to_string(),
            by: b.owner_hex(),
            by_name: None,
            answered_at,
        })
    );
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let snap = serde_json::to_value(cards(&b.root)).unwrap();
    let card = |snap: &Value| -> Value {
        let items = snap["items"].as_array().unwrap();
        items.iter().find(|i| i["id"] == ASK).unwrap().clone()
    };
    assert_eq!(card(&snap)["backToYou"], json!(true));
    assert_eq!(
        card(&snap)["previousAnswer"],
        json!({"option": 1, "label": "Keep the old one", "byName": null, "answeredAt": answered_at})
    );

    // Answered again: waiting no more, and the card says so in place.
    let send = tap(&p.craig, &new, OPTION_EMOJI[0], b.now);
    reply(&b, &[&list, &hold, &send]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert!(!b.s.items[ASK].back_to_you());
    let edit = &b.posts()[before].0;
    assert_eq!(tag_value(edit, "e").as_deref(), Some(new.as_str()));
    assert!(
        content(edit).starts_with("🟠 · ") && content(edit).contains("Recorded: 1. Send"),
        "{}",
        content(edit)
    );
    save_state(&b.root, &b.s, &mut String::new()).unwrap();
    let snap = serde_json::to_value(cards(&b.root)).unwrap();
    assert_eq!(card(&snap)["backToYou"], json!(false));
}

#[tokio::test]
async fn an_unanswered_card_asked_again_is_replaced_as_today() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let redrop = |b: &Bench, title: &str, options: Value| {
        let drop = drop_json(&[("title", json!(title)), ("options", options)]);
        b.drop_file("orchestrator", "relay-choice", &drop);
    };
    let replaced = |b: &Bench, before: usize, old: &str| {
        assert_eq!(b.kinds()[before..], [5, 9, 7, 7, 9, 5]);
        assert_eq!(tag_value(&b.posts()[before].0, "e").as_deref(), Some(old));
        content(&b.posts()[before + 1].0).to_string()
    };
    let old = b.card(ITEM).event_id;
    let before = b.posts().len();
    redrop(&b, "Relay choice", json!(["Stay", "Move away"]));
    b.settle(3).await;
    assert!(replaced(&b, before, &old).starts_with("🔴 · "));
    assert!(b.s.items[ITEM].previous.is_none());

    // Answered, then asked again: it comes back.
    let mv = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&mv]);
    b.tick().await.unwrap();
    redrop(&b, "Relay choice, again", json!(["Stay", "Move away"]));
    b.settle(3).await;
    assert_eq!(
        b.s.items[ITEM].previous.as_ref().unwrap().label,
        "Move away"
    );
    // Reworded before anyone answers, it is replaced as today and still
    // comes back from that answer.
    let back = b.card(ITEM).event_id;
    let before = b.posts().len();
    redrop(&b, "Relay choice, again", json!(["Stay", "Move now"]));
    b.settle(3).await;
    assert!(replaced(&b, before, &back).starts_with("Back to you\n🔴 · "));
    assert_eq!(
        b.s.items[ITEM].previous.as_ref().unwrap().label,
        "Move away"
    );
}

#[tokio::test]
async fn a_busy_thread_never_stalls_the_tick() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    bind(&mut b);
    let list = admins(&b, &b.relay.keys.clone(), &[]);
    reply(&b, &[&list]);
    let root = "ab".repeat(32);
    b.drop_file("reset-worker", "welcome", &ask(&[("replyTo", json!(root))]));
    b.settle(6).await;
    let card = b.card(ASK);
    // The owner answers by naming only the thread's root, as Block's phone does.
    let two = thread_reply(&b.ctx.owner, &root, "2", card.created_at + 1, &[]);
    reply(&b, &[&list, &two]);
    b.tick().await.unwrap();
    let answered = |b: &Bench| {
        b.s.items[ASK]
            .answer
            .as_ref()
            .map(|a| a.reaction_id.clone())
    };
    assert_eq!(answered(&b), Some(two.id.to_hex()));
    // Then the thread fills the owner's page, which cuts that answer out of
    // it; meanwhile the owner taps the #bench card and a drop arrives.
    let chat: Vec<nostr::Event> = (0..RECOVER_LIMIT as u64)
        .map(|i| thread_reply(&b.ctx.owner, &root, "chat", card.created_at + 2 + i, &[]))
        .collect();
    let stay = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[0], b.now);
    let mut evs = vec![&list, &two, &stay];
    evs.extend(&chat);
    reply(&b, &evs);
    b.drop_file(
        "orchestrator",
        "later",
        &drop_json(&[("title", json!("Later"))]),
    );
    let err = b.tick().await.unwrap_err();
    assert!(err.contains("those cards' answers stand"), "{err}");
    // The threaded card keeps its answer, and nothing in the cut page is
    // its Other; the #bench card's tap counts; the drop is taken.
    assert_eq!(answered(&b), Some(two.id.to_hex()));
    assert!(b.s.items[ASK].reply.is_none());
    assert_eq!(b.s.items[ITEM].answer.as_ref().map(|a| a.option), Some(0));
    assert!(b.s.items.contains_key("orchestrator/later"));
    for _ in 0..4 {
        let _ = b.tick().await;
    }
    assert!(b.s.items["orchestrator/later"].card.is_some());
    // The thread is read from an hour before its card, apart from the cards.
    let q = b.relay.queries.lock().unwrap().clone();
    let thread = q
        .iter()
        .rev()
        .find(|q| q[0]["#e"] == json!([root]))
        .unwrap();
    assert_eq!(thread[0]["since"], json!(card.created_at - 3600));
}
