//! The project registry: the kit's seed reads, a card's header names its
//! project by slug or alias (the longest wins), and a missing or invalid
//! file shows the built-in tabs and says why. Child of tests.rs.

use super::*;
use crate::bench::commands::cards;
use crate::bench::plain::{place_in, shown_place};
use crate::bench::projects::{built_in, validate, Project};

fn vectors() -> Value {
    serde_json::from_str(include_str!("../projects_vectors.json")).unwrap()
}

fn seed() -> Vec<Project> {
    serde_json::from_value(vectors()["registry"]["projects"].clone()).unwrap()
}

#[test]
fn the_kits_seed_places_every_card_by_slug_or_alias() {
    let projects = seed();
    validate(&projects).unwrap();
    for c in vectors()["cards"].as_array().unwrap() {
        let header: Vec<String> = serde_json::from_value(c[4].clone()).unwrap();
        let (project, area) = (c[0].as_str(), c[1].as_str().unwrap());
        assert_eq!(
            place_in(&projects, project, area),
            header.join(" \u{b7} "),
            "{c}"
        );
    }
    // Before the registry, byte for byte the header the kit's rebuild expects
    // of every card posted so far (bench-card.mjs place).
    assert_eq!(
        place_in(&built_in(), None, "dreamforge-talos"),
        "Dreamforge \u{b7} dreamforge-talos"
    );
    assert_eq!(place_in(&built_in(), None, "buzz"), "Dreamforge");
    assert_eq!(place_in(&built_in(), None, "28-day-reset"), "28 Day Reset");
}

#[test]
fn a_registry_that_would_misplace_a_card_is_refused() {
    let refused = |edit: fn(&mut Vec<Project>)| {
        let mut projects = seed();
        edit(&mut projects);
        validate(&projects).unwrap_err()
    };
    assert_eq!(
        refused(|p| p[2].aliases.push("buzz".into())),
        "\"buzz\" names two projects"
    );
    assert!(refused(|p| p[0].parent = Some("talos".into())).contains("parent"));
    assert_eq!(
        refused(|p| p[2].parent = Some("habits".into())),
        "project \"talos\": parent \"habits\" is not a top-level project"
    );
    assert!(refused(|p| p[1].label = "**Reset**".into()).contains("plain words"));
    assert!(refused(|p| p[1].label = String::new()).contains("plain words"));
    assert!(refused(|p| p[2].slug = "Talos".into()).contains("not a Bench slug"));
    assert!(refused(|p| p[2].aliases.push("other".into())).contains("not a Bench slug"));
    // No card's normalised name equals these: the tab would stay empty.
    assert!(refused(|p| p[2].aliases.push("talos--old".into())).contains("not a Bench slug"));
    assert!(refused(|p| p[2].slug = "talos-".into()).contains("not a Bench slug"));
    assert_eq!(refused(|p| p.clear()), "no projects");
    // The kit reads a null list as none, so one it accepts stays usable here.
    let null: Project = serde_json::from_value(json!({
        "slug": "a", "label": "A", "aliases": null
    }))
    .unwrap();
    assert!(null.aliases.is_empty());
}

#[test]
fn the_snapshot_carries_the_registry_or_says_why_not() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bench");
    std::fs::create_dir(&root).unwrap();
    let file = dir.path().join("projects.json");
    let snap = || serde_json::to_value(cards(&root)).unwrap();

    let missing = snap();
    assert_eq!(missing["projects"], Value::Null);
    let notice = missing["projectsNotice"].as_str().unwrap();
    assert!(notice.contains("projects.json is missing"), "{notice}");
    assert!(notice.contains("built-in tabs (28 Day Reset, Dreamforge)."));

    std::fs::write(&file, "{\"projects\": [").unwrap();
    let cut = snap()["projectsNotice"].as_str().unwrap().to_string();
    assert!(cut.contains("projects.json can't be used: "), "{cut}");

    // Over the cap it goes unread, and the notice says so.
    std::fs::write(&file, " ".repeat(64 * 1024 + 1)).unwrap();
    let big = snap()["projectsNotice"].as_str().unwrap().to_string();
    assert!(big.contains("projects.json is over 65536 bytes"), "{big}");

    // A reason echoing the file stops at the kit's length.
    let echo = json!({"projects": [{"slug": "x".repeat(50_000), "label": "X"}]});
    std::fs::write(&file, echo.to_string()).unwrap();
    let long = snap()["projectsNotice"].as_str().unwrap().to_string();
    assert!(long.contains("xxxx") && long.len() < 500, "{}", long.len());
    assert!(long.ends_with("once the file reads."));

    std::fs::write(&file, vectors()["registry"].to_string()).unwrap();
    let read = snap();
    assert_eq!(read["projectsNotice"], Value::Null);
    assert_eq!(
        read["projects"][2],
        json!({
            "slug": "talos",
            "label": "Talos",
            "parent": "dreamforge",
            "aliases": ["dreamforge-talos"]
        })
    );
}

/// A registry written beside the root reaches the next card's header, which
/// the card keeps for the kit's rebuild (bench-card.mjs cardHead), also through
/// a recovery from the relay; gone, the next card is back to the built-in name
/// and the first is unchanged.
#[tokio::test]
async fn a_tick_heads_its_cards_by_the_registry_beside_the_root() {
    let mut b = Bench::new(Gate::Open, &[]).await;
    let file = b.root.parent().unwrap().join("projects.json");
    let talos = drop_json(&[("area", json!("dreamforge-talos"))]);
    let reset = drop_json(&[("area", json!("reset-billing"))]);

    std::fs::write(&file, vectors()["registry"].to_string()).unwrap();
    b.drop_file("steward", "by-registry", &talos);
    b.drop_file("steward", "reset-area", &reset);
    b.settle(6).await;
    let first = b.card("steward/by-registry");
    assert_eq!(first.place.as_deref(), Some("Talos"));
    assert_eq!(
        b.card("steward/reset-area").place.as_deref(),
        Some("28 Day Reset \u{b7} reset-billing")
    );

    let event = json!(first.event_id);
    let posts = b.posts();
    let (card, _) = posts.iter().find(|(e, _)| e["id"] == event).unwrap();
    let content = card["content"].as_str().unwrap();
    assert!(content.starts_with("\u{1F534} \u{b7} Talos \u{b7} steward\n"));
    let writer = b.ctx.writer.public_key().to_hex();
    let relayed: nostr::Event = serde_json::from_value(card.clone()).unwrap();
    assert_eq!(
        rebuild_map(&[relayed], &writer).cards["steward/by-registry"].place,
        first.place
    );
    assert_eq!(
        shown_place("Back to you\n\u{1F7E0} \u{b7} Dreamforge \u{b7} buzz-x \u{b7} steward\n**T**")
            .as_deref(),
        Some("Dreamforge \u{b7} buzz-x")
    );

    std::fs::remove_file(&file).unwrap();
    b.drop_file("steward", "built-in", &talos);
    b.settle(6).await;
    assert_eq!(
        b.card("steward/built-in").place.as_deref(),
        Some("Dreamforge \u{b7} dreamforge-talos")
    );
    assert_eq!(b.card("steward/by-registry"), first);
}
