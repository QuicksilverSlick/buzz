//! Scoped cards in a bound project channel: the joins, the admin check, who
//! is pinged and who may answer, refused taps, Other replies and an
//! unreadable roles.json. Child of tests.rs so the fake relay and Bench
//! harness are in scope.

use super::super::roles::AgentAccess;
use super::answers::{last_query, reply, tap, ITEM};
use super::*;

const X: &str = "105f15cc-1d5a-431a-9826-9402fba1a12b";
const ASK: &str = "reset-worker/welcome";

struct People {
    craig: Keys,
    /// Reset Worker: writes the folder and is its route target.
    agent: Keys,
    /// An agent decider: never an answerer on a needs-you card.
    orch: Keys,
}

fn hex(k: &Keys) -> String {
    k.public_key().to_hex()
}

/// roles.json binds reset-worker to project reset in X, Craig decides;
/// routes.json sends the folder's answers to Reset Worker.
fn bind(b: &mut Bench) -> People {
    let p = People {
        craig: Keys::generate(),
        agent: Keys::generate(),
        orch: Keys::generate(),
    };
    let roles = json!({
        "ownerName": "Quicksilver",
        "projects": {"reset": {"name": "28 Day Reset", "channels": [X], "people": [
            {"pk": hex(&p.craig), "name": "Craig", "role": "decider"},
            {"pk": hex(&p.orch), "name": "28 Day Orchestrator", "role": "decider", "agent": true},
            {"pk": hex(&p.agent), "name": "Reset Worker", "role": "member", "agent": true},
        ]}},
        "writers": {"reset-worker": {"pubkey": hex(&p.agent), "projects": ["reset"]}},
    });
    std::fs::write(b.root.join("roles.json"), roles.to_string()).unwrap();
    b.ctx.agents = vec![hex(&p.agent)];
    let routes = json!({ "reset-worker": hex(&p.agent) });
    std::fs::write(b.root.join("routes.json"), routes.to_string()).unwrap();
    p
}

/// X's owner and admin list as `signer` publishes it: the owner, plus `more`.
fn admins(b: &Bench, signer: &Keys, more: &[&str]) -> nostr::Event {
    let mut tags = vec![
        nostr::Tag::parse(["d", X]).unwrap(),
        nostr::Tag::parse(["p", &b.owner_hex(), "owner"]).unwrap(),
    ];
    for pk in more {
        tags.push(nostr::Tag::parse(["p", pk, "admin"]).unwrap());
    }
    nostr::EventBuilder::new(nostr::Kind::Custom(39001), "")
        .tags(tags)
        .sign_with_keys(signer)
        .unwrap()
}

/// Reset Worker asks the project's deciders, recommending option 2.
fn ask(overrides: &[(&str, Value)]) -> String {
    let mut o = vec![
        ("project", json!("reset")),
        ("to", json!({"role": "decider"})),
        ("recommend", json!(2)),
        ("severity", json!("attention")),
        ("title", json!("Welcome email")),
        ("body", json!("Craig asked for a shorter welcome email.")),
        ("options", json!(["Send the new email", "Keep the old one"])),
    ];
    o.extend(overrides.iter().cloned());
    drop_json(&o)
}

/// The #bench bench with its legacy card, plus the scoped card in X.
async fn bound_bench() -> (Bench, People, nostr::Event) {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let p = bind(&mut b);
    let list = admins(&b, &b.relay.keys.clone(), &[]);
    reply(&b, &[&list]);
    b.drop_file("reset-worker", "welcome", &ask(&[]));
    b.settle(6).await;
    (b, p, list)
}

fn thread_reply(keys: &Keys, card: &str, text: &str, at: u64, pings: &[&str]) -> nostr::Event {
    let card = nostr::EventId::from_hex(card).unwrap();
    let thread = crate::events::ThreadRef {
        root_event_id: card,
        parent_event_id: card,
    };
    crate::events::build_message_with_client_tags(
        uuid::Uuid::parse_str(X).unwrap(),
        text,
        Some(&thread),
        pings,
        &[],
        &[],
        &[],
        &[],
        None,
        "http://127.0.0.1:1",
        &[],
    )
    .unwrap()
    .custom_created_at(nostr::Timestamp::from(at))
    .sign_with_keys(keys)
    .unwrap()
}

fn notices(b: &Bench, before: usize) -> Vec<Value> {
    b.posts()[before..]
        .iter()
        .filter(|(e, _)| tag_value(e, "client").is_some_and(|c| c.starts_with("bench:route:")))
        .map(|(e, _)| e.clone())
        .collect()
}

fn tags(ev: &Value, name: &str) -> Vec<Value> {
    ev["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t[0] == name)
        .cloned()
        .collect()
}

fn content(ev: &Value) -> &str {
    ev["content"].as_str().unwrap()
}

fn outbox(b: &Bench) -> Value {
    let text = std::fs::read_to_string(b.root.join(format!("outbox/{ASK}.json"))).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn a_bound_card_goes_to_its_channel_and_pings_its_answerers() {
    let (b, p, _) = bound_bench().await;
    let posts = b.posts();
    let (writer, agent) = (b.writer_hex(), hex(&p.agent));
    // The owner adds the writer, then the folder's route target, to X only.
    let joins: Vec<&Value> = posts
        .iter()
        .map(|(e, _)| e)
        .filter(|e| e["kind"] == 9000 && tag_value(e, "h").as_deref() == Some(X))
        .collect();
    assert_eq!(joins.len(), 2);
    assert!(joins.iter().all(|e| e["pubkey"] == b.owner_hex().as_str()));
    assert_eq!(tag_value(joins[0], "p"), Some(writer.clone()));
    assert_eq!(tag_value(joins[1], "p"), Some(agent.clone()));
    assert_eq!(b.s.joined[X], [writer, agent.clone()]);
    assert!(!b.s.members_added.contains(&agent));
    assert_eq!(b.s.channels[X].refused, None);

    let card_id = b.card(ASK).event_id;
    let card = &posts
        .iter()
        .find(|(e, _)| e["id"] == card_id.as_str())
        .unwrap()
        .0;
    let item = &b.s.items[ASK];
    assert_eq!(tag_value(card, "h").as_deref(), Some(X));
    assert_eq!(
        tags(card, "p"),
        [json!(["p", hex(&p.craig)])],
        "not the owner"
    );
    assert_eq!(content(card), render_card(item, "14:05"));
    assert!(
        content(card).ends_with(
            "For: Craig or Quicksilver (owner). Tap one number below to answer.\n\
             Recommended: 2 (Reset Worker)\nas of 14:05\nOr reply in this thread."
        ),
        "{}",
        content(card)
    );
    assert_eq!(
        tag_value(card, "client").unwrap(),
        format!("bench:card:{ASK}@{}", card_hash(item))
    );
    assert_eq!(item.pinged, [hex(&p.craig)]);
    assert_eq!(b.card(ASK).seeded, 2);
    // The board, in #bench, lists it as waiting on Craig.
    let board = posts.iter().rev().find(|(e, _)| e["kind"] == 9).unwrap();
    assert_eq!(tag_value(&board.0, "h"), Some(b.channel()));
    let link = format!("buzz://message?channel={X}&id={card_id}");
    let text = content(&board.0);
    assert!(
        text.contains("needs you: 1 · waiting on others: 1"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "Waiting on others\n- 🟠 Welcome email (dreamforge) for Craig → {link}"
        )),
        "{text}"
    );
}

#[tokio::test]
async fn craig_answers_and_the_owner_overrides() {
    let (mut b, p, list) = bound_bench().await;
    let (legacy, card) = (b.card(ITEM).event_id, b.card(ASK).event_id);
    let craig = hex(&p.craig);
    let craig_tap = tap(&p.craig, &card, OPTION_EMOJI[0], b.now);
    reply(&b, &[&list, &craig_tap]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    let mut chans = vec![b.channel(), X.to_string()];
    chans.sort();
    assert_eq!(
        last_query(&b),
        json!([{
            "kinds": [7, 9],
            "authors": [b.owner_hex(), "cd".repeat(32), craig],
            "#h": chans,
            "#e": [legacy, card],
            "limit": 500,
        }])
    );
    let a = b.s.items[ASK].answer.clone().unwrap();
    assert_eq!(
        (a.option, a.by.as_str(), a.by_name.as_deref(), a.role),
        (0, craig.as_str(), Some("Craig"), Some(AnswerRole::Decider))
    );
    let posts = b.posts()[before..].to_vec();
    let edit = &posts[0].0;
    assert_eq!(edit["kind"], 40003);
    assert_eq!(tag_value(edit, "h").as_deref(), Some(X));
    assert!(
        content(edit).contains(&format!(
            "\nRecorded: 1. Send the new email · Craig · {}\n",
            a.hhmm
        )),
        "{}",
        content(edit)
    );
    let out = outbox(&b);
    assert_eq!(out["answerer"], json!(craig));
    assert_eq!(out["answererRole"], json!("decider"));
    assert_eq!(out["eligible"], json!([craig]));
    assert_eq!(out["source"], json!("tap"));
    assert_eq!(out["reaction"]["id"], json!(craig_tap.id.to_hex()));

    // Routed after the settle, in the card's thread in X, naming Craig.
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.settle(2).await;
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert_eq!(tag_value(&n[0], "h").as_deref(), Some(X));
    assert_eq!(tags(&n[0], "p"), [json!(["p", hex(&p.agent)])]);
    assert_eq!(tags(&n[0], "e"), [json!(["e", card, "", "reply"])]);
    assert!(
        content(&n[0]).contains(" by Craig (decider). "),
        "{}",
        content(&n[0])
    );
    assert_eq!(
        tags(&n[0], "df-answer"),
        [json!(["df-answer", "1", craig, craig_tap.id.to_hex()])]
    );

    // The owner's tap wins, and the card and the notice say so.
    let owner_tap = tap(&b.ctx.owner, &card, OPTION_EMOJI[1], b.now);
    reply(&b, &[&list, &craig_tap, &owner_tap]);
    let before = b.posts().len();
    b.tick().await.unwrap();
    let a = b.s.items[ASK].answer.clone().unwrap();
    assert_eq!((a.option, a.by_name), (1, None));
    let edit = &b.posts()[before].0;
    assert!(
        content(edit).contains("\nRecorded: 2. Keep the old one · Quicksilver · "),
        "{}",
        content(edit)
    );
    assert_eq!(outbox(&b)["answererRole"], json!("owner"));
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.settle(2).await;
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert!(
        content(&n[0]).starts_with("Bench answer changed · reset-worker/welcome")
            && content(&n[0]).contains(" by Quicksilver (owner). "),
        "{}",
        content(&n[0])
    );
}

#[tokio::test]
async fn taps_that_do_not_count_are_logged_once() {
    let (mut b, p, list) = bound_bench().await;
    b.ctx.access = vec![AgentAccess {
        pubkey: hex(&p.orch),
        anyone: false,
        allowlist: vec![hex(&p.craig)],
    }];
    let (legacy, card) = (b.card(ITEM).event_id, b.card(ASK).event_id);
    let orch = tap(&p.orch, &card, OPTION_EMOJI[0], b.now);
    let stranger = tap(&Keys::generate(), &card, OPTION_EMOJI[1], b.now);
    let craig_on_bench = tap(&p.craig, &legacy, OPTION_EMOJI[0], b.now);
    reply(&b, &[&list, &orch, &stranger, &craig_on_bench]);
    b.settle(2).await;
    assert!(b.s.items[ASK].answer.is_none() && b.s.items[ITEM].answer.is_none());
    let got: Vec<(&str, String, usize, &str)> =
        b.s.refused_taps
            .iter()
            .map(|t| {
                (
                    t.item.as_str(),
                    t.event_id.clone(),
                    t.option,
                    t.why.as_str(),
                )
            })
            .collect();
    assert_eq!(
        got,
        [
            (
                ITEM,
                craig_on_bench.id.to_hex(),
                1,
                "not an answerer on this card"
            ),
            (ASK, orch.id.to_hex(), 1, "agent on a needs-you card"),
            (ASK, stranger.id.to_hex(), 2, "not an answerer on this card"),
        ]
    );
}

#[tokio::test]
async fn a_shared_admin_or_an_unsigned_list_refuses_the_channel() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let p = bind(&mut b);
    let relay = b.relay.keys.clone();
    let shared = admins(&b, &relay, &[&hex(&p.craig)]);
    reply(&b, &[&shared]);
    b.drop_file("reset-worker", "welcome", &ask(&[]));
    b.settle(4).await;
    let refused = |b: &Bench| b.s.channels[X].refused.clone();
    assert_eq!(
        refused(&b).as_deref(),
        Some("an owner or admin outside your keys")
    );
    assert!(b.s.items[ASK].card.is_none());
    assert!(!b
        .posts()
        .iter()
        .any(|(e, _)| e["kind"] == 9 && tag_value(e, "h").as_deref() == Some(X)));
    // The board, by its latest post or edit.
    let posts = b.posts();
    let board = posts
        .iter()
        .rev()
        .find(|(e, _)| content(e).starts_with("Bench · as of"))
        .unwrap();
    assert!(
        content(&board.0).ends_with(&format!(
            "\n\nCards refused\n- {X}: an owner or admin outside your keys"
        )),
        "{}",
        content(&board.0)
    );
    // A list the relay did not sign counts for nothing.
    let forged = admins(&b, &Keys::generate(), &[]);
    reply(&b, &[&forged]);
    b.now += 60;
    b.settle(2).await;
    assert_eq!(refused(&b).as_deref(), Some("no admin list from the relay"));
    assert!(b.s.items[ASK].card.is_none());
    // The relay's own list with only the owner lets the card in.
    let clean = admins(&b, &relay, &[]);
    reply(&b, &[&clean]);
    b.now += 60;
    b.settle(4).await;
    assert_eq!(refused(&b), None);
    assert!(b.s.items[ASK].card.is_some());
}

#[tokio::test]
async fn a_failed_join_never_stalls_bench() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    bind(&mut b);
    let list = admins(&b, &b.relay.keys.clone(), &[]);
    reply(&b, &[&list]);
    b.relay.gone.lock().unwrap().insert(X.to_string());
    b.drop_file("reset-worker", "welcome", &ask(&[]));
    b.drop_file(
        "orchestrator",
        "other",
        &drop_json(&[("title", json!("Other"))]),
    );
    b.settle(4).await;
    assert!(b.s.items["orchestrator/other"].card.is_some());
    assert!(b.s.items[ASK].card.is_none());
    let tried = |b: &Bench| {
        b.posts()
            .iter()
            .filter(|(e, _)| e["kind"] == 9000 && tag_value(e, "h").as_deref() == Some(X))
            .count()
    };
    // Once, not every tick, and the board says why.
    assert_eq!(tried(&b), 1);
    let why = format!("could not add {}", &b.writer_hex()[..8]);
    assert_eq!(b.s.channels[X].refused.as_deref(), Some(why.as_str()));
    b.relay.gone.lock().unwrap().clear();
    b.now += 60;
    b.settle(6).await;
    assert_eq!(tried(&b), 3);
    assert_eq!(b.s.channels[X].refused, None);
    assert!(b.s.items[ASK].card.is_some());
}

#[tokio::test]
async fn a_route_target_that_answers_is_not_routed() {
    let (mut b, p, list) = bound_bench().await;
    let card = b.card(ASK).event_id;
    let craig_tap = tap(&p.craig, &card, OPTION_EMOJI[0], b.now);
    reply(&b, &[&list, &craig_tap]);
    b.tick().await.unwrap();
    // An answered card keeps the stamp it was answered under, even one that
    // names the route target.
    let st = b.s.items.get_mut(ASK).unwrap().stamp.as_mut().unwrap();
    st.answerers.push(Answerer {
        pk: hex(&p.agent),
        name: "Reset Worker".to_string(),
        role: AnswerRole::Decider,
        agent: true,
    });
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.settle(2).await;
    assert!(notices(&b, before).is_empty());
    assert!(b
        .s
        .unroutable
        .contains(&format!("{ASK}: route target is an answerer on this card")));
}

#[tokio::test]
async fn other_replies_reach_the_route_target() {
    let (mut b, p, list) = bound_bench().await;
    let card = b.card(ASK).event_id;
    let agent = hex(&p.agent);
    let r1 = thread_reply(&p.craig, &card, "Can it be shorter still?", b.now, &[]);
    reply(&b, &[&list, &r1]);
    b.tick().await.unwrap();
    let r = b.s.items[ASK].reply.clone().unwrap();
    assert_eq!(
        (r.event_id.as_str(), r.by_name.as_deref()),
        (r1.id.to_hex().as_str(), Some("Craig"))
    );
    assert!(b.s.items[ASK].answer.is_none());
    // Held one settle after the joins, then one notice, with no reply text.
    b.now += ROUTE_SETTLE_SECS;
    let before = b.posts().len();
    b.settle(2).await;
    let n = notices(&b, before);
    assert_eq!(n.len(), 1);
    assert_eq!(
        tag_value(&n[0], "client").unwrap(),
        format!("bench:route:{ASK}@r{}", &r1.id.to_hex()[..16])
    );
    assert_eq!(
        content(&n[0]),
        render_route_reply(&b.s.items[ASK], &local_hhmm(r.at))
    );
    assert!(!content(&n[0]).contains("shorter"));
    assert_eq!(tags(&n[0], "p"), [json!(["p", agent])]);
    assert_eq!(tag_value(&n[0], "h").as_deref(), Some(X));
    assert_eq!(
        tags(&n[0], "df-answer"),
        [json!(["df-answer", "other", hex(&p.craig), r1.id.to_hex()])]
    );
    // A second reply is a second notice.
    let r2 = thread_reply(&p.craig, &card, "Or drop the second paragraph", b.now, &[]);
    reply(&b, &[&list, &r1, &r2]);
    let before = b.posts().len();
    b.settle(2).await;
    assert_eq!(notices(&b, before).len(), 1);
    // One that mentions the agent already woke it.
    let r3 = thread_reply(&p.craig, &card, "Over to you", b.now, &[&agent]);
    reply(&b, &[&list, &r1, &r2, &r3]);
    let before = b.posts().len();
    b.settle(2).await;
    assert!(notices(&b, before).is_empty());
    assert_eq!(b.s.items[ASK].routed_reply, Some(r3.id.to_hex()));
    // A reply that names an option is that answer.
    let r4 = thread_reply(&p.craig, &card, " 2 ", b.now, &[]);
    reply(&b, &[&list, &r1, &r2, &r3, &r4]);
    b.tick().await.unwrap();
    let a = b.s.items[ASK].answer.clone().unwrap();
    assert_eq!(
        (a.option, a.source, a.reaction_id),
        (1, AnswerSource::Reply, r4.id.to_hex())
    );
    assert_eq!(outbox(&b)["source"], json!("reply"));
    assert_eq!(
        b.s.items[ASK].reply.as_ref().unwrap().event_id,
        r3.id.to_hex()
    );
}

#[tokio::test]
async fn unreadable_roles_hold_scoped_drops_and_cards_go_owner_only() {
    let (mut b, _p, _list) = bound_bench().await;
    let old = b.card(ASK).event_id;
    let good = std::fs::read_to_string(b.root.join("roles.json")).unwrap();
    std::fs::write(b.root.join("roles.json"), "{not json").unwrap();
    b.drop_file(
        "reset-worker",
        "second",
        &ask(&[("title", json!("Second"))]),
    );
    // Every tick finishes, then says why.
    for _ in 0..4 {
        let err = b.tick().await.unwrap_err();
        assert!(err.starts_with("roles.json: "), "{err}");
    }
    assert!(b.root.join("inbox/reset-worker/second.json").exists());
    assert!(!b.s.items.contains_key("reset-worker/second"));
    // The open card was reposted for the owner alone, and pings him.
    let new = b.card(ASK).event_id;
    assert_ne!(new, old);
    let posts = b.posts();
    let card = &posts
        .iter()
        .find(|(e, _)| e["id"] == new.as_str())
        .unwrap()
        .0;
    assert!(content(card).contains("\nFor: the owner. Tap one number below to answer.\n"));
    assert_eq!(tags(card, "p"), [json!(["p", b.owner_hex()])]);
    // Readable again: the waiting drop is taken.
    std::fs::write(b.root.join("roles.json"), good).unwrap();
    b.tick().await.unwrap();
    assert!(b.s.items.contains_key("reset-worker/second"));
}

#[tokio::test]
async fn recover_pages_through_every_channel() {
    let (mut b, _p, _list) = bound_bench().await;
    let card = b.card(ASK);
    let posted = b
        .posts()
        .into_iter()
        .find(|(e, _)| e["id"] == card.event_id.as_str())
        .unwrap()
        .0;
    // 600 older writer events, then a launch that lost the card.
    let w = b.ctx.writer.clone();
    let mut relay: Vec<Value> = (0..600)
        .map(|i| serde_json::to_value(kind9(&w, b.now - 1000 - i, None)).unwrap())
        .collect();
    relay.push(posted);
    *b.relay.query_reply.lock().unwrap() = relay.clone();
    b.s.items.get_mut(ASK).unwrap().card = None;
    b.s.needs_rebuild = true;
    let queries = b.relay.queries.lock().unwrap().len();
    // The fake relay serves the same events to the poll that follows.
    let err = b.tick().await.unwrap_err();
    assert!(err.contains("answers truncated"), "{err}");
    let sent: Vec<Value> = b.relay.queries.lock().unwrap()[queries..]
        .iter()
        .filter(|q| q[0]["kinds"] == json!([9]))
        .map(|q| q[0].clone())
        .collect();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["#h"], json!([b.channel(), X]));
    assert_eq!(sent[0].get("until"), None);
    let mut ts: Vec<u64> = relay
        .iter()
        .map(|e| e["created_at"].as_u64().unwrap())
        .collect();
    ts.sort_by(|a, b| b.cmp(a));
    assert_eq!(sent[1]["until"], json!(ts[499] - 1));
    assert_eq!(b.card(ASK).event_id, card.event_id);
    assert_eq!(b.s.strays.len(), 600);
}
