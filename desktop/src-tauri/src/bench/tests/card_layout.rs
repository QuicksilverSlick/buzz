//! One decision-card layout on every surface: the header names the project,
//! and the labels a writer types at the start of a body line print bold, a
//! paragraph each, with the writer's text as inert as ever. Child of
//! tests.rs for `check`.

use super::*;
use crate::bench::item::prose;
use crate::bench::plain::{body, place};

const F: char = '\u{FEFF}';

fn vectors() -> Value {
    serde_json::from_str(include_str!("../card_layout_vectors.json")).unwrap()
}

fn card_of(body_text: &str) -> String {
    render_card(&check(&[("body", json!(body_text))]).unwrap(), "14:05")
}

/// Our markdown on a card: the title and each `**Label:**`.
fn trusted_marks(text: &str, labels: &[&str]) -> String {
    let mut rest = text.to_string();
    for l in labels {
        rest = rest.replace(&format!("**{l}:**"), "");
    }
    let title = rest
        .lines()
        .find(|l| l.starts_with("**"))
        .unwrap()
        .to_string();
    rest.replace(&title, "")
}

#[test]
fn the_shared_vector_is_the_card_the_bench_posts() {
    let v = vectors();
    let overrides: Vec<(&str, Value)> = v["drop"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    let text = render_card(&check(&overrides).unwrap(), "14:05");
    assert_eq!(text, v["text"].as_str().unwrap());
    // Replaced by a newer card, it is edited closed (publish (b)); a desktop
    // shows that text read-only in the same layout.
    let closed = render_closed(&text)
        + &thread_line("8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10", &"e".repeat(64));
    assert_eq!(closed, v["closed"].as_str().unwrap());
    // Every label, bold, once, in the writer's order.
    let labels: Vec<&str> = v["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    let at: Vec<usize> = labels
        .iter()
        .map(|l| text.find(&format!("**{l}:**")).unwrap())
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
    // The phone (gpt_markdown) honours no backslash escape, and a `*` after
    // a U+FEFF (Dart `\s`) can never close: only our `**` remain live.
    assert!(!text.contains('\\'));
    let rest = trusted_marks(&text, &labels);
    let chars: Vec<char> = rest.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == '*' {
            assert_eq!(chars[i - 1], F, "a live * in {rest:?}");
        }
    }
}

#[test]
fn a_label_opens_its_own_paragraph_and_its_value_stays_inert() {
    assert_eq!(
        body("Stay or move?\nWhat changes: **all** of #general\nAsked by: Craig"),
        format!(
            "Stay or move?\n\n**What changes:** {F}*{F}*all{F}*{F}* of #{F}general\n\n\
             **Asked by:** Craig"
        )
    );
    // Lines under a label join its paragraph until a blank line.
    assert_eq!(
        body("Details: one\ntwo\n\nthree"),
        "**Details:** one\ntwo\n\nthree"
    );
    // A blank line already there is kept, not doubled.
    assert_eq!(body("a\n\nIf you wait: b"), "a\n\n**If you wait:** b");
    assert_eq!(body("Recommended:"), "**Recommended:**");
    assert_eq!(
        body("If you wait: x\nThe choice: speed or safety"),
        "**If you wait:** x\n\n**The choice:** speed or safety"
    );
}

#[test]
fn the_risk_line_splits_only_at_its_own_labels() {
    // Its values read as written, through prose(), but no `*` of the
    // writer's sits between our `**`s: each is a lookalike.
    assert_eq!(
        body("Risk: *High*; Undo: Can't; Reach: 3,089 members; and more"),
        "**Risk:** \u{2217}High\u{2217} \u{b7} **Undo:** Can't \u{b7} **Reach:** 3,089 members; and more"
    );
    assert_eq!(
        body("Risk: Low; Undo: Easy; Reach: <5% of guests"),
        format!("**Risk:** Low \u{b7} **Undo:** Easy \u{b7} **Reach:** <{F}5% of guests")
    );
    assert_eq!(
        body("Reach: ~40 facilitators / 2 coaches, **all**"),
        format!("**Reach:** {F}~40 facilitators / 2 coaches, \u{2217}\u{2217}all\u{2217}\u{2217}")
    );
    // Undo and Reach open a risk line of their own too.
    assert_eq!(body("Reach: everyone"), "**Reach:** everyone");
    // Elsewhere "; Undo:" is the writer's text.
    assert_eq!(
        body("What changes: a; Undo: b"),
        format!("**What changes:** a; Undo:{F} b")
    );
}

#[test]
fn only_whitelisted_labels_at_a_line_start_are_labels() {
    for text in [
        "What happened: merged",
        "Note: What changes: x",
        "the Risk: low",
        "Risky: no",
        "what changes: lowercase",
        "**What changes:** typed bold",
        "> Asked by: a quote",
    ] {
        let out = body(text);
        assert!(!out.starts_with("**"), "{text:?} -> {out:?}");
        assert_eq!(out, prose(text), "{text:?}");
    }
    // A card without labels: today's text, byte for byte.
    let plain = "Stay or move?\n- one\n\n\n1. two @x";
    assert_eq!(body(plain), prose(plain));
    assert!(card_of(plain).contains(&format!("\n\n{}\n\n1. Stay", prose(plain))));
}

#[test]
fn the_header_names_the_project_never_its_slug() {
    for (project, area, header) in [
        (None, "28-day-reset", "28 Day Reset"),
        (None, "reset", "28 Day Reset"),
        (None, "dreamforge", "Dreamforge"),
        (None, "buzz", "Dreamforge"),
        (None, "resetting", "resetting"),
        (None, "guest-gate", "guest-gate"),
        (Some("reset"), "28-day-reset", "28 Day Reset"),
        (Some("acme"), "acme", "acme"),
        // An area that says more than the project stays.
        (None, "reset-q4", "28 Day Reset \u{b7} reset-q4"),
        (None, "buzz-relay", "Dreamforge \u{b7} buzz-relay"),
        (
            Some("reset"),
            "reset-billing",
            "28 Day Reset \u{b7} reset-billing",
        ),
        (Some("acme"), "release", "acme \u{b7} release"),
    ] {
        assert_eq!(place(project, area), header, "{project:?} {area}");
    }
    let card = |o: &[(&str, Value)]| render_card(&check(o).unwrap(), "14:05");
    assert!(card(&[("area", json!("28-day-reset"))])
        .starts_with("\u{1F534} \u{b7} 28 Day Reset \u{b7} orchestrator\n**Relay choice**"));
    assert!(card(&[("area", json!("guest-gate"))])
        .starts_with("\u{1F534} \u{b7} guest-gate \u{b7} orchestrator\n"));
}

#[test]
fn plain_details_print_after_the_recommendation_capped_and_inert() {
    let item = check(&[
        ("recommend", json!(2)),
        (
            "plain",
            json!({"details": "Checks: **green**\n- 2 files", "why": "safer"}),
        ),
    ])
    .unwrap();
    let text = render_card(&item, "14:05");
    assert!(
        text.contains(&format!(
            "\n\n**Recommended:** 2 (orchestrator) - safer\n\n\
             **Details:** Checks:{F} {F}*{F}*green{F}*{F}*\n{F}- 2 files\n\n1. Stay"
        )),
        "{text}"
    );
    // As long as a summary may be, no longer.
    let long = check(&[("plain", json!({"details": "a".repeat(3000)}))]).unwrap();
    let text = render_card(&long, "14:05");
    assert!(text.contains(&format!("**Details:** {}\u{2026}\n", "a".repeat(400))));
    // Blank details print nothing.
    let blank = check(&[("plain", json!({"details": " "}))]).unwrap();
    assert!(!render_card(&blank, "14:05").contains("Details"));
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
        "🔴 · Dreamforge · orchestrator\n\
         **Relay choice**\n\
         \n\
         Stay or move?\n\
         \n\
         1. Stay\n\
         2. Move\n\
         \n\
         due 2026-09-20T00:00:00Z\n\
         plan: https://github.com/QuicksilverSlick/buzz\n\
         Tap one number below to answer.\n\
         as of 14:05"
    );
    let decision = check(&[
        ("kind", json!("decision")),
        ("decidedBy", json!("claude")),
        ("severity", Value::Null),
        ("options", Value::Null),
    ])
    .unwrap();
    assert_eq!(
        render_card(&decision, "14:05"),
        "🧭 · Dreamforge · orchestrator\n**Relay choice**\n\nStay or move?\n\nas of 14:05"
    );
}
