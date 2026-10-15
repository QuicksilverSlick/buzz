//! The plain-language card layout (spec section 9): a drop's `plain` fields
//! and its `action` line. Pure functions beside item.rs: their checks, and
//! the lines a card prints for them.

use serde::{Deserialize, Serialize};

use super::item::{plain_text, valid_text, Item};

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
    /// Folded on the desktop; never on the phone's card.
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

/// The lines under a card's summary, words only like every card line but
/// the summary: "What changes", "Asked by", risk, undo and reach, "If you
/// wait". Empty without `plain`, so such a card's text is as it was.
pub(crate) fn lines(item: &Item) -> String {
    let Some(p) = &item.plain else {
        return String::new();
    };
    let undo = p.undo.map(|u| match u {
        Undo::Easy => "Easy",
        Undo::Hard => "Hard",
        Undo::Impossible => "Impossible",
    });
    let risk: Vec<String> = [
        p.risk.map(|r| format!("Risk: {}", level(r))),
        undo.map(|u| format!("Undo: {u}")),
        words(&p.reach).map(|r| format!("Reach: {r}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    [
        words(&p.what_changes).map(|t| format!("What changes: {t}")),
        words(&p.asked_by).map(|t| format!("Asked by: {t}")),
        Some(risk.join(" \u{b7} ")).filter(|r| !r.is_empty()),
        words(&p.if_you_wait).map(|t| format!("If you wait: {t}")),
    ]
    .into_iter()
    .flatten()
    .map(|l| format!("\n{l}"))
    .collect()
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
