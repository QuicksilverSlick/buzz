//! Step 1 scoped cards: the new drop fields, roles.json, owner-wins answers
//! over the shared vectors, and the For/Recommended/Other/receipt text.
//! Child of tests.rs so the drop helpers are in scope.

use super::*;

/// A lowercase 64-hex key of one repeated digit.
fn hex(c: char) -> String {
    c.to_string().repeat(64)
}

/// Recorded on the code before the new fields existed: a drop without them
/// must hash byte for byte as it always has, or every live card reposts.
#[test]
fn version_hash_is_unchanged_without_new_fields() {
    let item = check(&[]).unwrap();
    assert_eq!(item.hash, "2673997a20fd822e");
    assert_eq!(card_hash(&item), "2673997a20fd822e.4");
    assert!(!item.scoped());
    // Each new field joins the hash when it is set.
    let p = ("project", json!("reset"));
    let hashes: HashSet<String> = [
        vec![],
        vec![p.clone()],
        vec![p.clone(), ("channel", json!(UUID))],
        vec![p.clone(), ("replyTo", json!(hex('a')))],
        vec![p.clone(), ("to", json!({"role": "decider"}))],
        vec![("decision", json!("merge-pr"))],
        vec![("recommend", json!(1))],
    ]
    .iter()
    .map(|o| check(o).unwrap().hash)
    .collect();
    assert_eq!(hashes.len(), 7);
}

#[test]
fn validate_checks_the_scope_fields() {
    let p = ("project", json!("reset"));
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("project slug", vec![("project", json!("Reset_1"))]),
        ("decision slug", vec![("decision", json!("merge pr"))]),
        ("channel without project", vec![("channel", json!(UUID))]),
        (
            "replyTo without project",
            vec![("replyTo", json!(hex('a')))],
        ),
        (
            "to without project",
            vec![("to", json!({"role": "decider"}))],
        ),
        (
            "channel uppercase",
            vec![p.clone(), ("channel", json!(UUID.to_uppercase()))],
        ),
        (
            "channel nil uuid",
            vec![
                p.clone(),
                ("channel", json!("00000000-0000-0000-0000-000000000000")),
            ],
        ),
        ("replyTo short", vec![p.clone(), ("replyTo", json!("ab"))]),
        (
            "replyTo uppercase",
            vec![p.clone(), ("replyTo", json!(hex('A')))],
        ),
        ("to empty", vec![p.clone(), ("to", json!({}))]),
        (
            "to role unknown",
            vec![p.clone(), ("to", json!({"role": "boss"}))],
        ),
        (
            "to unknown field",
            vec![p.clone(), ("to", json!({"role": "owner", "whom": 1}))],
        ),
        (
            "to uppercase key",
            vec![p.clone(), ("to", json!({"people": [hex('A')]}))],
        ),
        (
            "to nine people",
            vec![
                p.clone(),
                (
                    "to",
                    json!({"people": "123456789".chars().map(hex).collect::<Vec<_>>()}),
                ),
            ],
        ),
        ("recommend 0", vec![("recommend", json!(0))]),
        ("recommend past the options", vec![("recommend", json!(3))]),
        ("recommend negative", vec![("recommend", json!(-1))]),
        (
            "recommend on a decision",
            vec![
                ("kind", json!("decision")),
                ("decidedBy", json!("claude")),
                ("recommend", json!(1)),
            ],
        ),
    ];
    for (name, overrides) in cases {
        assert!(check(&overrides).is_err(), "{name} should be rejected");
    }

    let full = |people: Value| {
        check(&[
            p.clone(),
            ("channel", json!(UUID)),
            ("replyTo", json!(hex('c'))),
            ("to", json!({"people": people, "role": "decider"})),
            ("decision", json!("merge-pr")),
            ("recommend", json!(2)),
        ])
        .unwrap()
    };
    let item = full(json!([hex('b'), hex('a'), hex('b')]));
    assert!(item.scoped());
    assert_eq!(item.project.as_deref(), Some("reset"));
    assert_eq!(item.channel.as_deref(), Some(UUID));
    assert_eq!(item.reply_to, Some(hex('c')));
    let to = item.to.as_ref().unwrap();
    assert_eq!(to.people, [hex('a'), hex('b')]);
    assert_eq!(to.role, Some(ToRole::Decider));
    assert_eq!(item.decision.as_deref(), Some("merge-pr"));
    assert_eq!(item.recommend, Some(2));
    // Order and repeats in `to.people` never change the hash.
    assert_eq!(item.hash, full(json!([hex('a'), hex('b')])).hash);
    // decision and recommend alone need no project.
    assert!(check(&[("decision", json!("merge-pr"))]).unwrap().scoped());
}

// ── roles.json ──────────────────────────────────────────────────────────

use super::super::roles::{self, AgentAccess, Roles};

const CH: &str = "105f15cc-1d5a-431a-9826-9402fba1a12b";
const CH3: &str = "223e4567-e89b-12d3-a456-426614174000";

/// Craig decides, the Orchestrator is an agent decider, Reset Worker a
/// member agent that writes the folder, Dana a project owner.
fn roster() -> Value {
    json!({
        "ownerName": "Quicksilver",
        "projects": {
            "reset": {"name": "28 Day Reset", "channels": [CH, UUID], "people": [
                {"pk": hex('1'), "name": "Craig", "role": "decider"},
                {"pk": hex('2'), "name": "28 Day Orchestrator", "role": "decider", "agent": true},
                {"pk": hex('3'), "name": "Reset Worker", "role": "member", "agent": true},
                {"pk": hex('4'), "name": "Dana", "role": "owner"},
            ]},
            "other": {"name": "Other", "channels": [CH3]},
        },
        "writers": {
            "reset-worker": {"pubkey": hex('3'), "projects": ["reset"]},
            "orch-folder": {"pubkey": hex('2'), "projects": ["reset"]},
        },
    })
}

fn roles_with(f: impl FnOnce(&mut Value)) -> Result<Roles, String> {
    let mut r = roster();
    f(&mut r);
    roles::parse_roles(&r.to_string())
}

fn drop_as(writer: &str, overrides: &[(&str, Value)]) -> Item {
    validate(writer, "q", &drop_json(overrides), NOW).unwrap()
}

type Mutation = Box<dyn FnOnce(&mut Value)>;
/// Writer, drop overrides, roles, the rejection.
type BindCase<'a> = (&'a str, Vec<(&'a str, Value)>, Option<&'a Roles>, &'a str);

#[test]
fn parse_roles_rejects_each_bad_shape() {
    roles_with(|_| {}).unwrap();
    let person = |field: &'static str, v: Value| -> Mutation {
        Box::new(move |r| r["projects"]["reset"]["people"][0][field] = v)
    };
    let other = |field: &'static str, v: Value| -> Mutation {
        Box::new(move |r| r["projects"]["other"][field] = v)
    };
    let cases: Vec<(&str, Mutation)> = vec![
        ("unknown field", Box::new(|r| r["extra"] = json!(1))),
        ("role unknown", person("role", json!("boss"))),
        ("bad pk", person("pk", json!(hex('A')))),
        ("pk twice", person("pk", json!(hex('2')))),
        ("name without letters", person("name", json!("!!"))),
        ("name too long", person("name", json!("C".repeat(33)))),
        ("name with a mention", person("name", json!("nostr:npub1x"))),
        ("project name", other("name", json!(""))),
        ("no channels", other("channels", json!([]))),
        ("channel not a uuid", other("channels", json!(["x"]))),
        (
            "channel uppercase",
            other("channels", json!([CH3.to_uppercase()])),
        ),
        ("channel in two projects", other("channels", json!([CH]))),
        (
            "65 people",
            other(
                "people",
                (0..65)
                    .map(|i| json!({"pk": format!("{i:064x}"), "name": "P", "role": "member"}))
                    .collect(),
            ),
        ),
        (
            "project slug",
            Box::new(|r| r["projects"]["Reset"] = r["projects"]["other"].take()),
        ),
        ("ownerName empty", Box::new(|r| r["ownerName"] = json!(""))),
        (
            "writer folder",
            Box::new(|r| r["writers"]["Reset_Worker"] = json!({"projects": []})),
        ),
        (
            "writer project unknown",
            Box::new(|r| r["writers"]["reset-worker"]["projects"] = json!(["nope"])),
        ),
        (
            "writer pubkey",
            Box::new(|r| r["writers"]["reset-worker"]["pubkey"] = json!("ab")),
        ),
    ];
    for (name, f) in cases {
        assert!(roles_with(f).is_err(), "{name} should be rejected");
    }
}

#[test]
fn bind_checks_the_folder_and_fills_the_channel() {
    let roles = roles_with(|_| {}).unwrap();
    let bind = |writer: &str, overrides: &[(&str, Value)], roles: Option<&Roles>| {
        let mut item = drop_as(writer, overrides);
        roles::bind(&mut item, roles).map(|()| item)
    };
    let p = ("project", json!("reset"));
    // Unbound folders keep today's cards; decision and recommend are open.
    assert_eq!(
        bind("orchestrator", &[], Some(&roles)).unwrap().channel,
        None
    );
    let open = [("decision", json!("merge-pr")), ("recommend", json!(1))];
    bind("orchestrator", &open, None).unwrap();
    let member = format!("to: {} is not an owner or decider of reset", hex('3'));
    let cases: Vec<BindCase> = vec![
        (
            "orchestrator",
            vec![p.clone()],
            Some(&roles),
            "project: folder orchestrator is not bound in roles.json",
        ),
        (
            "reset-worker",
            vec![p.clone()],
            None,
            "project: folder reset-worker is not bound in roles.json",
        ),
        (
            "reset-worker",
            vec![],
            Some(&roles),
            "bound folder: project is required",
        ),
        (
            "reset-worker",
            vec![("project", json!("other"))],
            Some(&roles),
            "project other: not one of this folder's projects",
        ),
        (
            "reset-worker",
            vec![("project", json!("nope"))],
            Some(&roles),
            "project nope: not one of this folder's projects",
        ),
        (
            "reset-worker",
            vec![p.clone(), ("channel", json!(CH3))],
            Some(&roles),
            "channel: not a channel of project reset",
        ),
        (
            "reset-worker",
            vec![p.clone(), ("to", json!({"people": [hex('3')]}))],
            Some(&roles),
            &member,
        ),
    ];
    for (writer, overrides, roles, want) in cases {
        assert_eq!(bind(writer, &overrides, roles).unwrap_err(), want);
    }
    let to_craig = ("to", json!({"people": [hex('1')]}));
    let item = bind("reset-worker", &[p.clone(), to_craig], Some(&roles)).unwrap();
    assert_eq!(item.channel.as_deref(), Some(CH));
    let item = bind("reset-worker", &[p, ("channel", json!(UUID))], Some(&roles)).unwrap();
    assert_eq!(item.channel.as_deref(), Some(UUID));
}

#[test]
fn stamp_resolves_who_may_answer() {
    let roles = roles_with(|_| {}).unwrap();
    let owner_set = [hex('f')];
    let writer_hex = hex('e');
    let stamp = |writer: &str,
                 overrides: &[(&str, Value)],
                 access: &[AgentAccess],
                 route: Option<&str>,
                 owners: &[String]| {
        let mut o = vec![("project", json!("reset"))];
        o.extend(overrides.iter().cloned());
        let item = drop_as(writer, &o);
        roles::stamp(&item, Some(&roles), owners, access, route, &writer_hex).unwrap()
    };
    let names = |writer: &str,
                 overrides: &[(&str, Value)],
                 access: &[AgentAccess],
                 route: Option<&str>,
                 owners: &[String]| {
        stamp(writer, overrides, access, route, owners)
            .answerers
            .into_iter()
            .map(|a| a.name)
            .collect::<Vec<_>>()
    };
    let w = "reset-worker";
    let decider = ("to", json!({"role": "decider"}));
    let delegable = ("needs", json!("delegable"));
    let just_decider = [decider.clone()];
    let both = [decider.clone(), delegable.clone()];
    let allow = |list: Vec<String>| {
        vec![AgentAccess {
            pubkey: hex('2'),
            anyone: false,
            allowlist: list,
        }]
    };

    // A needs-you card: the agent decider never answers.
    assert_eq!(
        names(w, &just_decider, &[], None, &owner_set),
        ["Craig", "Dana"]
    );
    let to_owner = ("to", json!({"role": "owner"}));
    assert_eq!(names(w, &[to_owner], &[], None, &owner_set), ["Dana"]);
    // Delegable: an agent whose allowlist names a human on the card goes,
    // and so does one with no access entry (it counts as Anyone).
    let craig_allowed = allow(vec![hex('1')]);
    assert_eq!(
        names(w, &both, &craig_allowed, None, &owner_set),
        ["Craig", "Dana"]
    );
    assert_eq!(names(w, &both, &[], None, &owner_set), ["Craig", "Dana"]);
    // A closed agent stays beside humans who cannot instruct it.
    assert_eq!(
        names(w, &both, &allow(vec![hex('9')]), None, &owner_set),
        ["Craig", "28 Day Orchestrator", "Dana"]
    );
    // With no human on the card the agent stays, whatever its access.
    let only_agent = [("to", json!({"people": [hex('2')]})), delegable.clone()];
    assert_eq!(
        names(w, &only_agent, &[], None, &owner_set),
        ["28 Day Orchestrator"]
    );
    // The raiser, the route target, the owner's keys and the Bench writer go.
    let closed = allow(vec![]);
    assert_eq!(
        names("orch-folder", &both, &closed, None, &owner_set),
        ["Craig", "Dana"]
    );
    let craig = hex('1');
    assert_eq!(
        names(w, &just_decider, &[], Some(&craig), &owner_set),
        ["Dana"]
    );
    assert_eq!(names(w, &just_decider, &[], None, &[hex('4')]), ["Craig"]);
    let item = drop_as(w, &[("project", json!("reset")), decider.clone()]);
    let as_writer = roles::stamp(&item, Some(&roles), &owner_set, &[], None, &craig).unwrap();
    assert_eq!(as_writer.answerers.len(), 1);
    // A host agent is an agent even when the roster does not say so.
    let host = [AgentAccess {
        pubkey: hex('1'),
        anyone: false,
        allowlist: vec![],
    }];
    assert_eq!(names(w, &just_decider, &host, None, &owner_set), ["Dana"]);

    let st = stamp(
        w,
        &[decider.clone(), ("recommend", json!(2))],
        &[],
        None,
        &owner_set,
    );
    assert_eq!(st.owner_name, "Quicksilver");
    assert_eq!(st.recommender.as_deref(), Some("Reset Worker"));
    let (craig, dana) = (&st.answerers[0], &st.answerers[1]);
    assert_eq!((craig.role, craig.agent), (AnswerRole::Decider, false));
    assert_eq!(
        (dana.pk.as_str(), dana.role),
        (hex('4').as_str(), AnswerRole::Owner)
    );
    // No roles, no `to`, or not scoped: never a wider card.
    let none = roles::stamp(&item, None, &owner_set, &[], None, &writer_hex);
    assert_eq!(none, Some(Stamp::default()));
    assert!(names(w, &[], &[], None, &owner_set).is_empty());
    let legacy = check(&[]).unwrap();
    assert_eq!(
        roles::stamp(&legacy, Some(&roles), &owner_set, &[], None, &writer_hex),
        None
    );
}

#[test]
fn card_hash_carries_the_stamp_only_when_set() {
    let roles = roles_with(|_| {}).unwrap();
    let to = ("to", json!({"role": "decider"}));
    let mut item = drop_as("reset-worker", &[("project", json!("reset")), to]);
    assert_eq!(card_hash(&item), format!("{}.4", item.hash));
    item.stamp = roles::stamp(&item, Some(&roles), &[], &[], None, "");
    let stamped = card_hash(&item);
    let parts: Vec<&str> = stamped.split('.').collect();
    assert_eq!(
        (parts[0], parts[1].len(), parts[2]),
        (item.hash.as_str(), 8, "4")
    );
    // Another roster reposts the card; the Drop part is unchanged.
    item.stamp.as_mut().unwrap().answerers.pop();
    assert_ne!(card_hash(&item), stamped);
    assert!(card_hash(&item).starts_with(&format!("{}.", item.hash)));
}

// ── answers: owner wins, over the vectors the desktop shares ────────────

use super::answers::tap;
use std::collections::HashMap;

const ALIASES: [&str; 6] = ["owner", "phone", "craig", "agent", "stranger", "writer"];

#[test]
fn answer_vectors() {
    let v: Value = serde_json::from_str(include_str!("../answer_vectors.json")).unwrap();
    let keys: HashMap<&str, Keys> = ALIASES.iter().map(|a| (*a, Keys::generate())).collect();
    let hex_of = |who: &Value| keys[who.as_str().unwrap()].public_key().to_hex();
    let alias = |e: &nostr::Event| {
        let pk = e.pubkey;
        *ALIASES
            .iter()
            .find(|a| keys[*a].public_key() == pk)
            .unwrap()
    };
    let owner_set: Vec<String> = v["ownerSet"]
        .as_array()
        .unwrap()
        .iter()
        .map(hex_of)
        .collect();
    let writer_hex = keys["writer"].public_key().to_hex();
    let (card, other) = (hex('c'), hex('d'));
    for case in v["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut overrides = vec![
            ("options", case["options"].clone()),
            ("needs", case["needs"].clone()),
        ];
        if case["scoped"] == json!(true) {
            overrides.push(("project", json!("p")));
        }
        let mut item = check(&overrides).unwrap();
        assert_eq!(item.scoped(), case["scoped"] == json!(true), "{name}");
        item.card = Some(Posted {
            event_id: card.clone(),
            created_at: 1,
            hash: item.hash.clone(),
            seeded: 2,
        });
        if item.scoped() {
            let answerers = case["to"].as_array().unwrap().iter().map(|t| Answerer {
                pk: hex_of(&t["who"]),
                name: t["who"].as_str().unwrap().to_string(),
                role: serde_json::from_value(t["role"].clone()).unwrap(),
                agent: t["agent"].as_bool().unwrap(),
            });
            item.stamp = Some(Stamp {
                answerers: answerers.collect(),
                ..Default::default()
            });
        }
        let events: Vec<nostr::Event> = case["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                let k = &keys[e["who"].as_str().unwrap()];
                let target = if e["target"] == json!("other") {
                    &other
                } else {
                    &card
                };
                let at = e["at"].as_u64().unwrap();
                let content = e["content"].as_str().unwrap();
                if e["kind"] == json!(7) {
                    let emoji = match content {
                        "heart" => "\u{2764}\u{FE0F}".to_string(),
                        n => format!("{n}\u{FE0F}\u{20E3}"),
                    };
                    return tap(k, target, &emoji, at);
                }
                let root = nostr::Tag::parse(["e", target.as_str(), "", "root"]).unwrap();
                nostr::EventBuilder::new(nostr::Kind::Custom(9), content)
                    .tags([root])
                    .custom_created_at(nostr::Timestamp::from(at))
                    .sign_with_keys(k)
                    .unwrap()
            })
            .collect();

        let answer = pick_answer(&item, &events, &owner_set, &writer_hex)
            .map(|(i, e)| json!({"who": alias(e), "option": i + 1}));
        assert_eq!(
            answer.unwrap_or(Value::Null),
            case["answer"],
            "{name}: answer"
        );
        let reply =
            pick_reply(&item, &events, &owner_set, &writer_hex).map(|e| json!({"who": alias(e)}));
        assert_eq!(reply.unwrap_or(Value::Null), case["reply"], "{name}: reply");
        let refused: Vec<&str> = refused_taps(&item, &events, &owner_set, &writer_hex)
            .into_iter()
            .map(alias)
            .collect();
        assert_eq!(json!(refused), case["refused"], "{name}: refused");
        for (who, want) in case["eligible"].as_object().unwrap() {
            let pk = keys[who.as_str()].public_key().to_hex();
            let got = eligible(&item, &pk, &owner_set);
            assert_eq!(json!(got), *want, "{name}: eligible {who}");
        }
    }
}

#[test]
fn card_pings_follow_the_answerers() {
    let owner = hex('f');
    let pings = |item: &Item| card_pings(item, &owner);
    // Owner-only cards keep today's rule, scoped or not.
    let legacy = check(&[]).unwrap();
    assert_eq!(pings(&legacy), std::slice::from_ref(&owner));
    let mut quiet = legacy.clone();
    quiet.quiet_repost = true;
    assert!(pings(&quiet).is_empty());
    assert!(pings(&check(&[("needs", json!("delegable"))]).unwrap()).is_empty());
    let mut scoped = check(&[("project", json!("p"))]).unwrap();
    scoped.stamp = Some(Stamp::default());
    assert_eq!(pings(&scoped), std::slice::from_ref(&owner));
    // With answerers: the humans, and agents only on a delegable card, each
    // once per Drop; the owner is not pinged.
    let person = |c: char, agent: bool| Answerer {
        pk: hex(c),
        name: c.to_string(),
        role: AnswerRole::Decider,
        agent,
    };
    scoped.stamp = Some(Stamp {
        answerers: vec![person('1', false), person('2', true)],
        ..Default::default()
    });
    assert_eq!(pings(&scoped), [hex('1')]);
    scoped.needs = Needs::Delegable;
    assert_eq!(pings(&scoped), [hex('1'), hex('2')]);
    scoped.pinged = vec![hex('1')];
    assert_eq!(pings(&scoped), [hex('2')]);
}

// ── card and notice text ────────────────────────────────────────────────

const ASK: &str = "🟠 · dreamforge · reset-worker\n\
                   **Welcome email**\n\
                   \n\
                   Craig asked for a shorter welcome email.\n\
                   \n\
                   1. Send the new email\n\
                   2. Keep the old one\n";

fn craig() -> Answerer {
    Answerer {
        pk: hex('1'),
        name: "Craig".to_string(),
        role: AnswerRole::Decider,
        agent: false,
    }
}

/// Reset Worker asks Craig, recommending option 2, stamped as the service
/// would from the test roster.
fn ask() -> Item {
    let mut item = drop_as(
        "reset-worker",
        &[
            ("project", json!("reset")),
            ("to", json!({"role": "decider"})),
            ("recommend", json!(2)),
            ("severity", json!("attention")),
            ("title", json!("Welcome email")),
            ("body", json!("Craig asked for a shorter welcome email.")),
            ("options", json!(["Send the new email", "Keep the old one"])),
        ],
    );
    item.stamp = Some(Stamp {
        owner_name: "Quicksilver".to_string(),
        recommender: Some("Reset Worker".to_string()),
        answerers: vec![craig()],
    });
    item
}

fn answer_by(option: usize, by_name: Option<&str>, hhmm: &str) -> Option<Answer> {
    Some(Answer {
        reaction_id: hex('9'),
        option,
        answered_at: 1,
        by: hex('1'),
        by_name: by_name.map(str::to_string),
        role: Some(AnswerRole::Decider).filter(|_| by_name.is_some()),
        source: AnswerSource::Tap,
        hhmm: hhmm.to_string(),
    })
}

#[test]
fn scoped_card_says_who_it_is_for() {
    let mut item = ask();
    assert_eq!(
        render_card(&item, "14:05"),
        format!(
            "{ASK}\n\
             For: Craig or Quicksilver (owner). Tap one number below to answer.\n\
             Recommended: 2 (Reset Worker)\n\
             as of 14:05\n\
             Or reply in this thread."
        )
    );
    item.answer = answer_by(1, Some("Craig"), "14:02");
    assert_eq!(
        render_card(&item, "14:05"),
        format!(
            "{ASK}\n\
             For: Craig or Quicksilver (owner).\n\
             Recommended: 2 (Reset Worker)\n\
             as of 14:05\n\
             Recorded: 2. Keep the old one · Craig · 14:02\n\
             Or reply in this thread."
        )
    );
    // The owner's own keys: named from roles.json, timed by the edit.
    item.answer = answer_by(0, None, "");
    assert!(render_card(&item, "14:05").ends_with(
        "\nRecorded: 1. Send the new email · Quicksilver · 14:05\nOr reply in this thread."
    ));

    // Owner-only, and no owner name or recommender in roles.json.
    item.answer = None;
    let st = item.stamp.as_mut().unwrap();
    st.answerers.clear();
    assert!(render_card(&item, "14:05")
        .contains("\nFor: Quicksilver (owner). Tap one number below to answer.\n"));
    let dana = Answerer {
        pk: hex('4'),
        name: "Dana [x]".to_string(),
        role: AnswerRole::Owner,
        agent: false,
    };
    let st = item.stamp.as_mut().unwrap();
    *st = Stamp {
        owner_name: String::new(),
        recommender: None,
        answerers: vec![craig(), dana],
    };
    let card = render_card(&item, "14:05");
    assert!(card.contains("\nFor: Craig, Dana x or the owner. Tap one number below to answer.\nRecommended: 2 (reset-worker)\n"), "{card}");
    // Before the service stamps it, a scoped card is owner-only.
    item.stamp = None;
    assert!(render_card(&item, "14:05").contains("\nFor: the owner. Tap"));
}

#[test]
fn receipt_keeps_the_request_and_adds_the_outcome() {
    let mut request = ask();
    request.answer = answer_by(1, Some("Craig"), "14:02");
    let mut outcome = drop_as(
        "reset-worker",
        &[
            ("project", json!("reset")),
            ("kind", json!("status")),
            ("state", json!("done")),
            ("severity", Value::Null),
            ("options", Value::Null),
            ("title", json!("Kept the old email")),
            ("body", json!("No change was sent.")),
            (
                "links",
                json!([{"label": "PR", "url": "https://github.com/QuicksilverSlick/buzz/pull/1"}]),
            ),
        ],
    );
    outcome.request = Some(Box::new(request.clone()));
    assert_eq!(
        render_card(&outcome, "15:10"),
        format!(
            "{ASK}\n\
             Recommended: 2 (Reset Worker)\n\
             Answered: 2. Keep the old one · Craig · 14:02\n\
             \n\
             Outcome · done · 15:10\n\
             **Kept the old email**\n\
             \n\
             No change was sent.\n\
             \n\
             PR: https://github.com/QuicksilverSlick/buzz/pull/1"
        )
    );
    // Answered by a reply in the thread instead, or not at all.
    request.answer = None;
    request.reply = Some(Reply {
        event_id: hex('8'),
        by: hex('1'),
        by_name: Some("Craig".to_string()),
        role: Some(AnswerRole::Decider),
        at: 1,
        hhmm: "14:03".to_string(),
    });
    outcome.request = Some(Box::new(request.clone()));
    let card = render_card(&outcome, "15:10");
    assert!(
        card.contains("\nAnswered: Other, in this thread · Craig · 14:03\n\nOutcome"),
        "{card}"
    );
    request.reply = None;
    outcome.request = Some(Box::new(request));
    assert!(render_card(&outcome, "15:10").contains("\nAnswered: no answer recorded\n\nOutcome"));

    // A legacy card names nobody, and an answer from before step 1 has no
    // time of its own.
    let mut legacy = check(&[]).unwrap();
    legacy.answer = answer_by(1, None, "");
    let mut decision = check(&[
        ("kind", json!("decision")),
        ("decidedBy", json!("claude")),
        ("severity", Value::Null),
        ("options", Value::Null),
        ("title", json!("Moved the relay")),
        ("body", json!("")),
    ])
    .unwrap();
    decision.request = Some(Box::new(legacy));
    assert_eq!(
        render_card(&decision, "15:10"),
        "🔴 · dreamforge · orchestrator\n\
         **Relay choice**\n\
         \n\
         Stay or move?\n\
         \n\
         1. Stay\n\
         2. Move\n\
         \n\
         Answered: 2. Move\n\
         \n\
         Outcome · 15:10\n\
         **Moved the relay**"
    );
}

#[test]
fn notices_name_who_answered_on_scoped_cards() {
    let details = |item: &Item| {
        render_route(item, None, "14:05")
            .lines()
            .find(|l| l.starts_with("Answered "))
            .unwrap()
            .to_string()
    };
    let mut item = ask();
    item.answer = answer_by(1, Some("Craig"), "14:02");
    assert_eq!(
        details(&item),
        "Answered 14:05 by Craig (decider). Details: outbox/reset-worker/q.json in the Bench folder."
    );
    item.answer = answer_by(1, None, "");
    assert!(details(&item).starts_with("Answered 14:05 by Quicksilver (owner). "));
    item.answer = None;
    assert!(
        render_route(&item, Some(1), "14:07").contains("\nThe answer was cleared at 14:07. Stop")
    );

    item.reply = Some(Reply {
        event_id: hex('8'),
        by: hex('1'),
        by_name: Some("Craig".to_string()),
        role: Some(AnswerRole::Decider),
        at: 1,
        hhmm: "14:03".to_string(),
    });
    assert_eq!(
        render_route_reply(&item, "14:03"),
        format!(
            "Bench reply · reset-worker/q\n\
             Card: Welcome email\n\
             Craig (decider) replied in this thread at 14:03 instead of choosing an option. \
             Read their reply (event {}) in this thread, answer them there, and when it is \
             settled close the card by re-dropping reset-worker/q as a decision or status with \
             the outcome.",
            hex('8')
        )
    );
}
