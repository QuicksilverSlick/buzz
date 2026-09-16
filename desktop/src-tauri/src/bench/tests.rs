//! Bench tests: the pure item.rs and mod.rs checks, then the fake-relay
//! tick tests. This file carries the only literal events route in bench/.

use super::item::*;
use super::*;
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
        ("https with port", link("https://github.com:1337/x")),
        // The parser encodes or drops these; the card prints the raw string.
        (
            "https with a space",
            link("https://github.com/ [Approve](https://evil.example)"),
        ),
        (
            "https with a newline",
            link("https://github.com/\nnostr:npub1x @Honey"),
        ),
        ("https with a tab", link("https://github.com/\tx")),
        (
            "https with nsec in the path",
            link("https://github.com/nsec1abc"),
        ),
        (
            "https with @ in the fragment",
            link("https://github.com/x#@Honey"),
        ),
        (
            "https 513 bytes",
            link(&format!("https://github.com/{}", "a".repeat(494))),
        ),
        ("title www.", vec![("title", json!("see www.evil.example"))]),
        (
            "label www.",
            vec![("links", json!([{"label": "www.evil.example", "url": good}]))],
        ),
        ("bidi override", vec![("title", json!("a\u{202E}b"))]),
        (
            "zero width",
            vec![("options", json!(["Stay\u{200B}", "Move"]))],
        ),
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
    assert_eq!(full.deadline.as_deref(), Some("2026-09-20T00:00:00Z"));
    // The deadline is kept canonical, however loosely chrono parsed it.
    let sloppy = format!("2026-09-20 00:00:00.{}Z", "0".repeat(60));
    assert_eq!(
        check(&[("deadline", json!(sloppy))])
            .unwrap()
            .deadline
            .as_deref(),
        Some("2026-09-20T00:00:00Z")
    );
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
         Tap one number below to answer.\n\
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
    let board = render_board(&all, UUID, "14:05", 0);
    assert_eq!(
        board,
        format!(
            "Bench · as of 14:05 · needs you: 3\n\
             To answer: tap one number under a card. To undo or change it, tap that number again first.\n\
             \n\
             Needs you\n\
             - 🔴 Relay choice (dreamforge) → buzz://message?channel={UUID}&id={hex64}\n\
             - 🟠 Installer (dreamforge) (no card yet)\n\
             - 🟠 Phone alerts (dreamforge) (no card yet)\n\
             \n\
             Decisions\n\
             - 🧭 label(https:evil.example) Stay (dreamforge) (decided on claude.ai)\n\
             - 🧭 Own relay later (dreamforge) (claude)\n\
             \n\
             Status\n\
             - ▪ dreamforge: Bench M1 — in-progress"
        )
    );
    assert!(!board.contains("Stale thing") && !board.contains("Old thing"));
    assert!(!board.contains("https://") && !board.contains('['));
    // The clock alone never changes the hash; a line change does.
    assert_eq!(
        board_hash(&board),
        board_hash(&render_board(&all, UUID, "23:59", 0))
    );
    assert_ne!(
        board_hash(&board),
        board_hash(&render_board(&all[1..], UUID, "14:05", 0))
    );
    assert_eq!(board_hash(&board).len(), 16);
}

#[test]
fn option_emoji_are_fully_qualified_keycaps() {
    for (i, e) in OPTION_EMOJI.iter().enumerate() {
        assert_eq!(*e, format!("{}\u{FE0F}\u{20E3}", i + 1));
    }
}

// ── mod.rs: the pure/local half of the service ──────────────────────────

use nostr::Keys;
use std::time::{Duration, SystemTime};

#[test]
fn relay_ws_from_admits_only_loopback_in_debug() {
    // cfg(test) builds carry debug_assertions, so the fence is up here.
    assert_eq!(relay_ws_from(None), None);
    assert_eq!(relay_ws_from(Some(RELAY_WS)), None);
    assert_eq!(relay_ws_from(Some("wss://127.0.0.1:1")), None);
    assert_eq!(relay_ws_from(Some("ws://localhost@evil.example/")), None);
    assert_eq!(relay_ws_from(Some("ws://localhost.evil.example/")), None);
    assert_eq!(
        relay_ws_from(Some("ws://127.0.0.1:1")).as_deref(),
        Some("ws://127.0.0.1:1")
    );
    assert_eq!(
        relay_ws_from(Some("ws://localhost:3000")).as_deref(),
        Some("ws://localhost:3000")
    );
}

#[test]
fn channel_id_is_per_owner_and_the_archive_predicate_matches_it() {
    let a = "aa".repeat(32);
    let b = "bb".repeat(32);
    assert_eq!(channel_id(&a), channel_id(&a));
    assert_ne!(channel_id(&a), channel_id(&b));
    let id = channel_id(&a).to_string();
    assert!(is_bench_channel(&a, &id));
    assert!(is_bench_channel(&a, &format!(" {} ", id.to_uppercase())));
    assert!(!is_bench_channel(&b, &id));
    assert!(!is_bench_channel(&a, UUID));
}

#[test]
fn next_ts_is_strictly_increasing_and_bounded_by_the_clock_lead() {
    let mut s = BenchState::default();
    let now = 1_757_779_500;
    let mut last = 0;
    for _ in 0..200 {
        let t = next_ts(&mut s, now).unwrap().as_secs();
        assert!(t > last);
        last = t;
    }
    assert_eq!(last, now + 199);
    for _ in 200..=600 {
        next_ts(&mut s, now).unwrap();
    }
    assert_eq!(s.last_created_at, now + 600);
    assert!(next_ts(&mut s, now).is_err());
    // Deferring does not move the floor; a wall-clock jump is followed.
    assert_eq!(s.last_created_at, now + 600);
    assert_eq!(
        next_ts(&mut s, now + 10_000).unwrap().as_secs(),
        now + 10_000
    );
}

#[test]
fn owner_tag_binds_the_writer_to_the_owner_for_an_hour() {
    let owner = Keys::generate();
    let writer = Keys::generate();
    let now = 1_757_779_500;
    let tag = owner_tag(&owner, &writer.public_key(), now).unwrap();
    let parts: Vec<String> = serde_json::from_str(&tag).unwrap();
    assert_eq!(parts[0], "auth");
    assert_eq!(parts[1], owner.public_key().to_hex());
    assert_eq!(parts[2], format!("created_at<{}", now + 3600));
    assert_eq!(parts[3].len(), 128);
    assert!(owner_tag(&owner, &owner.public_key(), now).is_err());
}

#[test]
fn writer_profile_carries_no_tags() {
    let ev = crate::events::build_profile(Some("Bench"), Some("bench"), None, Some(ABOUT), None)
        .unwrap()
        .sign_with_keys(&Keys::generate())
        .unwrap();
    assert_eq!(ev.kind.as_u16(), 0);
    assert!(ev.tags.is_empty());
    assert!(!ev.content.contains("auth"));
}

fn kind9(keys: &Keys, created_at: u64, client: Option<&str>) -> nostr::Event {
    let tags: Vec<Vec<String>> = client
        .map(|m| vec!["client".to_string(), m.to_string()])
        .into_iter()
        .collect();
    crate::events::build_message_with_client_tags(
        channel_id(&"aa".repeat(32)),
        "x",
        None,
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        "http://127.0.0.1:1",
        &tags,
    )
    .unwrap()
    .custom_created_at(nostr::Timestamp::from(created_at))
    .sign_with_keys(keys)
    .unwrap()
}

#[test]
fn rebuild_map_keeps_the_newest_per_marker_and_lists_the_rest() {
    let w = Keys::generate();
    let card_old = kind9(&w, 100, Some("bench:card:orchestrator/x@aaaa"));
    let card_new = kind9(&w, 101, Some("bench:card:orchestrator/x@bbbb"));
    let card_y = kind9(&w, 102, Some("bench:card:orchestrator/y@cccc"));
    let board_old = kind9(&w, 103, Some("bench:board@dddd"));
    let board_new = kind9(&w, 104, Some("bench:board@eeee"));
    let untagged = kind9(&w, 105, None);
    let foreign = kind9(&Keys::generate(), 106, Some("bench:board@ffff"));
    let events = [
        &board_new, &card_old, &untagged, &card_y, &board_old, &card_new, &foreign,
    ]
    .map(|e| e.clone());

    let r = rebuild_map(&events, &w.public_key().to_hex());
    assert_eq!(r.cards.len(), 2);
    let x = &r.cards["orchestrator/x"];
    assert_eq!(
        (x.event_id.as_str(), x.created_at, x.hash.as_str(), x.seeded),
        (card_new.id.to_hex().as_str(), 101, "bbbb", usize::MAX)
    );
    assert_eq!(r.cards["orchestrator/y"].hash, "cccc");
    let b = r.board.unwrap();
    assert_eq!(
        (b.event_id, b.hash),
        (board_new.id.to_hex(), "eeee".to_string())
    );
    let mut strays = r.stray_ids;
    strays.sort();
    let mut want = vec![
        card_old.id.to_hex(),
        board_old.id.to_hex(),
        untagged.id.to_hex(),
    ];
    want.sort();
    assert_eq!(strays, want);
}

#[test]
fn save_state_writes_only_on_change_and_load_state_falls_back_to_prev() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state = root.join("state.json");
    let prev = root.join("state.prev.json");
    let mut s = BenchState::default();
    let mut last = String::new();

    save_state(root, &s, &mut last).unwrap();
    let first = std::fs::read(&state).unwrap();
    assert!(!prev.exists());
    // Unchanged snapshot: nothing is written, even with the file gone.
    std::fs::remove_file(&state).unwrap();
    save_state(root, &s, &mut last).unwrap();
    assert!(!state.exists());
    std::fs::write(&state, &first).unwrap();

    s.last_tick = "t1".to_string();
    save_state(root, &s, &mut last).unwrap();
    assert_eq!(std::fs::read(&prev).unwrap(), first);
    assert_eq!(load_state(root).0.last_tick, "t1");

    std::fs::write(&state, b"{not json").unwrap();
    let (loaded, fresh) = load_state(root);
    assert!(!fresh && loaded.last_tick.is_empty() && !loaded.needs_rebuild);

    let (loaded, fresh) = load_state(&root.join("nowhere"));
    assert!(fresh && loaded.needs_rebuild);
}

#[test]
fn intake_takes_good_drops_and_rejects_the_rest_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let inbox = root.join("inbox").join("orchestrator");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::create_dir_all(root.join("inbox").join("Bad Writer")).unwrap();
    let bad = drop_json(&[("answer", json!("x"))]);
    let files = [
        ("orchestrator/good.json", drop_json(&[])),
        ("orchestrator/bad.json", bad.clone()),
        ("orchestrator/big.json", "x".repeat(65 * 1024)),
        ("orchestrator/old.rejected.json", "{}".to_string()),
        ("Bad Writer/z.json", drop_json(&[])),
    ];
    for (name, text) in &files {
        std::fs::write(root.join("inbox").join(name), text).unwrap();
    }
    let now = SystemTime::now();
    // Just-written files may be half written: they wait a tick.
    assert!(scan_inbox(root, now).is_empty());
    let backdate = |name: &str| {
        std::fs::File::options()
            .write(true)
            .open(root.join("inbox").join(name))
            .unwrap()
            .set_modified(now - Duration::from_secs(2))
            .unwrap();
    };
    for (name, _) in &files {
        backdate(name);
    }
    assert_eq!(scan_inbox(root, now).len(), 3);

    let mut s = BenchState::default();
    assert!(intake(&mut s, root, NOW));
    let item = &s.items["orchestrator/good"];
    assert_eq!(
        (item.writer.as_str(), item.updated_at.as_str()),
        ("orchestrator", NOW)
    );
    let good_hash = item.hash.clone();
    assert_eq!(s.items.len(), 1);
    let rejected = |stem: &str| -> Value {
        let text = std::fs::read_to_string(inbox.join(format!("{stem}.rejected.json"))).unwrap();
        serde_json::from_str(&text).unwrap()
    };
    let r = rejected("bad");
    assert!(r["reason"]
        .as_str()
        .unwrap()
        .contains("unknown field `answer`"));
    assert_eq!(r["drop"], bad);
    let r = rejected("big");
    assert_eq!(
        (r["reason"].as_str(), &r["drop"]),
        (Some("too large"), &Value::Null)
    );
    for stem in ["good", "bad", "big"] {
        assert!(!inbox.join(format!("{stem}.json")).exists());
    }
    assert!(inbox.join("old.rejected.json").exists());
    assert!(root.join("inbox/Bad Writer/z.json").exists());
    assert!(!intake(&mut s, root, NOW));

    // An identical re-drop is a no-op; a reworded one keeps the card.
    let card = Posted {
        event_id: "ee".repeat(32),
        created_at: 1,
        hash: good_hash,
        seeded: 2,
    };
    s.items.get_mut("orchestrator/good").unwrap().card = Some(card.clone());
    std::fs::write(inbox.join("good.json"), drop_json(&[])).unwrap();
    backdate("orchestrator/good.json");
    assert!(!intake(&mut s, root, "2026-09-13T15:00:00Z"));
    assert_eq!(s.items["orchestrator/good"].updated_at, NOW);
    std::fs::write(
        inbox.join("good.json"),
        drop_json(&[("title", json!("Relay choice, reworded"))]),
    )
    .unwrap();
    backdate("orchestrator/good.json");
    assert!(intake(&mut s, root, "2026-09-13T15:00:00Z"));
    let item = &s.items["orchestrator/good"];
    assert_eq!(item.card, Some(card));
    assert_ne!(item.hash, item.card.as_ref().unwrap().hash);
    assert!(!inbox.join("good.json").exists());
}

// ── mod.rs: the relay half, driven through an axum loopback ─────────────

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::{routing::post, Json, Router};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Who the fake relay lets write: everyone, members or owner-tagged writers,
/// or members only (the owner is always a member).
#[derive(Clone, Copy, PartialEq)]
enum Gate {
    Open,
    MemberOrTag,
    MembersOnly,
}

struct Relay {
    owner_hex: String,
    gate: Gate,
    /// Every POST in order: the event and its x-auth-tag header, if any.
    log: Mutex<Vec<(Value, Option<String>)>>,
    /// Kinds always answered with the canned `duplicate:` reply.
    force_dup: HashSet<u64>,
    seen: Mutex<HashSet<String>>,
    query_reply: Mutex<Vec<Value>>,
    /// Every /query body in order.
    queries: Mutex<Vec<Value>>,
    /// Event ids the relay no longer has: a kind 5 or 40003 on them is refused.
    gone: Mutex<HashSet<String>>,
}

fn tag_value(ev: &Value, name: &str) -> Option<String> {
    ev["tags"]
        .as_array()?
        .iter()
        .find_map(|t| (t[0] == name).then(|| t[1].as_str().unwrap_or("").to_string()))
}

async fn fake_events(
    State(r): State<Arc<Relay>>,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, Json<Value>) {
    let ev: Value = serde_json::from_str(&body).unwrap();
    let tag = headers
        .get("x-auth-tag")
        .map(|v| v.to_str().unwrap().to_string());
    r.log.lock().unwrap().push((ev.clone(), tag.clone()));
    let member = ev["pubkey"] == r.owner_hex.as_str();
    let admitted = match r.gate {
        Gate::Open => true,
        Gate::MemberOrTag => member || tag.is_some(),
        Gate::MembersOnly => member,
    };
    if !admitted {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "relay_membership_required",
                "message": "You must be a relay member to access this relay",
            })),
        );
    }
    let kind = ev["kind"].as_u64().unwrap();
    let target = tag_value(&ev, "e").unwrap_or_default();
    if matches!(kind, 5 | 40003) && r.gone.lock().unwrap().contains(&target) {
        let message = if kind == 5 {
            "target event not found"
        } else {
            "invalid: edit target event not found"
        };
        let reply = json!({ "event_id": ev["id"], "accepted": false, "message": message });
        return (StatusCode::OK, Json(reply));
    }
    let key = match kind {
        7 => Some(format!("7:{}:{target}:{}", ev["pubkey"], ev["content"])),
        9007 => Some(format!("9007:{}", tag_value(&ev, "h").unwrap_or_default())),
        _ => None,
    };
    let dup = r.force_dup.contains(&kind) || key.is_some_and(|k| !r.seen.lock().unwrap().insert(k));
    let message = match (dup, kind) {
        (false, _) => "",
        (true, 7) => "duplicate: reaction already exists",
        (true, _) => "duplicate: channel already exists",
    };
    let reply = json!({ "event_id": ev["id"], "accepted": !dup, "message": message });
    (StatusCode::OK, Json(reply))
}

/// The reply is filter-agnostic: rebuild_map skips kind != 9 and pick_answer
/// skips kind != 7, so one list serves recover and the poll.
async fn fake_query(State(r): State<Arc<Relay>>, body: String) -> Json<Vec<Value>> {
    r.queries
        .lock()
        .unwrap()
        .push(serde_json::from_str(&body).unwrap());
    Json(r.query_reply.lock().unwrap().clone())
}

/// Serve the fake relay on a loopback port; returns the http base.
async fn fake_relay(relay: Arc<Relay>) -> String {
    let app = Router::new()
        .route("/events", post(fake_events))
        .route("/query", post(fake_query))
        .with_state(relay);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}")
}

/// One Bench under test: a TempDir root, fresh owner and writer keys, a fake
/// relay and a clock that moves 2 s per tick.
struct Bench {
    _dir: tempfile::TempDir,
    root: PathBuf,
    ctx: Ctx<'static>,
    relay: Arc<Relay>,
    s: BenchState,
    now: u64,
}

impl Bench {
    async fn new(gate: Gate, force_dup: &[u64]) -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("inbox")).unwrap();
        let owner = Keys::generate();
        let writer = Keys::generate();
        let relay = Arc::new(Relay {
            owner_hex: owner.public_key().to_hex(),
            gate,
            log: Mutex::new(Vec::new()),
            force_dup: force_dup.iter().copied().collect(),
            seen: Mutex::new(HashSet::new()),
            query_reply: Mutex::new(Vec::new()),
            queries: Mutex::new(Vec::new()),
            gone: Mutex::new(HashSet::new()),
        });
        let base = fake_relay(relay.clone()).await;
        let state: &'static AppState = Box::leak(Box::new(crate::app_state::build_app_state()));
        let ctx = Ctx {
            state,
            base,
            owner,
            writer,
            root: root.clone(),
        };
        let (s, fresh) = load_state(&root);
        assert!(fresh);
        Bench {
            _dir: dir,
            root,
            ctx,
            relay,
            s,
            now: 1_757_779_500,
        }
    }

    fn owner_hex(&self) -> String {
        self.ctx.owner.public_key().to_hex()
    }

    fn writer_hex(&self) -> String {
        self.ctx.writer.public_key().to_hex()
    }

    fn channel(&self) -> String {
        channel_id(&self.owner_hex()).to_string()
    }

    /// Write a drop and backdate it past the half-written window.
    fn drop_file(&self, writer: &str, stem: &str, text: &str) {
        let dir = self.root.join("inbox").join(writer);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{stem}.json"));
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(2))
            .unwrap();
    }

    async fn tick(&mut self) -> Result<(), String> {
        self.now += 2;
        tick(&self.ctx, &mut self.s, self.now, "14:05", NOW).await
    }

    async fn settle(&mut self, ticks: usize) {
        for _ in 0..ticks {
            self.tick().await.unwrap();
        }
    }

    fn posts(&self) -> Vec<(Value, Option<String>)> {
        self.relay.log.lock().unwrap().clone()
    }

    fn kinds(&self) -> Vec<u64> {
        self.posts()
            .iter()
            .map(|(e, _)| e["kind"].as_u64().unwrap())
            .collect()
    }

    fn card(&self, id: &str) -> Posted {
        self.s.items[id].card.clone().unwrap()
    }
}

/// A bench with one pending drop (2 options) and one extra member, settled.
async fn seeded_bench(gate: Gate, force_dup: &[u64]) -> Bench {
    let mut b = Bench::new(gate, force_dup).await;
    b.drop_file("orchestrator", "relay-choice", &drop_json(&[]));
    std::fs::write(
        b.root.join("members.txt"),
        format!("# phone\n{}\nnot a key\n", "cd".repeat(32)),
    )
    .unwrap();
    b.settle(6).await;
    b
}

#[tokio::test]
async fn one_tick_posts_canary_channel_member_card_seeds_board() {
    let b = seeded_bench(Gate::Open, &[]).await;
    let posts = b.posts();
    assert_eq!(b.kinds(), [0, 9007, 9000, 9000, 9, 7, 7, 9]);
    assert!(posts.iter().all(|(_, tag)| tag.is_none()));
    let (owner, writer, ch) = (b.owner_hex(), b.writer_hex(), b.channel());
    let by = |i: usize| posts[i].0["pubkey"].as_str().unwrap().to_string();
    assert_eq!(by(0), writer);
    for i in 1..4 {
        assert_eq!(by(i), owner, "post {i} is owner-signed");
    }
    let e = |i: usize| &posts[i].0;
    assert_eq!(tag_value(e(1), "name").as_deref(), Some("bench"));
    assert_eq!(tag_value(e(1), "visibility").as_deref(), Some("private"));
    assert_eq!(tag_value(e(1), "channel_type").as_deref(), Some("stream"));
    assert_eq!(tag_value(e(1), "h").as_deref(), Some(ch.as_str()));
    assert_eq!(tag_value(e(2), "p").as_deref(), Some(writer.as_str()));
    assert_eq!(
        tag_value(e(3), "p").as_deref(),
        Some("cd".repeat(32).as_str())
    );

    let item = &b.s.items["orchestrator/relay-choice"];
    let card = e(4);
    assert_eq!(by(4), writer);
    assert_eq!(tag_value(card, "h").as_deref(), Some(ch.as_str()));
    assert_eq!(tag_value(card, "p").as_deref(), Some(owner.as_str()));
    assert_eq!(tag_value(card, "e"), None);
    assert_eq!(
        tag_value(card, "client").unwrap(),
        format!("bench:card:orchestrator/relay-choice@{}", card_hash(item))
    );
    let card_id = card["id"].as_str().unwrap();
    for (i, emoji) in [(5, OPTION_EMOJI[0]), (6, OPTION_EMOJI[1])] {
        assert_eq!(tag_value(e(i), "e").as_deref(), Some(card_id));
        assert_eq!(e(i)["content"], emoji);
    }
    let board = e(7);
    assert_eq!(by(7), writer);
    assert!(tag_value(board, "client")
        .unwrap()
        .starts_with("bench:board@"));
    assert_eq!(tag_value(board, "p"), None);
    assert!(board["content"]
        .as_str()
        .unwrap()
        .contains(&format!("buzz://message?channel={ch}&id={card_id}")));

    let writer_ts: Vec<u64> = posts
        .iter()
        .filter(|(e, _)| e["pubkey"] == writer.as_str())
        .map(|(e, _)| e["created_at"].as_u64().unwrap())
        .collect();
    assert!(writer_ts.windows(2).all(|w| w[1] > w[0]), "{writer_ts:?}");

    assert!(!b.root.join("inbox/orchestrator/relay-choice.json").exists());
    assert_eq!(b.card("orchestrator/relay-choice").event_id, card_id);
    assert_eq!(b.card("orchestrator/relay-choice").seeded, 2);
    assert_eq!(
        b.s.board.as_ref().unwrap().event_id,
        board["id"].as_str().unwrap()
    );
    assert_eq!(b.s.admission, Admission::Open);
    assert_eq!(b.s.members_added, [writer, "cd".repeat(32)]);
    assert!(b.s.profile_published && b.s.channel_created && !b.s.needs_rebuild);
    assert_eq!(b.s.last_tick, NOW);
}

#[tokio::test]
async fn resync_is_idempotent() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let before = b.posts().len();
    b.drop_file("orchestrator", "relay-choice", &drop_json(&[]));
    b.settle(2).await;
    assert_eq!(b.posts().len(), before);
    assert!(!b.root.join("inbox/orchestrator/relay-choice.json").exists());
}

#[tokio::test]
async fn pending_reword_deletes_and_reposts_never_edits() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old_card = b.card("orchestrator/relay-choice");
    let old_board = b.s.board.clone().unwrap();
    let before = b.posts().len();
    b.drop_file(
        "orchestrator",
        "relay-choice",
        &drop_json(&[("options", json!(["Stay", "Move away"]))]),
    );
    b.settle(3).await;
    let posts = b.posts()[before..].to_vec();
    let kinds: Vec<u64> = posts
        .iter()
        .map(|(e, _)| e["kind"].as_u64().unwrap())
        .collect();
    assert_eq!(kinds, [5, 9, 7, 7, 9, 5]);
    assert_eq!(tag_value(&posts[0].0, "e").unwrap(), old_card.event_id);
    let new_card = b.card("orchestrator/relay-choice");
    assert_ne!(new_card.hash, old_card.hash);
    assert_eq!(
        tag_value(&posts[1].0, "client").unwrap(),
        format!("bench:card:orchestrator/relay-choice@{}", new_card.hash)
    );
    assert_eq!(tag_value(&posts[5].0, "e").unwrap(), old_board.event_id);
    assert!(!b.kinds().contains(&40003));
}

#[tokio::test]
async fn status_change_edits_board_only() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let status = |state: &str| {
        drop_json(&[
            ("kind", json!("status")),
            ("state", json!(state)),
            ("title", json!("Bench M1")),
            ("severity", Value::Null),
            ("options", Value::Null),
        ])
    };
    b.drop_file("orchestrator", "m1", &status("in-progress"));
    b.settle(2).await;
    let before = b.posts().len();
    b.drop_file("orchestrator", "m1", &status("done"));
    b.settle(2).await;
    let posts = b.posts()[before..].to_vec();
    assert_eq!(posts.len(), 1);
    let edit = &posts[0].0;
    assert_eq!(edit["kind"], 40003);
    assert_eq!(tag_value(edit, "h").unwrap(), b.channel());
    assert_eq!(
        tag_value(edit, "e").unwrap(),
        b.s.board.as_ref().unwrap().event_id
    );
    assert!(edit["content"]
        .as_str()
        .unwrap()
        .contains("Bench M1 — done"));
    assert_eq!(
        b.s.board.as_ref().unwrap().hash,
        board_hash(edit["content"].as_str().unwrap())
    );
}

#[tokio::test]
async fn membership_403_triggers_lazy_tag_then_denied() {
    let mut b = Bench::new(Gate::MemberOrTag, &[]).await;
    b.drop_file("orchestrator", "relay-choice", &drop_json(&[]));
    b.settle(3).await;
    let posts = b.posts();
    assert_eq!(b.kinds()[..3], [0, 0, 9007]);
    assert_eq!(posts[0].1, None);
    let tag = posts[1].1.clone().expect("the retry carries the tag");
    assert_eq!(posts[0].0["id"], posts[1].0["id"]);
    let now = posts[1].0["created_at"].as_u64().unwrap();
    let parsed = buzz_sdk_pkg::nip_oa::parse_auth_tag(&tag).unwrap();
    assert_eq!(parsed.as_slice()[1], b.owner_hex());
    assert_eq!(parsed.as_slice()[2], format!("created_at<{}", now + 3600));
    let bound = now + 3600;
    let writer_pk = b.ctx.writer.public_key();
    assert!(buzz_sdk_pkg::nip_oa::verify_auth_tag_for_auth_event(&tag, &writer_pk, now).is_ok());
    assert!(buzz_sdk_pkg::nip_oa::verify_auth_tag_for_auth_event(&tag, &writer_pk, bound).is_err());
    assert_eq!(b.s.admission, Admission::ViaOwner);
    // After the 403, every writer request is tagged and no owner request is.
    for (e, tag) in &posts[1..] {
        let owner_signed = e["pubkey"] == b.owner_hex().as_str();
        assert_eq!(tag.is_none(), owner_signed, "kind {}", e["kind"]);
    }
    assert!(posts.iter().any(|(e, _)| e["kind"] == 9));

    let mut denied = Bench::new(Gate::MembersOnly, &[]).await;
    let err = denied.tick().await.unwrap_err();
    assert!(err.contains(&denied.writer_hex()), "{err}");
    assert_eq!(denied.s.admission, Admission::Denied);
    assert_eq!(denied.kinds(), [0, 0]);
    assert!(!denied.s.profile_published);
}

#[tokio::test]
async fn duplicate_reaction_counts_as_success() {
    let b = seeded_bench(Gate::Open, &[7]).await;
    assert_eq!(b.kinds(), [0, 9007, 9000, 9000, 9, 7, 7, 9]);
    assert_eq!(b.card("orchestrator/relay-choice").seeded, 2);
    assert_eq!(b.s.last_error, None);
}

#[tokio::test]
async fn duplicate_channel_counts_as_success() {
    let b = seeded_bench(Gate::Open, &[9007]).await;
    assert_eq!(b.kinds(), [0, 9007, 9000, 9000, 9, 7, 7, 9]);
    assert!(b.s.channel_created);
}

#[tokio::test]
async fn recover_from_query_rebuilds_cards_and_deletes_nothing() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    let w = b.ctx.writer.clone();
    let now = b.now;
    let item = check(&[]).unwrap();
    let card = kind9(
        &w,
        now - 50,
        Some(&format!("bench:card:{}@{}", item.id, card_hash(&item))),
    );
    let unknown = kind9(&w, now - 40, Some("bench:card:orchestrator/gone@abcd"));
    let y = check(&[("id", json!("y")), ("options", json!(["Only"]))]).unwrap();
    let card_y = kind9(
        &w,
        now - 35,
        Some(&format!("bench:card:{}@{}", y.id, card_hash(&y))),
    );
    let board_old = kind9(&w, now - 30, Some("bench:board@dddd"));
    let board_new = kind9(&w, now - 20, Some("bench:board@eeee"));
    let untagged = kind9(&w, now - 10, None);
    let forged = kind9(&Keys::generate(), now - 5, Some("bench:board@ffff"));
    *b.relay.query_reply.lock().unwrap() = [
        &card, &unknown, &card_y, &board_old, &board_new, &untagged, &forged,
    ]
    .iter()
    .map(|e| serde_json::to_value(e).unwrap())
    .collect();
    // State that knew the item but lost its card (a `duplicate:` on repost),
    // and one whose card it still tracks (a rebuild at launch).
    b.s.items.insert(item.id.clone(), item.clone());
    let tracked = Posted {
        event_id: card_y.id.to_hex(),
        created_at: now - 35,
        hash: card_hash(&y),
        seeded: 1,
    };
    b.s.items.insert(
        y.id.clone(),
        Item {
            card: Some(tracked.clone()),
            ..y.clone()
        },
    );
    assert!(b.s.needs_rebuild);
    b.settle(2).await;

    assert_eq!(b.card(&item.id).event_id, card.id.to_hex());
    assert_eq!(b.card(&item.id).seeded, usize::MAX);
    assert_eq!(b.card(&y.id), tracked);
    assert_eq!(
        b.s.orphan_cards["orchestrator/gone"].event_id,
        unknown.id.to_hex()
    );
    assert_eq!(b.s.strays, [board_old.id.to_hex(), untagged.id.to_hex()]);
    assert!(!b.s.needs_rebuild);
    // The writer's own re-add stamps a member_joined row above the recovered
    // board, so the board is reposted and the recovered one retired; no card
    // is reposted and no orphan or stray is deleted.
    assert_eq!(b.kinds(), [0, 9007, 9000, 9, 5]);
    let posts = b.posts();
    assert_eq!(tag_value(&posts[4].0, "e").unwrap(), board_new.id.to_hex());
    let board = b.s.board.clone().unwrap();
    assert_eq!(board.event_id, posts[3].0["id"].as_str().unwrap());
    assert_ne!(board.hash, "eeee");
}

#[tokio::test]
async fn truncated_rebuild_is_reported() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    let stray = serde_json::to_value(kind9(&b.ctx.writer, b.now - 10, None)).unwrap();
    *b.relay.query_reply.lock().unwrap() = vec![stray; RECOVER_LIMIT as usize];
    let err = b.tick().await.unwrap_err();
    assert!(err.contains("rebuild truncated"), "{err}");
    // The partial map is kept and the next tick carries on.
    assert!(!b.s.needs_rebuild);
    assert_eq!(b.s.strays.len(), RECOVER_LIMIT as usize);
    b.tick().await.unwrap();
}

#[tokio::test]
async fn vanished_targets_are_dropped_not_retried() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    // The owner deleted the board by hand: the text-only edit finds no target,
    // so the board is reposted instead of the edit retrying every tick.
    let old_board = b.s.board.clone().unwrap();
    b.relay
        .gone
        .lock()
        .unwrap()
        .insert(old_board.event_id.clone());
    let before = b.posts().len();
    b.drop_file(
        "orchestrator",
        "m1",
        &drop_json(&[
            ("kind", json!("status")),
            ("state", json!("done")),
            ("title", json!("Bench M1")),
            ("severity", Value::Null),
            ("options", Value::Null),
        ]),
    );
    b.settle(2).await;
    assert_eq!(b.kinds()[before..], [40003, 9]);
    assert_ne!(b.s.board.as_ref().unwrap().event_id, old_board.event_id);
    // A hard-purged card: the retiring kind 5 is refused, the repost goes on.
    let old_card = b.card("orchestrator/relay-choice");
    b.relay
        .gone
        .lock()
        .unwrap()
        .insert(old_card.event_id.clone());
    let before = b.posts().len();
    b.drop_file(
        "orchestrator",
        "relay-choice",
        &drop_json(&[("options", json!(["Stay", "Go"]))]),
    );
    b.settle(3).await;
    assert_eq!(b.kinds()[before..], [5, 9, 7, 7, 9, 5]);
    assert_ne!(
        b.card("orchestrator/relay-choice").event_id,
        old_card.event_id
    );
}

#[tokio::test]
async fn member_added_later_reposts_the_board() {
    let mut b = seeded_bench(Gate::Open, &[]).await;
    let old_board = b.s.board.clone().unwrap();
    let before = b.posts().len();
    std::fs::write(b.root.join("members.txt"), "ef".repeat(32)).unwrap();
    b.settle(2).await;
    let posts = b.posts()[before..].to_vec();
    // The relay stamps a member_joined row after the 9000; the board follows.
    assert_eq!(b.kinds()[before..], [9000, 9, 5]);
    assert_eq!(
        tag_value(&posts[0].0, "p").as_deref(),
        Some("ef".repeat(32).as_str())
    );
    assert_eq!(tag_value(&posts[2].0, "e").unwrap(), old_board.event_id);
}

#[tokio::test]
async fn a_tick_never_exceeds_the_post_budget() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    let one = |opt: &str| drop_json(&[("options", json!([opt]))]);
    b.drop_file("orchestrator", "relay-choice", &one("Stay"));
    b.settle(4).await;
    let before = b.posts().len();
    b.drop_file("orchestrator", "relay-choice", &one("Go"));
    for _ in 0..3 {
        let n = b.posts().len();
        b.tick().await.unwrap();
        assert!(b.posts().len() - n <= MAX_POSTS_PER_TICK);
    }
    // Delete, card, seed; then the board and its predecessor's delete as a pair.
    assert_eq!(b.kinds()[before..], [5, 9, 7, 9, 5]);
}

#[tokio::test]
async fn malformed_drop_is_rejected_in_place() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    b.drop_file("orchestrator", "bad", &drop_json(&[("answer", json!("x"))]));
    b.drop_file("orchestrator", "big", &"x".repeat(65 * 1024));
    b.settle(2).await;
    let inbox = b.root.join("inbox/orchestrator");
    for stem in ["bad", "big"] {
        assert!(!inbox.join(format!("{stem}.json")).exists());
        assert!(inbox.join(format!("{stem}.rejected.json")).exists());
    }
    let bad: Value =
        serde_json::from_str(&std::fs::read_to_string(inbox.join("bad.rejected.json")).unwrap())
            .unwrap();
    assert!(bad["reason"]
        .as_str()
        .unwrap()
        .contains("unknown field `answer`"));
    let big: Value =
        serde_json::from_str(&std::fs::read_to_string(inbox.join("big.rejected.json")).unwrap())
            .unwrap();
    assert_eq!(
        (big["reason"].as_str(), &big["drop"]),
        (Some("too large"), &Value::Null)
    );
    assert!(b.s.items.is_empty());
    // Bootstrap and the empty board only: no card, seed or delete.
    assert_eq!(b.kinds(), [0, 9007, 9000, 9]);
    assert!(tag_value(&b.posts()[3].0, "client")
        .unwrap()
        .starts_with("bench:board@"));
}

#[tokio::test]
async fn owner_change_refuses_to_tick() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    b.s.owner_pubkey = "bb".repeat(32);
    let err = b.tick().await.unwrap_err();
    assert!(err.contains("owner identity changed"), "{err}");
    assert!(b.posts().is_empty());
}

#[test]
fn archive_guard_predicate() {
    let owner = "aa".repeat(32);
    assert!(is_bench_channel(&owner, &channel_id(&owner).to_string()));
    assert!(!is_bench_channel(
        &"bb".repeat(32),
        &channel_id(&owner).to_string()
    ));
}

/// Every drop the exporter (scripts/bench/export-board.py) wrote must pass
/// the trust boundary. Needs a board dump, so it is opt-in:
/// `BENCH_EXPORT_DIR=<inbox/claude-ai> cargo test -- --ignored exported_board`.
#[test]
#[ignore]
fn exported_board_drops_all_validate() {
    let dir = std::env::var("BENCH_EXPORT_DIR").expect("BENCH_EXPORT_DIR");
    let (mut total, mut rejected) = (0, Vec::new());
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        total += 1;
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let raw = std::fs::read_to_string(&path).unwrap();
        if let Err(e) = validate(MIGRATION_WRITER, &stem, &raw, NOW) {
            rejected.push(format!("{stem}: {e}"));
        }
    }
    assert!(total > 0, "no drops under {dir}");
    assert!(
        rejected.is_empty(),
        "{} of {total} rejected:\n{}",
        rejected.len(),
        rejected.join("\n")
    );
    println!("{total} drops validated");
}

mod answers;
