//! Desktop cards: `bench_cards`, a read-only view of state.json for the
//! Dreamforge Bench screen, and `bench_answer`, which answers a card with the
//! owner's own keycap reaction so the running tick records it like a phone
//! tap. Neither writes state.json; neither returns or logs key material.

use std::path::Path;

use serde::Serialize;

use super::item::{self, ItemKind, Link, Severity, StatusState, Validity, OPTION_EMOJI};
use super::{live_card, load_state, post_as_owner, query, root_dir, BenchState, Ctx};
use crate::app_state::AppState;
use crate::events;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchCards {
    enabled: bool,
    /// Absent while the opt-in folder is missing.
    #[serde(flatten)]
    snapshot: Option<Snapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    channel_id: String,
    writer_pubkey: String,
    owner_pubkey: Option<String>,
    last_error: Option<String>,
    last_tick: String,
    items: Vec<Card>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Card {
    id: String,
    writer: String,
    kind: ItemKind,
    title: String,
    body: String,
    area: String,
    severity: Option<Severity>,
    options: Vec<String>,
    state: Option<StatusState>,
    decided_by: Option<String>,
    deadline: Option<String>,
    links: Vec<Link>,
    order: i64,
    validity: Validity,
    /// The live card only: one showing the item's current Drop.
    card_event_id: Option<String>,
    answer: Option<CardAnswer>,
    recorded: Option<usize>,
}

/// 0-based, like `options` and `bench_answer`'s `option`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CardAnswer {
    option: usize,
    answered_at: u64,
}

/// The snapshot under `root`, through the service's own state loader.
pub(crate) fn cards(root: &Path) -> BenchCards {
    if !root.is_dir() {
        return BenchCards {
            enabled: false,
            snapshot: None,
        };
    }
    let (s, _) = load_state(root);
    let items = s
        .items
        .values()
        .map(|i| Card {
            id: i.id.clone(),
            writer: i.writer.clone(),
            kind: i.kind,
            title: i.title.clone(),
            body: i.body.clone(),
            area: i.area.clone(),
            severity: i.severity,
            options: i.options.clone(),
            state: i.state,
            decided_by: i.decided_by.clone(),
            deadline: i.deadline.clone(),
            links: i.links.clone(),
            order: i.order,
            validity: i.validity,
            card_event_id: live_card(i).map(|c| c.event_id.clone()),
            answer: i.answer.as_ref().map(|a| CardAnswer {
                option: a.option,
                answered_at: a.answered_at,
            }),
            recorded: i.recorded,
        })
        .collect();
    BenchCards {
        enabled: true,
        snapshot: Some(Snapshot {
            channel_id: s.channel_id,
            writer_pubkey: s.writer_pubkey,
            owner_pubkey: Some(s.owner_pubkey).filter(|o| !o.is_empty()),
            last_error: s.last_error,
            last_tick: s.last_tick,
            items,
        }),
    }
}

/// The Bench as state.json last recorded it; read-only.
#[tauri::command]
pub(crate) async fn bench_cards() -> Result<BenchCards, String> {
    Ok(cards(&root_dir()?))
}

/// What to publish so the owner's keycaps on one card show exactly `chosen`
/// (None: none at all).
#[derive(Debug, Default, PartialEq)]
pub(crate) struct AnswerPlan {
    pub add: Option<&'static str>,
    /// Reaction event ids.
    pub delete: Vec<String>,
}

/// `existing` is the owner's (event id, content) reactions on the card. Only
/// the card's own keycaps (the set pick_answer reads) are touched; any other
/// content is left alone. An out-of-range choice plans nothing.
pub(crate) fn plan_answer(
    existing: &[(String, String)],
    chosen: Option<usize>,
    n_options: usize,
) -> AnswerPlan {
    let keycaps = &OPTION_EMOJI[..n_options.min(OPTION_EMOJI.len())];
    let want = match chosen {
        Some(i) if i >= keycaps.len() => return AnswerPlan::default(),
        c => c.map(|i| keycaps[i]),
    };
    AnswerPlan {
        add: want.filter(|w| !existing.iter().any(|(_, c)| c == w)),
        delete: existing
            .iter()
            .filter(|(_, c)| keycaps.contains(&c.as_str()) && want != Some(c.as_str()))
            .map(|(id, _)| id.clone())
            .collect(),
    }
}

/// bench_answer's relay half. Reads the owner's reactions on the live card
/// with the poll's own /query, then publishes the add before the deletes, so
/// a failed delete never loses the new answer (latest wins in pick_answer).
pub(crate) async fn answer_card(
    ctx: &Ctx<'_>,
    s: &BenchState,
    item_id: &str,
    option: Option<u8>,
    now: u64,
) -> Result<(), String> {
    let item = s
        .items
        .get(item_id)
        .ok_or_else(|| format!("no Bench item {item_id:?}"))?;
    if item.kind != ItemKind::Pending {
        return Err(format!("{item_id} is not a pending item"));
    }
    let card = live_card(item)
        .ok_or_else(|| format!("{item_id} has no live card yet"))?
        .event_id
        .clone();
    let n = item.options.len().min(OPTION_EMOJI.len());
    let chosen = option.map(usize::from);
    if let Some(i) = chosen.filter(|i| *i >= n) {
        return Err(format!(
            "option {i} out of range: {item_id} has {n} options"
        ));
    }
    let owner = ctx.owner.public_key();
    if s.owner_pubkey != owner.to_hex() {
        return Err("owner identity changed; refusing to answer".to_string());
    }
    let filter = serde_json::json!({
        "kinds": [7], "authors": [s.owner_pubkey], "#h": [s.channel_id], "#e": [card],
        "limit": super::RECOVER_LIMIT,
    });
    let events = query(ctx, s, filter, now).await?;
    // A cut page could hide a keycap that would outrank the new one.
    if events.len() as u32 >= super::RECOVER_LIMIT {
        return Err("too many reactions on this card to plan an answer".to_string());
    }
    let existing: Vec<(String, String)> = events
        .iter()
        .filter(|e| {
            e.kind.as_u16() == 7
                && e.pubkey == owner
                && item::reaction_target(e) == Some(card.as_str())
        })
        .map(|e| (e.id.to_hex(), e.content.clone()))
        .collect();
    let plan = plan_answer(&existing, chosen, n);
    let target = nostr::EventId::from_hex(&card).map_err(|e| e.to_string())?;
    if let Some(emoji) = plan.add {
        post_as_owner(ctx, events::build_reaction(target, emoji)?).await?;
    }
    for id in plan.delete {
        let id = nostr::EventId::from_hex(&id).map_err(|e| e.to_string())?;
        post_as_owner(ctx, events::build_remove_reaction(id)?).await?;
    }
    Ok(())
}

/// `option`: Some(i) answers with option i (0-based), None clears the answer.
/// The Bench relay and keys, never the app's active community.
#[tauri::command]
pub(crate) async fn bench_answer(
    item_id: String,
    option: Option<u8>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let env = std::env::var("BUZZ_BENCH_RELAY_URL").ok();
    let relay =
        super::relay_ws_from(env.as_deref()).ok_or("the Bench is disabled in this build")?;
    let root = root_dir()?;
    if !root.is_dir() {
        return Err("the Bench is not enabled".to_string());
    }
    let (s, _) = load_state(&root);
    let ctx = Ctx {
        state: state.inner(),
        base: crate::relay::relay_http_base_url(&relay),
        owner: state.signing_keys()?,
        writer: super::writer_keys()?,
        root,
    };
    let now = nostr::Timestamp::now().as_secs();
    answer_card(&ctx, &s, &item_id, option, now).await
}
