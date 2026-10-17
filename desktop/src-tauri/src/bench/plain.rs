//! The plain-language card layout (spec section 9): a drop's `plain` fields
//! and its `action` line. Pure functions beside item.rs: their checks, and
//! the lines a card prints for them.

use serde::{Deserialize, Serialize};

use super::item::{plain_text, prose, summary, valid_text, Item};

const PLAIN_MAX: usize = 300;
const DETAILS_MAX: usize = 3000;
const ACTION_MAX: usize = 300;

/// What a decision means, in plain words: chips on the desktop, short lines
/// on the phone. `why` and `confidence` go with `recommend`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Plain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what_changes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<Level>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<Undo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_you_wait: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Level>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Folded on the desktop; on the phone, capped like the summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Level {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Undo {
    Easy,
    Hard,
    Impossible,
}

/// Text members follow the body's rules. `recommended` says whether the drop
/// has a `recommend` for `why`, `confidence` and the action to go with.
pub(crate) fn validate(
    plain: Option<&Plain>,
    action: Option<&str>,
    recommended: bool,
) -> Result<(), String> {
    if let Some(p) = plain {
        let texts = [
            ("plain.whatChanges", &p.what_changes, PLAIN_MAX),
            ("plain.askedBy", &p.asked_by, PLAIN_MAX),
            ("plain.reach", &p.reach, PLAIN_MAX),
            ("plain.ifYouWait", &p.if_you_wait, PLAIN_MAX),
            ("plain.why", &p.why, PLAIN_MAX),
            ("plain.details", &p.details, DETAILS_MAX),
        ];
        for (field, text, max) in texts {
            if let Some(t) = text {
                valid_text(t, max, field)?;
            }
        }
        if !recommended && (p.why.is_some() || p.confidence.is_some()) {
            return Err("plain.why and plain.confidence: require recommend".to_string());
        }
    }
    if let Some(a) = action {
        if !recommended {
            return Err("action: requires recommend".to_string());
        }
        valid_action(a)?;
    }
    Ok(())
}

/// An action prints as it is, on the card and in the owner's notice, so the
/// owner approves exactly what the agent runs: one line of plain_text's inert
/// characters plus `/`, which neither app reads as markdown there, never
/// ending in a dash (the phone draws a line ending in `---` as a rule).
pub(crate) fn valid_action(a: &str) -> Result<(), String> {
    valid_text(a, ACTION_MAX, "action")?;
    let inert = |c: char| c.is_ascii_alphanumeric() || " ,.!?'\"()&+-/".contains(c);
    if a.is_empty() || a.trim() != a || a.ends_with('-') || !a.chars().all(inert) {
        return Err(
            "action: one line of letters, digits, spaces and ,.!?'\"()&+-/ only".to_string(),
        );
    }
    Ok(())
}

fn words(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(|t| plain_text(t, usize::MAX))
        .filter(|t| !t.is_empty())
}

fn level(l: Level) -> &'static str {
    match l {
        Level::Low => "Low",
        Level::Medium => "Medium",
        Level::High => "High",
    }
}

/// The paragraphs after a card's summary, words only like every card line
/// but the summary: "What changes", "Asked by", risk, undo and reach, "If
/// you wait", each label bold. None without `plain`.
pub(crate) fn lines(item: &Item) -> Vec<String> {
    let Some(p) = &item.plain else {
        return Vec::new();
    };
    let undo = p.undo.map(|u| match u {
        Undo::Easy => "Easy",
        Undo::Hard => "Hard",
        Undo::Impossible => "Impossible",
    });
    let risk: Vec<String> = [
        p.risk.map(|r| bold("Risk", level(r))),
        undo.map(|u| bold("Undo", u)),
        words(&p.reach).map(|r| bold("Reach", &r)),
    ]
    .into_iter()
    .flatten()
    .collect();
    [
        words(&p.what_changes).map(|t| bold("What changes", &t)),
        words(&p.asked_by).map(|t| bold("Asked by", &t)),
        Some(risk.join(RISK_JOIN)).filter(|r| !r.is_empty()),
        words(&p.if_you_wait).map(|t| bold("If you wait", &t)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The labels a writer may type at the start of a line of a body. They
/// print as the plain fields do: bold, a paragraph each. Only these and only
/// at a line's start; anything else, mid-line too, stays the writer's text.
const FIELDS: [&str; 5] = [
    "What changes",
    "Asked by",
    "If you wait",
    "Recommended",
    "Details",
];
/// One line, as the plain risk, undo and reach print; a line that opens with
/// one of these takes the others too, each after a "; ".
const RISK: [&str; 3] = ["Risk", "Undo", "Reach"];
const RISK_JOIN: &str = " \u{b7} ";

/// Our markdown around a value that is already inert.
pub(crate) fn bold(label: &str, value: &str) -> String {
    if value.is_empty() {
        format!("**{label}:**")
    } else {
        format!("**{label}:** {value}")
    }
}

fn label_at<'a>(line: &'a str, labels: &[&'static str]) -> Option<(&'static str, &'a str)> {
    labels
        .iter()
        .find_map(|&l| Some((l, line.strip_prefix(l)?.strip_prefix(':')?.trim())))
}

/// "Risk: Low; Undo: Easy; Reach: x" as the plain risk line prints. Its
/// values go through prose(), so "<5%" and "~40 / 2" read as written, with
/// each `*` as a lookalike: they sit between our `**`s mid-line, where a
/// star of the writer's could pair with one of ours.
fn risk_line(line: &str) -> Option<String> {
    let value = |v: &str| prose(&v.replace('*', "\u{2217}"));
    let (mut label, mut rest) = label_at(line, &RISK)?;
    let mut parts = Vec::new();
    loop {
        let next = rest
            .match_indices("; ")
            .find_map(|(i, _)| Some((i, label_at(&rest[i + 2..], &RISK)?)));
        let Some((i, (next_label, next_rest))) = next else {
            parts.push(bold(label, &value(rest)));
            return Some(parts.join(RISK_JOIN));
        };
        parts.push(bold(label, &value(&rest[..i])));
        (label, rest) = (next_label, next_rest);
    }
}

/// A card's summary: prose() line for line as always, except that a line
/// opening with a label above prints as the label in bold and its value
/// through prose(), after a blank line. Lines under it join its paragraph
/// until a blank line, as in markdown. Nothing of the writer's pairs with our
/// `**`: they open the line, and prose() leaves no `*` that can close.
pub(crate) fn body(summary: &str) -> String {
    let mut out = String::with_capacity(summary.len());
    let mut after_blank = true;
    for (i, line) in summary.trim().split('\n').enumerate() {
        let t = line.trim();
        let field = risk_line(t)
            .or_else(|| label_at(t, &FIELDS).map(|(label, value)| bold(label, &prose(value))));
        if i > 0 {
            out.push_str(if field.is_some() && !after_blank {
                "\n\n"
            } else {
                "\n"
            });
        }
        after_blank = t.is_empty();
        out.push_str(&field.unwrap_or_else(|| prose(line)));
    }
    out
}

/// The workbenches the desktop's Bench tabs name (WORKBENCHES in
/// desktop/src/features/bench/lib/workbenches.ts; keep the two in step).
const WORKBENCHES: [(&str, &[&str]); 2] = [
    ("28 Day Reset", &["28-day-reset", "reset"]),
    ("Dreamforge", &["dreamforge", "buzz"]),
];

/// Where a card's header says it is: its project (else area) by name when
/// the slug is a workbench's or starts with one and a "-", else as it is;
/// then the area when it says more: "28 Day Reset · reset-billing", never
/// "28 Day Reset · 28-day-reset" (benchCardPlace on the desktop).
pub(crate) fn place(project: Option<&str>, area: &str) -> String {
    let slug = project.unwrap_or(area);
    let claims = |known: &&str| {
        slug.strip_prefix(*known)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
    };
    let workbench = WORKBENCHES
        .iter()
        .find(|(_, slugs)| slugs.iter().any(claims));
    let name = workbench.map_or(slug, |(name, _)| name);
    if area == name || workbench.is_some_and(|(_, slugs)| slugs.contains(&area)) {
        name.to_string()
    } else {
        format!("{name}{RISK_JOIN}{area}")
    }
}

/// The plain details, after the recommendation as the desktop shows them,
/// capped like the summary.
pub(crate) fn details(item: &Item) -> Option<String> {
    let d = item.plain.as_ref()?.details.as_deref()?.trim();
    (!d.is_empty()).then(|| bold("Details", &prose(&summary(d))))
}

/// What follows the Recommended line's name: " (high confidence) - <why>",
/// then the action a tap on the recommended option approves, on its own line
/// exactly as the owner's notice carries it.
pub(crate) fn recommended_tail(item: &Item, n: usize) -> String {
    let mut out = String::new();
    if let Some(p) = &item.plain {
        if let Some(c) = p.confidence {
            out.push_str(&format!(" ({} confidence)", level(c).to_lowercase()));
        }
        if let Some(why) = words(&p.why) {
            out.push_str(&format!(" - {why}"));
        }
    }
    let label = item.options.get(n.wrapping_sub(1));
    if let (Some(action), Some(label)) = (&item.action, label) {
        out.push_str(&format!(
            "\nIf you choose {}: {action}",
            plain_text(label, usize::MAX)
        ));
    }
    out
}
