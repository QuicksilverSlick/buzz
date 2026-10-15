//! The notices to a writer's route target: when one is due, the writer's
//! own text, and the owner's own message (spec section 9) with exactly what
//! may go in it. Pure functions, split out of item.rs (the file-size
//! ratchet).

use serde::{Deserialize, Serialize};

use super::item::*;

/// An answer change waits this long before it is routed, so an untap and a
/// new tap (how the owner changes an answer) reach the agent as one notice.
pub(crate) const ROUTE_SETTLE_SECS: u64 = 120;

/// The last answer notice sent to the writer's route target.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Routed {
    /// 0-based option the notice announced on this item's card. None: a
    /// withdrawal, or, with `label`, an answer on the card before a re-ask
    /// (`carry_card`), which still stands.
    pub option: Option<usize>,
    pub to: String,
    pub event_id: String,
    pub at: u64,
    /// The notice was the owner's own message (`render_owner_route`): a
    /// withdrawal or someone else's answer takes it back the same way first.
    #[serde(default)]
    pub owner: bool,
    /// The told option's text, which still stands; None after a withdrawal
    /// (and on a notice from before it was kept: then this card's option).
    #[serde(default)]
    pub label: Option<String>,
    /// The card it was told for, which the owner's own message names.
    #[serde(default)]
    pub card: Option<String>,
}

/// An owner's notice signed and maybe not on the relay yet, on disk before
/// it is first sent (`publish`), so a lost reply, a crash or an older state
/// file only ever sends this same event again, never a second approval.
/// `routed` lands once the relay has it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnerSent {
    pub event: nostr::Event,
    pub routed: Routed,
}

/// Whether the route target is owed a notice, once the answer has settled:
/// it is not the one it was last told; or the owner's message told it and
/// it is someone else's now; or it may go out as the owner's own message
/// (`owner_route`) and the last notice was not one. A clear that was never
/// routed owes nothing; a re-tap of the told option is no change, and nor
/// is choosing again the answer a re-ask carried over.
pub(crate) fn route_due(item: &Item, now: u64, owner_set: &[String]) -> bool {
    let want = item.answer.as_ref().map(|a| a.option);
    let told = item.routed.as_ref();
    let changed = match told {
        None => want.is_some(),
        Some(r) if r.option.is_none() && r.label.is_some() => want
            .and_then(|w| item.options.get(w))
            .is_some_and(|l| r.label.as_ref() != Some(l)),
        Some(r) => r.option != want,
    };
    let owner = told.is_some_and(|r| r.owner);
    let theirs = owner && item.answer.as_ref().is_some_and(|a| a.by_name.is_some());
    let unsigned = !owner && owner_route(item, owner_set).is_some();
    (changed || theirs || unsigned) && now.saturating_sub(item.changed_at) >= ROUTE_SETTLE_SECS
}

/// The notice for the item's current answer; `prev` is the option the target
/// was last told (None: never, or a withdrawal). Card text goes through
/// plain_text as on the card: it is drop text, so the agent is pointed at the
/// outbox file, which carries the owner's signed reaction.
pub(crate) fn render_route(item: &Item, prev: Option<usize>, hhmm: &str) -> String {
    let id = &item.id;
    let opt = |i: usize| {
        let text = item.options.get(i).map(|o| plain_text(o, usize::MAX));
        format!("{}. {}", i + 1, text.unwrap_or_default())
    };
    let head = format!("{id}\nCard: {}", plain_text(&item.title, usize::MAX));
    let by = match &item.answer {
        Some(a) if item.scoped() => answerer_text(item, a.by_name.as_deref(), a.role),
        _ => "the owner".to_string(),
    };
    let details =
        format!("Answered {hhmm} by {by}. Details: outbox/{id}.json in the Bench folder.");
    let clear = format!(
        "reply in this thread, then clear the card by re-dropping {id} as a decision or status."
    );
    match (item.answer.as_ref().map(|a| a.option), prev) {
        (Some(n), None) => format!(
            "Bench answer \u{b7} {head}\nChosen: {}\n{details}\nAct on it, {clear}",
            opt(n)
        ),
        (Some(n), Some(p)) => format!(
            "Bench answer changed \u{b7} {head}\nChosen: {}\nWas: {}\n{details}\n\
             Undo what you did for the old choice if it conflicts, {clear}",
            opt(n),
            opt(p)
        ),
        (None, Some(p)) => {
            // On a scoped card someone other than the owner may have cleared it.
            let cleared = if item.scoped() {
                format!("The answer was cleared at {hhmm}.")
            } else {
                format!("The owner cleared the answer at {hhmm}.")
            };
            format!(
                "Bench answer withdrawn \u{b7} {head}\nWas: {}\n{cleared} Stop if you have not \
                 acted; if you have, say in this thread what to undo. Do not clear the card.",
                opt(p)
            )
        }
        (None, None) => String::new(),
    }
}

/// The notice for a free-text reply on a scoped card (its Other). Like an
/// answer notice it carries no reply text: the agent reads the reply itself
/// in the thread, with the trust it gives that person.
pub(crate) fn render_route_reply(item: &Item, hhmm: &str) -> String {
    let Some(r) = &item.reply else {
        return String::new();
    };
    let id = &item.id;
    format!(
        "Bench reply \u{b7} {id}\nCard: {}\n{} replied in this thread at {hhmm} instead of \
         choosing an option. Read their reply (event {}) in this thread, answer them there, \
         and when it is settled close the card by re-dropping {id} as a decision or status \
         with the outcome.",
        plain_text(&item.title, usize::MAX),
        answerer_text(item, r.by_name.as_deref(), r.role),
        r.event_id
    )
}

/// Text the owner's own message may carry: exactly what both apps showed
/// (the desktop prints drop text as it is, the phone through plain_text),
/// so nothing the owner signs reads differently from what the owner saw.
fn exact(t: &str, max: usize) -> Option<&str> {
    (!t.is_empty() && plain_text(t, max) == t).then_some(t)
}

/// An option's label for the owner's own message: it passes the drop checks
/// again and prints as it is.
fn owner_label(o: &str) -> Option<&str> {
    valid_text(o, OPTION_MAX, "option").ok()?;
    exact(o, usize::MAX)
}

/// The option a notice told, which still stands: as kept, else (told before
/// the text was kept) this card's.
fn told_label<'a>(item: &'a Item, r: &'a Routed) -> Option<&'a str> {
    r.label
        .as_deref()
        .or_else(|| item.options.get(r.option?).map(String::as_str))
}

/// Who the owner's own message says spoke: roles.json's name for the owner,
/// only as it prints, else "The owner".
fn owner_who(item: &Item) -> Option<String> {
    let name = item.stamp.as_ref().map(|s| s.owner_name.as_str());
    match name.filter(|n| !n.is_empty()) {
        Some(n) => exact(n, BOARD_TITLE_MAX).map(str::to_string),
        None => Some("The owner".to_string()),
    }
}

/// How the owner's own message names a card: the first 12 hex of its event
/// id, which both apps open and no writer chose.
fn card_name(event_id: &str) -> Option<&str> {
    valid_hex64(event_id).then(|| &event_id[..12])
}

/// The answer as the owner's own message, or None and the writer's notice
/// goes instead: only a tap from the owner's keys, which names its card
/// exactly (a reply may name only its thread's root, so it may be meant for
/// another card there), and only text `render_owner_route` passes.
pub(crate) fn owner_route(item: &Item, owner_set: &[String]) -> Option<String> {
    item.answer
        .as_ref()
        .filter(|a| a.source == AnswerSource::Tap && owner_set.contains(&a.by))?;
    render_owner_route(item)
}

/// The notice for an answer from the owner's own keys, which the Bench posts
/// from the owner's identity so the agent acts on the owner's word (spec
/// section 9). Fixed wording around the option labels the card shows and
/// the live card's id, never the title or the item id (writer text the
/// owner never saw chosen), plus the card's action line when the
/// recommended option was chosen, and the told answer it replaces unless
/// that is the same option. None when any of them fails the drop checks
/// again or does not print as it is (`exact`): the writer's notice goes.
pub(crate) fn render_owner_route(item: &Item) -> Option<String> {
    let a = item.answer.as_ref()?;
    let label = owner_label(item.options.get(a.option)?)?;
    let card = card_name(&super::live_card(item)?.event_id)?;
    let who = owner_who(item)?;
    let mut out =
        format!("{who} chose: {label} on card {card}. I approve the action that option describes.");
    if let Some(action) = item.action.as_deref() {
        super::plain::valid_action(action).ok()?;
        if item.recommend == Some(a.option + 1) {
            out.push_str(&format!("\nAction: {action}"));
        }
    }
    let told = item.routed.as_ref().and_then(|r| told_label(item, r));
    if let Some(was) = told.filter(|was| *was != label) {
        out.push_str(&format!(
            "\nThis replaces the earlier answer: {}. Do not act on that one; undo what was \
             done for it if it conflicts.",
            owner_label(was)?
        ));
    }
    Some(out)
}

/// The owner's own message taking back `told`, last routed as
/// `render_owner_route`, now withdrawn or someone else's: so no approval of
/// the owner's outlives it. It names the card that approval named.
pub(crate) fn render_owner_withdrawal(item: &Item, told: &Routed) -> Option<String> {
    let label = owner_label(told_label(item, told)?)?;
    let card = told
        .card
        .as_deref()
        .or_else(|| super::live_card(item).map(|c| c.event_id.as_str()));
    let card = card_name(card?)?;
    let who = owner_who(item)?;
    Some(format!(
        "{who} withdrew: {label} on card {card}. I no longer approve the action that option \
         describes. Stop if you have not acted; if you have, say in this thread what to undo. \
         Do not clear the card."
    ))
}
