//! Who answered a card: reading taps and thread replies against a card's
//! answerers. Pure functions, split out of item.rs (the file-size ratchet).

use super::item::*;

/// The event a reaction is aimed at: its last `e` tag, how the relay scopes
/// a reaction.
pub(crate) fn reaction_target(e: &nostr::Event) -> Option<&str> {
    e.tags
        .iter()
        .filter_map(|t| match t.as_slice() {
            [k, v, ..] if k == "e" => Some(v.as_str()),
            _ => None,
        })
        .last()
}

/// Whether `pk` may answer this card: the owner's keys (`owner_set`: the
/// owner plus members.txt) always, a stamped answerer if it is a human or
/// the card is delegable. The second half repeats `roles::stamp`'s rule.
pub(crate) fn eligible(item: &Item, pk: &str, owner_set: &[String]) -> bool {
    owner_set.iter().any(|o| o == pk)
        || item.stamp.as_ref().is_some_and(|s| {
            s.answerers
                .iter()
                .any(|a| a.pk == pk && (!a.agent || item.needs == Needs::Delegable))
        })
}

/// The event a kind 9 replies to: its NIP-10 `reply` e tag, else its only e
/// tag. A reply deeper in a thread names its own parent, so a card that is
/// only the root of another card's thread is not what it answers.
fn parent(e: &nostr::Event) -> Option<&str> {
    let es: Vec<&[String]> = e
        .tags
        .iter()
        .map(|t| t.as_slice())
        .filter(|t| matches!(t, [k, _, ..] if k == "e"))
        .collect();
    let reply = es.iter().find(|t| t.get(3).is_some_and(|m| m == "reply"));
    reply
        .or(match es.as_slice() {
            [only] => Some(only),
            _ => None,
        })
        .map(|t| t[1].as_str())
}

/// Until when (inclusive) the live card of `item` also takes replies that
/// name only its thread's root, as Block's phone sends from a thread it
/// shows from the root: from its card to the next card the Bench has in that
/// thread, kept ones (`kept_at`) included, so each such reply counts for one
/// card, the newest before it. None without a thread, or in one rooted at a
/// card: a reply there is that card's own.
// shortcut: a recovered card publish (c) keeps has no time here, so an
// older card in its thread can take its root replies; rare (a post state
// lost), record its thread root from its tags if it comes up.
pub(crate) fn root_until<'a>(
    item: &Item,
    items: impl IntoIterator<Item = &'a Item>,
    kept: &[String],
    kept_at: &[(String, u64)],
) -> Option<u64> {
    let root = item.reply_to.as_deref()?;
    let after = item.card.as_ref()?.created_at;
    let mut until = u64::MAX;
    for j in items {
        let Some(c) = &j.card else { continue };
        if c.event_id == root {
            return None;
        }
        let thread = j.request.as_deref().unwrap_or(j).reply_to.as_deref();
        if thread == Some(root) && c.created_at > after {
            until = until.min(c.created_at);
        }
    }
    for (r, at) in kept_at {
        if r == root && *at > after {
            until = until.min(*at);
        }
    }
    (!kept.iter().any(|k| k == root)).then_some(until)
}

/// Whether the kind 9 `e` replies to the card `card_id`: it names the card
/// as its parent, or names only the card's thread root, after the card and
/// by `root_until`. The Bench's own route notices never do: it signs those
/// with the owner's keys for the owner's answers.
fn on_card(item: &Item, card_id: &str, e: &nostr::Event, root_until: Option<u64>) -> bool {
    let notice = e.tags.iter().any(
        |t| matches!(t.as_slice(), [k, m, ..] if k == "client" && m.starts_with("bench:route:")),
    );
    let (p, at) = (parent(e), e.created_at.as_secs());
    !notice
        && (p == Some(card_id)
            || root_until.is_some_and(|until| {
                p.is_some()
                    && p == item.reply_to.as_deref()
                    && item.card.as_ref().is_some_and(|c| c.created_at < at)
                    && at <= until
            }))
}

/// The 0-based option an event answers on the card `card_id`: a kind 7
/// aimed at it by its `reaction_target` and echoing one seeded keycap
/// byte for byte, or, on a scoped card only, a kind 9 replying to it
/// (`on_card`) whose whole text is "1" to "n" or exactly one option's label.
pub(crate) fn answer_option(
    item: &Item,
    card_id: &str,
    e: &nostr::Event,
    root_until: Option<u64>,
) -> Option<usize> {
    let n = item.options.len().min(OPTION_EMOJI.len());
    match e.kind.as_u16() {
        7 if reaction_target(e) == Some(card_id) => {
            OPTION_EMOJI[..n].iter().position(|o| *o == e.content)
        }
        9 if item.scoped() && on_card(item, card_id, e, root_until) => {
            let t = e.content.trim();
            (1..=n).position(|k| t == k.to_string()).or_else(|| {
                let mut hits = item.options[..n]
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| plain_text(o, usize::MAX).eq_ignore_ascii_case(t));
                match (hits.next(), hits.next()) {
                    (Some((i, _)), None) => Some(i),
                    _ => None,
                }
            })
        }
        _ => None,
    }
}

/// A scoped card's Other: a kind 9 replying to it that names no option.
fn free_text(item: &Item, card_id: &str, e: &nostr::Event, root_until: Option<u64>) -> bool {
    item.scoped()
        && e.kind.as_u16() == 9
        && on_card(item, card_id, e, root_until)
        && answer_option(item, card_id, e, root_until).is_none()
}

/// Whether the owner's keys answered or replied on the live card `card_id`.
pub(crate) fn owner_spoke(
    item: &Item,
    card_id: &str,
    events: &[nostr::Event],
    owner_set: &[String],
    root_until: Option<u64>,
) -> bool {
    events.iter().any(|e| {
        owner_set.contains(&e.pubkey.to_hex())
            && (answer_option(item, card_id, e, root_until).is_some()
                || free_text(item, card_id, e, root_until))
            && e.verify().is_ok()
    })
}

/// Whether `e` counts on the live card: from a key that may answer it,
/// never the writer's own seeds, and once the owner's keys answered or
/// replied there (`only_owner`), from theirs alone. Taps and Other alike,
/// so nobody else's tap or reply ever outranks the owner's.
fn counts(
    item: &Item,
    e: &nostr::Event,
    owner_set: &[String],
    writer_hex: &str,
    only_owner: bool,
) -> bool {
    let pk = e.pubkey.to_hex();
    pk != writer_hex
        && eligible(item, &pk, owner_set)
        && (!only_owner || owner_set.contains(&pk))
        && e.verify().is_ok()
}

/// The answer on `item`'s live card: the newest answer that `counts`, so
/// a second tap changes it. A retraction is the event no longer coming back
/// from the relay, which hands the answer to the next.
pub(crate) fn pick_answer<'e>(
    item: &Item,
    events: &'e [nostr::Event],
    owner_set: &[String],
    writer_hex: &str,
    root_until: Option<u64>,
) -> Option<(usize, &'e nostr::Event)> {
    let card = &item.card.as_ref()?.event_id;
    let only_owner = owner_spoke(item, card, events, owner_set, root_until);
    events
        .iter()
        .filter(|e| counts(item, e, owner_set, writer_hex, only_owner))
        .filter_map(|e| Some((answer_option(item, card, e, root_until)?, e)))
        .max_by_key(|(_, e)| (e.created_at.as_secs(), e.id.to_hex()))
}

/// The newest free-text reply on a scoped card that `counts`.
pub(crate) fn pick_reply<'e>(
    item: &Item,
    events: &'e [nostr::Event],
    owner_set: &[String],
    writer_hex: &str,
    root_until: Option<u64>,
) -> Option<&'e nostr::Event> {
    let card = &item.card.as_ref().filter(|_| item.scoped())?.event_id;
    let only_owner = owner_spoke(item, card, events, owner_set, root_until);
    events
        .iter()
        .filter(|e| {
            free_text(item, card, e, root_until)
                && counts(item, e, owner_set, writer_hex, only_owner)
        })
        .max_by_key(|e| (e.created_at.as_secs(), e.id.to_hex()))
}

/// Valid answers on the live card from keys that may not answer it, for
/// the refusedTaps log.
pub(crate) fn refused_taps<'e>(
    item: &Item,
    events: &'e [nostr::Event],
    owner_set: &[String],
    writer_hex: &str,
    root_until: Option<u64>,
) -> Vec<&'e nostr::Event> {
    let Some(card) = &item.card else {
        return Vec::new();
    };
    events
        .iter()
        .filter(|e| {
            let pk = e.pubkey.to_hex();
            pk != writer_hex
                && answer_option(item, &card.event_id, e, root_until).is_some()
                && !eligible(item, &pk, owner_set)
                && e.verify().is_ok()
        })
        .collect()
}
