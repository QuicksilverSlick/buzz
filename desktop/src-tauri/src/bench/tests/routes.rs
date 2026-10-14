//! Answer routing: routes.json, the settle, and the notice in the card's
//! thread. Child of tests.rs so the fake relay and Bench harness are in scope.

use super::answers::{last_query, reply, tap, ITEM};
use super::*;

const HEAD: &str = "Bench answer · orchestrator/relay-choice\nCard: Relay choice\n";

fn answered(option: usize, changed_at: u64) -> Item {
    let mut item = check(&[]).unwrap();
    item.answer = Some(Answer {
        reaction_id: "ab".repeat(32),
        option,
        answered_at: changed_at,
        ..Default::default()
    });
    item.changed_at = changed_at;
    item
}

fn told(option: Option<usize>) -> Option<Routed> {
    Some(Routed {
        option,
        to: "ee".repeat(32),
        event_id: String::new(),
        at: 0,
    })
}

#[test]
fn route_due_fires_once_per_settled_answer() {
    let s = ROUTE_SETTLE_SECS;
    let mut item = answered(1, 1000);
    // Settle gating: not before 120 s, then due.
    assert!(!route_due(&item, 1000 + s - 1));
    assert!(route_due(&item, 1000 + s));
    // Told: due no more, and a re-tap of the same option is no change.
    item.routed = told(Some(1));
    assert!(!route_due(&item, 9999));
    item.changed_at = 5000;
    assert!(!route_due(&item, 5000 + s));
    // A changed answer is due again once it settles.
    item.answer.as_mut().unwrap().option = 0;
    assert!(!route_due(&item, 5000 + s - 1) && route_due(&item, 5000 + s));
    // A clear after a notice is a withdrawal, once.
    item.answer = None;
    assert!(route_due(&item, 5000 + s));
    item.routed = told(None);
    assert!(!route_due(&item, 9999));
    // A clear that was never routed owes nothing.
    item.routed = None;
    assert!(!route_due(&item, 9999));
    // After a withdrawal a new answer is due again.
    item.routed = told(None);
    item.answer = answered(0, 5000).answer;
    assert!(route_due(&item, 5000 + s));
}

#[test]
fn render_route_text() {
    // Today's tap, as the design worked it: curly quotes fold to ASCII.
    let drop = drop_json(&[
        ("title", json!("Who should Honey and Pollen answer?")),
        (
            "options",
            json!(["Tell me here and I\u{2019}ll set it", "Reset all three"]),
        ),
    ]);
    let mut item = validate("claude-ai", "agent-roster", &drop, NOW).unwrap();
    item.answer = answered(0, 1).answer;
    assert_eq!(
        render_route(&item, None, "15:57"),
        "Bench answer · claude-ai/agent-roster\n\
         Card: Who should Honey and Pollen answer?\n\
         Chosen: 1. Tell me here and I'll set it\n\
         Answered 15:57 by the owner. Details: outbox/claude-ai/agent-roster.json in the Bench folder.\n\
         Act on it, reply in this thread, then clear the card by re-dropping claude-ai/agent-roster as a decision or status."
    );
    let mut item = answered(0, 1);
    item.title = "**Relay** [x](https://evil.example) @Honey".to_string();
    item.options[1] = "Move `now` #general".to_string();
    assert_eq!(
        render_route(&item, Some(1), "14:05"),
        "Bench answer changed · orchestrator/relay-choice\n\
         Card: Relay x (https: evil.example) Honey\n\
         Chosen: 1. Stay\n\
         Was: 2. Move now general\n\
         Answered 14:05 by the owner. Details: outbox/orchestrator/relay-choice.json in the Bench folder.\n\
         Undo what you did for the old choice if it conflicts, reply in this thread, then clear the card by re-dropping orchestrator/relay-choice as a decision or status."
    );
    item.answer = None;
    assert_eq!(
        render_route(&item, Some(1), "14:07"),
        "Bench answer withdrawn · orchestrator/relay-choice\n\
         Card: Relay x (https: evil.example) Honey\n\
         Was: 2. Move now general\n\
         The owner cleared the answer at 14:07. Stop if you have not acted; if you have, say in this thread what to undo. Do not clear the card."
    );
}

#[test]
fn admitting_agents_follows_the_projected_access() {
    let writer = "d1".repeat(32);
    let rec =
        |pubkey: &str, mode: &str, list: &[&str]| -> crate::managed_agents::ManagedAgentRecord {
            serde_json::from_value(json!({
                "pubkey": pubkey, "name": "A", "relay_url": "", "acp_command": "",
                "agent_command": "", "agent_args": [], "mcp_command": "",
                "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
                "updated_at": "", "last_started_at": null, "last_stopped_at": null,
                "last_exit_code": null, "last_error": null,
                "respond_to": mode, "respond_to_allowlist": list,
            }))
            .unwrap()
        };
    let upper = format!(" {} ", writer.to_ascii_uppercase());
    let records = [
        rec(&"A1".repeat(32), "anyone", &[]),
        rec(&"a2".repeat(32), "allowlist", &[&"cc".repeat(32), &upper]),
        rec(&"a3".repeat(32), "allowlist", &[&"cc".repeat(32)]),
        rec(&"a4".repeat(32), "owner-only", &[&writer]),
    ];
    assert_eq!(
        admitting_agents(&records, &writer, false),
        ["a1".repeat(32), "a2".repeat(32)]
    );
    // A build that clamps every agent to owner-only routes nowhere.
    assert!(admitting_agents(&records, &writer, true).is_empty());
}

/// One agent that admits the writer, routed for the orchestrator folder.
fn route(b: &mut Bench) -> Keys {
    let agent = Keys::generate();
    let hex = agent.public_key().to_hex();
    b.ctx.agents = vec![hex.clone()];
    let routes = json!({ "orchestrator": hex.to_ascii_uppercase() });
    std::fs::write(b.root.join("routes.json"), routes.to_string()).unwrap();
    agent
}

/// The answer notices posted since `before`.
fn notices(b: &Bench, before: usize) -> Vec<Value> {
    b.posts()[before..]
        .iter()
        .filter(|(e, _)| tag_value(e, "client").is_some_and(|c| c.starts_with("bench:route:")))
        .map(|(e, _)| e.clone())
        .collect()
}

fn content(ev: &Value) -> &str {
    ev["content"].as_str().unwrap()
}

/// Let `ticks` ticks run with the clock past the settle, returning the
/// notices they posted.
async fn settle_past(b: &mut Bench, ticks: usize) -> Vec<Value> {
    let before = b.posts().len();
    b.now += ROUTE_SETTLE_SECS;
    b.settle(ticks).await;
    notices(b, before)
}

#[tokio::test]
async fn routed_answer_lifecycle() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let agent = route(&mut b);
    let agent_hex = agent.public_key().to_hex();
    let card = b.card(ITEM).event_id;
    let move_tap = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now);
    // The agent's own keycap, newer: never an answer.
    let agent_tap = tap(&agent, &card, OPTION_EMOJI[0], b.now + 1);
    reply(&b, &[&move_tap, &agent_tap]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    // The agent joins #bench by the owner's 9000; the tap is recorded, not routed.
    let posts = b.posts()[before..].to_vec();
    assert_eq!(b.kinds()[before..], [9000, 40003, 9, 5]);
    assert_eq!(posts[0].0["pubkey"], json!(b.owner_hex()));
    assert_eq!(tag_value(&posts[0].0, "p"), Some(agent_hex.clone()));
    assert_eq!(b.s.items[ITEM].answer.as_ref().map(|a| a.option), Some(1));
    assert!(!last_query(&b)[0]["authors"]
        .as_array()
        .unwrap()
        .contains(&json!(agent_hex)));
    assert!(!std::fs::read_to_string(b.root.join("members.txt"))
        .unwrap()
        .contains(&agent_hex));
    // Not before the settle, then exactly one notice, in the card's thread.
    b.now += ROUTE_SETTLE_SECS - 4;
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert!(notices(&b, before).is_empty());
    b.tick().await.unwrap();
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    // A thread reply leaves the board newest: nothing else was posted.
    assert_eq!(b.kinds()[before..], [9]);
    let ev = &n[0];
    assert_eq!(ev["pubkey"], json!(b.writer_hex()));
    assert_eq!(tag_value(ev, "h"), Some(b.channel()));
    assert!(ev["tags"]
        .as_array()
        .unwrap()
        .contains(&json!(["e", card, "", "reply"])));
    let ps: Vec<&Value> = ev["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t[0] == "p")
        .collect();
    assert_eq!(ps, [&json!(["p", agent_hex])], "the owner is never pinged");
    assert_eq!(
        tag_value(ev, "client").unwrap(),
        "bench:route:orchestrator/relay-choice@2"
    );
    assert!(
        content(ev).starts_with(&format!("{HEAD}Chosen: 2. Move\nAnswered ")),
        "{}",
        content(ev)
    );
    assert_eq!(
        b.s.items[ITEM].routed,
        Some(Routed {
            option: Some(1),
            to: agent_hex.clone(),
            event_id: ev["id"].as_str().unwrap().to_string(),
            at: b.now,
        })
    );
    assert!(settle_past(&mut b, 2).await.is_empty());

    // Re-tapping the same option (untap, tap again) sends nothing.
    reply(&b, &[]);
    b.tick().await.unwrap();
    let again = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now + 1);
    reply(&b, &[&again]);
    b.tick().await.unwrap();
    assert!(settle_past(&mut b, 2).await.is_empty());

    // A change, once settled, says what it was.
    reply(&b, &[]);
    b.tick().await.unwrap();
    let stay = tap(&b.ctx.owner, &card, OPTION_EMOJI[0], b.now + 1);
    reply(&b, &[&stay]);
    let before = b.posts().len();
    b.settle(2).await;
    assert!(notices(&b, before).is_empty());
    let n = settle_past(&mut b, 2).await;
    assert_eq!(n.len(), 1);
    assert_eq!(
        tag_value(&n[0], "client").unwrap(),
        "bench:route:orchestrator/relay-choice@1"
    );
    assert!(content(&n[0]).starts_with(
        "Bench answer changed · orchestrator/relay-choice\nCard: Relay choice\n\
         Chosen: 1. Stay\nWas: 2. Move\n"
    ));

    // A clear after a notice is a withdrawal, sent once, after the settle.
    reply(&b, &[]);
    let before = b.posts().len();
    b.settle(2).await;
    assert!(notices(&b, before).is_empty());
    let n = settle_past(&mut b, 3).await;
    assert_eq!(n.len(), 1);
    assert_eq!(
        tag_value(&n[0], "client").unwrap(),
        "bench:route:orchestrator/relay-choice@withdrawn"
    );
    assert!(content(&n[0]).starts_with(
        "Bench answer withdrawn · orchestrator/relay-choice\nCard: Relay choice\nWas: 1. Stay\n"
    ));
    assert!(content(&n[0]).ends_with("Do not clear the card."));

    // Answered again, then acked by a re-drop: nothing more is ever sent.
    let last = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now + 1);
    reply(&b, &[&last]);
    b.tick().await.unwrap();
    let n = settle_past(&mut b, 1).await;
    assert!(content(&n[0]).starts_with(&format!("{HEAD}Chosen: 2. Move\n")));
    b.drop_file(
        "orchestrator",
        "relay-choice",
        &drop_json(&[
            ("kind", json!("decision")),
            ("decidedBy", json!("claude")),
            ("severity", Value::Null),
            ("options", Value::Null),
        ]),
    );
    b.tick().await.unwrap();
    let item = &b.s.items[ITEM];
    assert!(item.answer.is_none() && item.routed.is_none());
    assert!(settle_past(&mut b, 4).await.is_empty());
}

#[tokio::test]
async fn quick_changes_coalesce_into_one_notice() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    route(&mut b);
    let card = b.card(ITEM).event_id;
    // 1, clear, 2 inside the settle: one notice, for 2, as a new answer.
    let stay = tap(&b.ctx.owner, &card, OPTION_EMOJI[0], b.now);
    reply(&b, &[&stay]);
    b.tick().await.unwrap();
    reply(&b, &[]);
    b.tick().await.unwrap();
    let mv = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now + 1);
    reply(&b, &[&mv]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    b.now += ROUTE_SETTLE_SECS;
    b.settle(3).await;
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert!(content(&n[0]).starts_with(&format!("{HEAD}Chosen: 2. Move\n")));
    // A tap withdrawn inside the settle was never routed: nothing is sent.
    let mut b = seeded_bench(Gate::Open, &[]).await;
    route(&mut b);
    let stay = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[0], b.now);
    reply(&b, &[&stay]);
    b.tick().await.unwrap();
    reply(&b, &[]);
    b.tick().await.unwrap();
    assert!(settle_past(&mut b, 3).await.is_empty());
    assert!(b.s.items[ITEM].routed.is_none());
}

#[tokio::test]
async fn no_route_or_a_refused_route_sends_nothing_and_says_why() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let owner_tap = tap(&b.ctx.owner, &b.card(ITEM).event_id, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    // No routes.json: today's behaviour, the outbox only.
    assert!(settle_past(&mut b, 3).await.is_empty());
    assert!(b.s.unroutable.is_empty() && b.s.items[ITEM].routed.is_none());
    // Every refused entry is listed; none joins #bench or is told anything.
    let stranger = "a1".repeat(32);
    b.ctx.agents = vec![b.owner_hex(), "cd".repeat(32), b.writer_hex()];
    let routes = json!({
        "orchestrator": stranger,
        "Bad Writer": "a2".repeat(32),
        "short": "abc",
        "me": b.owner_hex(),
        "phone": "cd".repeat(32),
        "self": b.writer_hex(),
    });
    std::fs::write(b.root.join("routes.json"), routes.to_string()).unwrap();
    let members = b.s.members_added.clone();
    assert!(settle_past(&mut b, 3).await.is_empty());
    assert_eq!(b.s.members_added, members);
    assert_eq!(
        b.s.unroutable,
        [
            "Bad Writer: not a writer folder name",
            "me: the writer, the owner or an approver",
            "orchestrator: not a managed agent here that admits the Bench writer",
            "phone: the writer, the owner or an approver",
            "self: the writer, the owner or an approver",
            "short: not a 64-hex pubkey",
        ]
    );
    std::fs::write(b.root.join("routes.json"), "{not json").unwrap();
    b.tick().await.unwrap();
    assert_eq!(b.s.unroutable.len(), 1);
    assert!(b.s.unroutable[0].starts_with("routes.json: "));
}

#[tokio::test]
async fn notice_for_a_deleted_card_goes_top_level() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let agent = route(&mut b).public_key().to_hex();
    let card = b.card(ITEM).event_id;
    let owner_tap = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now);
    reply(&b, &[&owner_tap]);
    b.tick().await.unwrap();
    // Deleted by hand after the Recorded edit landed.
    b.relay.gone.lock().unwrap().insert(card.clone());
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.tick().await.unwrap();
    // The refused reply forgets the card (and reposts the board) ...
    assert_eq!(b.kinds()[before..], [9, 9, 5]);
    assert!(b.s.items[ITEM].card.is_none() && b.s.items[ITEM].routed.is_none());
    let before = b.posts().len();
    b.tick().await.unwrap();
    // ... and the notice goes top-level, followed by the board.
    assert_eq!(b.kinds()[before..], [9, 9, 5]);
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert_eq!(tag_value(&n[0], "e"), None);
    assert_eq!(tag_value(&n[0], "p"), Some(agent));
    let board = &b.posts()[before + 1].0;
    assert!(tag_value(board, "client")
        .unwrap()
        .starts_with("bench:board@"));
    assert!(b.s.items[ITEM].routed.is_some());
    assert!(settle_past(&mut b, 2).await.is_empty());
}

#[tokio::test]
async fn a_join_or_a_launch_holds_notices_one_settle() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let card = b.card(ITEM).event_id;
    let mv = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now);
    reply(&b, &[&mv]);
    b.tick().await.unwrap();
    // Settled before the route existed: the agent only sees #bench from its
    // join on, so the notice waits a settle after the join.
    b.now += ROUTE_SETTLE_SECS;
    route(&mut b);
    let before = b.posts().len();
    b.tick().await.unwrap();
    assert_eq!(b.kinds()[before], 9000);
    b.now += ROUTE_SETTLE_SECS - 4;
    b.tick().await.unwrap();
    assert!(notices(&b, before).is_empty());
    b.tick().await.unwrap();
    assert_eq!(notices(&b, before).len(), 1);

    // A change that settled while the app was closed waits a settle after
    // the launch (every launch rebuilds), when the agent is listening again.
    reply(&b, &[]);
    b.tick().await.unwrap();
    let stay = tap(&b.ctx.owner, &card, OPTION_EMOJI[0], b.now + 1);
    reply(&b, &[&stay]);
    b.tick().await.unwrap();
    b.now += ROUTE_SETTLE_SECS;
    b.s.needs_rebuild = true;
    let before = b.posts().len();
    b.tick().await.unwrap();
    b.now += ROUTE_SETTLE_SECS - 4;
    b.tick().await.unwrap();
    assert!(notices(&b, before).is_empty());
    b.tick().await.unwrap();
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert!(content(&n[0]).starts_with("Bench answer changed"));
}
