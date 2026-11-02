//! The project registry: `projects.json` beside the Bench root, in the steward
//! kit's format (its reference/projects.seed.json; `status`, `repos` and the
//! rest are the kit's). Read-only here: the desktop's tabs get it with the
//! snapshot, and a card's header names its place by it (`plain::place`).
//! Missing or invalid, both use `built_in()` and the snapshot says why.

use std::collections::HashSet;
use std::path::Path;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::item::{plain_text, valid_name, NAME_MAX};

const LABEL_MAX: usize = 40;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub(crate) struct Project {
    pub slug: String,
    pub label: String,
    /// A sub-product's top-level project (Talos under Dreamforge).
    #[serde(default)]
    pub parent: Option<String>,
    /// Old areas and names that still mean this project.
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl Project {
    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.slug.as_str()).chain(self.aliases.iter().map(String::as_str))
    }
}

#[derive(Deserialize)]
struct File {
    projects: Vec<Project>,
}

/// What the snapshot carries: the registry's projects, or None and why the
/// built-in tabs show instead.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Registry {
    pub projects: Option<Vec<Project>>,
    pub projects_notice: Option<String>,
}

/// The tabs before the registry (BUILT_IN_WORKBENCHES in
/// desktop/src/features/bench/lib/workbenches.ts).
pub(crate) fn built_in() -> Vec<Project> {
    let p = |slug: &str, label: &str, alias: &str| Project {
        slug: slug.to_string(),
        label: label.to_string(),
        parent: None,
        aliases: vec![alias.to_string()],
    };
    vec![
        p("reset", "28 Day Reset", "28-day-reset"),
        p("dreamforge", "Dreamforge", "buzz"),
    ]
}

/// The registry beside `root`: `~/.dreamforge/projects.json` (debug:
/// `~/.dreamforge-dev`).
pub(crate) fn load(root: &Path) -> Registry {
    let path = root.parent().unwrap_or(root).join("projects.json");
    let shown = path.display();
    let read = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("{shown} is missing")),
        Err(e) => Err(format!("{shown} can't be read ({e})")),
        Ok(text) => serde_json::from_str::<File>(&text)
            .map_err(|e| e.to_string())
            .and_then(|f| validate(&f.projects).map(|()| f.projects))
            .map_err(|e| format!("{shown} can't be used: {e}")),
    };
    match read {
        Ok(projects) => Registry {
            projects: Some(projects),
            projects_notice: None,
        },
        Err(why) => Registry {
            projects: None,
            projects_notice: Some(format!(
                "{why}, so the Bench shows its built-in tabs (28 Day Reset, Dreamforge). This clears by itself once the file reads."
            )),
        },
    }
}

/// The kit's checks the Bench relies on (lane/projects.mjs laneOf), plus labels
/// that print as written and slugs that are not a tab of the Bench's own.
pub(crate) fn validate(projects: &[Project]) -> Result<(), String> {
    if projects.is_empty() {
        return Err("no projects".to_string());
    }
    let mut keys = HashSet::new();
    for p in projects {
        if p.label.is_empty() || plain_text(&p.label, LABEL_MAX) != p.label {
            return Err(format!(
                "project {:?}: label {:?} is not plain words (up to {LABEL_MAX})",
                p.slug, p.label
            ));
        }
        for k in p.keys() {
            if !valid_name(k, NAME_MAX) || matches!(k, "all" | "other") {
                return Err(format!("project {:?}: {k:?} is not a Bench slug", p.slug));
            }
            if !keys.insert(k) {
                return Err(format!("{k:?} names two projects"));
            }
        }
        let top = |q: &Project| Some(&q.slug) == p.parent.as_ref() && q.parent.is_none();
        if p.parent.is_some() && !projects.iter().any(top) {
            return Err(format!(
                "project {:?}: parent {:?} is not a top-level project",
                p.slug, p.parent
            ));
        }
    }
    Ok(())
}

/// The registry `publish` read this tick, like routes.json: a new project's
/// cards take its name without a relaunch. None: `built_in()`.
// ponytail: one process-wide copy; pass it through render_card if a second
// Bench ever runs in one process.
static CURRENT: RwLock<Option<Vec<Project>>> = RwLock::new(None);

pub(crate) fn refresh(root: &Path) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = load(root).projects;
}

pub(crate) fn with_current<T>(f: impl FnOnce(&[Project]) -> T) -> T {
    let current = CURRENT.read().unwrap_or_else(|e| e.into_inner());
    match current.as_deref() {
        Some(projects) => f(projects),
        None => f(&built_in()),
    }
}
