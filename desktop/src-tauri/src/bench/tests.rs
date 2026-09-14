//! Bench tests. The pure item.rs checks live here now; the fake-relay tick
//! tests join them with mod.rs.

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

// ── mod.rs: the pure/local half of the service ──────────────────────────

use nostr::Keys;
use std::time::{Duration, SystemTime};

#[test]
fn relay_ws_from_admits_only_loopback_in_debug() {
    // cfg(test) builds carry debug_assertions, so the fence is up here.
    assert_eq!(relay_ws_from(None), None);
    assert_eq!(relay_ws_from(Some(RELAY_WS)), None);
    assert_eq!(relay_ws_from(Some("wss://127.0.0.1:1")), None);
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
