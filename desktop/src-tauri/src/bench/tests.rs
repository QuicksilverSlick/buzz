//! Bench tests. The pure item.rs checks live here now; the fake-relay tick
//! tests join them with mod.rs.

use super::item::*;
use super::Posted;
use serde_json::{json, Value};

const NOW: &str = "2026-09-13T14:05:00Z";
const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";

/// Minimal pending drop with `overrides` applied; a `Value::Null` override
/// removes the key.
fn drop_json(overrides: &[(&str, Value)]) -> String {
    let mut d = json!({
        "kind": "pending",
        "title": "Relay choice",
        "body": "Stay or move?",
        "area": "dreamforge",
        "severity": "critical",
        "options": ["Stay", "Move"],
    });
    for (k, v) in overrides {
        if v.is_null() {
            d.as_object_mut().unwrap().remove(*k);
        } else {
            d[*k] = v.clone();
        }
    }
    d.to_string()
}

fn check(overrides: &[(&str, Value)]) -> Result<Item, String> {
    validate("orchestrator", "relay-choice", &drop_json(overrides), NOW)
}

fn link(url: &str) -> Vec<(&'static str, Value)> {
    vec![("links", json!([{"label": "plan", "url": url}]))]
}

fn buzz(uuid: &str, id: &str) -> String {
    format!("buzz://message?channel={uuid}&id={id}")
}

#[test]
fn validate_rejects_bad_drops() {
    let hex64 = "ab".repeat(32);
    let good_owned = buzz(UUID, &hex64);
    let good = good_owned.as_str();
    let encrypted = ["ncrypt", "sec1"].concat();
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("unknown field answer", vec![("answer", json!("x"))]),
        ("unknown field verdict", vec![("verdict", json!("x"))]),
        ("unknown field source", vec![("source", json!("x"))]),
        ("unknown field version", vec![("version", json!(1))]),
        ("unknown field card", vec![("card", json!({}))]),
        ("unknown field updatedAt", vec![("updatedAt", json!(NOW))]),
        (
            "decidedBy you under orchestrator",
            vec![("kind", json!("decision")), ("decidedBy", json!("you"))],
        ),
        (
            "decidedBy orchestrator under orchestrator",
            vec![
                ("kind", json!("decision")),
                ("decidedBy", json!("orchestrator")),
            ],
        ),
        (
            "decidedBy someone else",
            vec![("kind", json!("decision")), ("decidedBy", json!("craig"))],
        ),
        (
            "decision without decidedBy",
            vec![("kind", json!("decision"))],
        ),
        (
            "decidedBy on a pending",
            vec![("decidedBy", json!("claude"))],
        ),
        ("private", vec![("private", json!(true))]),
        ("enum typo kind", vec![("kind", json!("pendng"))]),
        ("enum typo severity", vec![("severity", json!("urgent"))]),
        ("enum typo validity", vec![("validity", json!("stale"))]),
        ("enum typo needs", vec![("needs", json!("me"))]),
        (
            "enum typo state",
            vec![("kind", json!("status")), ("state", json!("doing"))],
        ),
        ("status without state", vec![("kind", json!("status"))]),
        ("pending without severity", vec![("severity", Value::Null)]),
        ("pending without options", vec![("options", json!([]))]),
        ("title 121 chars", vec![("title", json!("t".repeat(121)))]),
        ("body 4001 chars", vec![("body", json!("b".repeat(4001)))]),
        (
            "5 options",
            vec![("options", json!(["a", "b", "c", "d", "e"]))],
        ),
        (
            "option 81 chars",
            vec![("options", json!(["o".repeat(81)]))],
        ),
        (
            "5 links",
            vec![(
                "links",
                Value::Array(vec![json!({"label": "l", "url": good}); 5]),
            )],
        ),
        ("http", link("http://github.com/x")),
        ("https host not allowlisted", link("https://evil.example/x")),
        ("https with userinfo", link("https://user@github.com/x")),
        (
            "buzz with path",
            link(&format!("buzz://message/x?channel={UUID}&id={hex64}")),
        ),
        ("buzz with fragment", link(&format!("{good}#f"))),
        ("buzz extra param", link(&format!("{good}&x=1"))),
        ("buzz duplicate param", link(&format!("{good}&id={hex64}"))),
        (
            "buzz non-rfc variant uuid",
            link(&buzz("123e4567-e89b-12d3-c456-426614174000", &hex64)),
        ),
        ("buzz 63-hex id", link(&buzz(UUID, &"a".repeat(63)))),
        (
            "link label with brackets",
            vec![("links", json!([{"label": "[x]", "url": good}]))],
        ),
        (
            "deadline not rfc3339",
            vec![("deadline", json!("tomorrow"))],
        ),
        ("nsec1", vec![("body", json!("key NSEC1abc"))]),
        ("nostr:", vec![("title", json!("see nostr:npub1x"))]),
        ("encrypted key prefix", vec![("body", json!(encrypted))]),
        ("control char", vec![("title", json!("a\u{7}b"))]),
        ("area uppercase", vec![("area", json!("Dreamforge"))]),
        ("id uppercase", vec![("id", json!("Relay"))]),
        ("id slash", vec![("id", json!("a/b"))]),
    ];
    for (name, overrides) in cases {
        assert!(check(&overrides).is_err(), "{name} should be rejected");
    }
    for writer in ["Orchestrator", "a/b", "-x", ""] {
        assert!(
            validate(writer, "x", &drop_json(&[]), NOW).is_err(),
            "writer {writer:?}"
        );
    }
    for stem in ["X", "a/b"] {
        assert!(
            validate("orchestrator", stem, &drop_json(&[]), NOW).is_err(),
            "stem {stem:?}"
        );
    }
}

#[test]
fn validate_accepts_minimal_drops_with_defaults() {
    let p = check(&[]).unwrap();
    assert_eq!(p.id, "orchestrator/relay-choice");
    assert_eq!(p.writer, "orchestrator");
    assert_eq!(p.kind, ItemKind::Pending);
    assert_eq!(p.validity, Validity::Current);
    assert_eq!(p.needs, Needs::You);
    assert_eq!(p.order, 0);
    assert_eq!(p.updated_at, NOW);
    assert!(p.card.is_none() && p.links.is_empty() && p.deadline.is_none());
    assert_eq!(p.hash.len(), 16);
    assert_eq!(
        check(&[("id", json!("other"))]).unwrap().id,
        "orchestrator/other"
    );

    let d = check(&[
        ("kind", json!("decision")),
        ("decidedBy", json!("claude")),
        ("severity", Value::Null),
        ("options", Value::Null),
    ])
    .unwrap();
    assert_eq!(d.kind, ItemKind::Decision);
    assert_eq!(d.decided_by.as_deref(), Some("claude"));
    let s = check(&[
        ("kind", json!("status")),
        ("state", json!("in-progress")),
        ("severity", Value::Null),
        ("options", Value::Null),
    ])
    .unwrap();
    assert_eq!(s.state, Some(StatusState::InProgress));
    // Only the migration import may carry the owner's own decisions.
    for by in ["you", "orchestrator"] {
        let raw = drop_json(&[("kind", json!("decision")), ("decidedBy", json!(by))]);
        assert!(
            validate("claude-ai", "x", &raw, NOW).is_ok(),
            "decidedBy {by} under claude-ai"
        );
    }
    let full = check(&[
        ("deadline", json!("2026-09-20T00:00:00Z")),
        (
            "links",
            json!([
                {"label": "plan", "url": "https://github.com/QuicksilverSlick/buzz"},
                {"label": "card", "url": buzz(UUID, &"ab".repeat(32))},
            ]),
        ),
        ("needs", json!("delegable")),
        ("validity", json!("suspected-stale")),
        ("order", json!(3)),
    ])
    .unwrap();
    assert_eq!(full.needs, Needs::Delegable);
    assert_eq!(full.validity, Validity::SuspectedStale);
    assert_eq!(full.order, 3);
    assert_eq!(full.links.len(), 2);
}

#[test]
fn ids_are_namespaced_by_writer() {
    for w in ["a", "b"] {
        assert_eq!(
            validate(w, "x", &drop_json(&[]), NOW).unwrap().id,
            format!("{w}/x")
        );
    }
}

#[test]
fn version_hash_ignores_key_order_id_and_updated_at() {
    let a = validate(
        "w",
        "one",
        r#"{"id":"one","kind":"pending","title":"T","body":"B","area":"a","severity":"info","options":["x","y"]}"#,
        NOW,
    )
    .unwrap();
    let b = validate(
        "w",
        "two",
        r#"{"options":["x","y"],"severity":"info","area":"a","body":"B","title":"T","kind":"pending","id":"two"}"#,
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    assert_eq!(a.hash, b.hash);
    let c = validate(
        "w",
        "one",
        r#"{"kind":"pending","title":"T","body":"B","area":"a","severity":"info","options":["x","z"]}"#,
        NOW,
    )
    .unwrap();
    assert_ne!(a.hash, c.hash);
}

#[test]
fn fence_outgrows_the_longest_backtick_run() {
    for (text, len) in [
        ("plain", 3),
        ("a ``` b", 4),
        ("a ````` b", 6),
        ("``x`` ```` `", 5),
    ] {
        let out = fence(text);
        let f = "`".repeat(len);
        let inner = out
            .strip_prefix(&format!("{f}text\n"))
            .unwrap()
            .strip_suffix(&format!("\n{f}"))
            .unwrap();
        assert_eq!(inner, text);
        let longest = inner.split(|c| c != '`').map(str::len).max().unwrap();
        assert!(longest < len, "{text:?}");
    }
}

#[test]
fn summary_cuts_on_a_char_boundary() {
    assert_eq!(summary("short"), "short");
    let exact = "é".repeat(400);
    assert_eq!(summary(&exact), exact);
    let s = summary(&"é".repeat(401));
    assert_eq!(s.chars().count(), 401);
    assert!(s.starts_with(&exact) && s.ends_with('…'));
}

#[test]
fn render_card_golden() {
    let item = check(&[
        ("deadline", json!("2026-09-20T00:00:00Z")),
        (
            "links",
            json!([{"label": "plan", "url": "https://github.com/QuicksilverSlick/buzz"}]),
        ),
    ])
    .unwrap();
    assert_eq!(
        render_card(&item, "14:05"),
        "🔴 · dreamforge · orchestrator\n\
         ```text\n\
         Relay choice\n\
         \n\
         Stay or move?\n\
         \n\
         1. Stay\n\
         2. Move\n\
         ```\n\
         due 2026-09-20T00:00:00Z\n\
         plan: https://github.com/QuicksilverSlick/buzz\n\
         as of 14:05"
    );
}

#[test]
fn render_card_keeps_mentions_inside_the_fence() {
    // validate() refuses these strings; a card built from state must still
    // keep them where neither app resolves them.
    let mut item = check(&[]).unwrap();
    item.title = "@Honey nostr:npub1abc".to_string();
    item.body = "ping @Honey nostr:npub1abc".to_string();
    let card = render_card(&item, "14:05");
    let (head, rest) = card.split_once("```text\n").unwrap();
    let (inner, tail) = rest.split_once("\n```").unwrap();
    assert!(inner.contains("@Honey nostr:npub1abc"));
    for outside in [head, tail] {
        assert!(
            !outside.contains('@') && !outside.contains("nostr:"),
            "{outside:?}"
        );
    }
}

#[test]
fn ping_owner_only_for_fresh_needs_you() {
    assert!(ping_owner(&check(&[]).unwrap()));
    assert!(!ping_owner(
        &check(&[("needs", json!("delegable"))]).unwrap()
    ));
    assert!(!ping_owner(
        &check(&[("validity", json!("suspected-stale"))]).unwrap()
    ));
    assert!(!ping_owner(
        &check(&[("kind", json!("status")), ("state", json!("done"))]).unwrap()
    ));
    assert!(!ping_owner(
        &validate("claude-ai", "x", &drop_json(&[]), NOW).unwrap()
    ));
}

#[test]
fn render_board_orders_links_and_folds() {
    let hex64 = "ab".repeat(32);
    let mut crit = check(&[("id", json!("crit")), ("order", json!(9))]).unwrap();
    crit.card = Some(Posted {
        event_id: hex64.clone(),
        created_at: 1,
        hash: crit.hash.clone(),
        seeded: 0,
    });
    let att = check(&[
        ("id", json!("att")),
        ("title", json!("Phone alerts")),
        ("severity", json!("attention")),
        ("order", json!(1)),
    ])
    .unwrap();
    let att0 = check(&[
        ("id", json!("att0")),
        ("title", json!("Installer")),
        ("severity", json!("attention")),
        ("order", json!(0)),
    ])
    .unwrap();
    let stale = check(&[
        ("id", json!("stale")),
        ("title", json!("Stale thing")),
        ("validity", json!("suspected-stale")),
    ])
    .unwrap();
    let old = check(&[
        ("id", json!("old")),
        ("title", json!("Old thing")),
        ("validity", json!("superseded")),
    ])
    .unwrap();
    let imported = validate(
        "claude-ai",
        "dec",
        &drop_json(&[
            ("kind", json!("decision")),
            ("decidedBy", json!("you")),
            ("title", json!("[label](https://evil.example) Stay")),
        ]),
        NOW,
    )
    .unwrap();
    let dec = check(&[
        ("id", json!("dec2")),
        ("kind", json!("decision")),
        ("decidedBy", json!("claude")),
        ("title", json!("Own relay later")),
    ])
    .unwrap();
    let status = check(&[
        ("id", json!("st")),
        ("kind", json!("status")),
        ("state", json!("in-progress")),
        ("title", json!("Bench M1")),
    ])
    .unwrap();
    let all = [&att, &stale, &imported, &crit, &old, &att0, &status, &dec];
    let board = render_board(&all, UUID, "14:05");
    assert_eq!(
        board,
        format!(
            "Bench · as of 14:05 · 3 need you\n\
             Needs you\n\
             1. 🔴 Relay choice (dreamforge) → buzz://message?channel={UUID}&id={hex64}\n\
             2. 🟠 Installer (dreamforge) (no card yet)\n\
             3. 🟠 Phone alerts (dreamforge) (no card yet)\n\
             Decisions\n\
             - 🧭 label(https:evil.example) Stay (dreamforge) (decided on claude.ai)\n\
             - 🧭 Own relay later (dreamforge) (claude)\n\
             Status\n\
             - ▪ dreamforge: Bench M1 — in-progress"
        )
    );
    assert!(!board.contains("Stale thing") && !board.contains("Old thing"));
    assert!(!board.contains("https://") && !board.contains('['));
    // The clock alone never changes the hash; a line change does.
    assert_eq!(
        board_hash(&board),
        board_hash(&render_board(&all, UUID, "23:59"))
    );
    assert_ne!(
        board_hash(&board),
        board_hash(&render_board(&all[1..], UUID, "14:05"))
    );
    assert_eq!(board_hash(&board).len(), 16);
}

#[test]
fn option_emoji_are_fully_qualified_keycaps() {
    for (i, e) in OPTION_EMOJI.iter().enumerate() {
        assert_eq!(*e, format!("{}\u{FE0F}\u{20E3}", i + 1));
    }
}
