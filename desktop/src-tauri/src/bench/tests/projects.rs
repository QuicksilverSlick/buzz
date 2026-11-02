//! The project registry: the kit's seed reads, a card's header names its
//! project by slug or alias (the longest wins), and a missing or invalid
//! file shows the built-in tabs and says why. Child of tests.rs.

use super::*;
use crate::bench::commands::cards;
use crate::bench::plain::place_in;
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
    assert!(refused(|p| p[2].parent = Some("habits".into())).contains("parent"));
    assert!(refused(|p| p[1].label = "**Reset**".into()).contains("plain words"));
    assert!(refused(|p| p[1].label = String::new()).contains("plain words"));
    assert!(refused(|p| p[2].slug = "Talos".into()).contains("not a Bench slug"));
    assert!(refused(|p| p[2].aliases.push("other".into())).contains("not a Bench slug"));
    assert_eq!(refused(|p| p.clear()), "no projects");
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
