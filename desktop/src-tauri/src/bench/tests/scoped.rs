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
