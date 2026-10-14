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
