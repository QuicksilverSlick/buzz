//! Desktop cards: `bench_cards`, a read-only view of state.json for the
//! Dreamforge Bench screen, `bench_answer`, which answers a card with the
//! owner's own keycap reaction so the running tick records it like a phone
//! tap, `bench_reply`, the owner's one-line reply in a card's thread (a
//! hold's reason), and `bench_bump`, which asks the tick to post a queued
//! card next.
//! None writes state.json; none returns or logs key material.

use std::path::Path;

use serde::Serialize;

use super::item::{self, ItemKind, Link, Needs, Severity, StatusState, Validity, OPTION_EMOJI};
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
    /// `MAX_CARDS`: new cards post only while fewer slots are in use.
    max_cards: usize,
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
    /// RFC 3339: when the Bench took this drop (a receipt's outcome time).
    updated_at: String,
    /// The live card only: one showing the item's current Drop. A receipt's
    /// is the card it keeps.
    card_event_id: Option<String>,
    /// Why an open card has no card on the relay yet: "slot" (queued for a
    /// `MAX_CARDS` slot) or "channel" (its channel is not ready for cards);
    /// None when it is posted or about to be.
    waits_for: Option<&'static str>,
    answer: Option<CardAnswer>,
    recorded: Option<usize>,
    /// None: #bench. A receipt's is its request's.
    channel_id: Option<String>,
    project: Option<String>,
    /// Gets the For and Recommended lines and Other (`Item::scoped`).
    scoped: bool,
    needs: Needs,
    /// Who may answer besides the owner's own keys.
    answerers: Vec<item::Answerer>,
    owner_name: Option<String>,
    /// 1-based.
    recommend: Option<usize>,
    recommender: Option<String>,
    /// The newest Other reply in the card's thread.
    reply: Option<item::Reply>,
    /// A receipt: the question as it closed.
    request: Option<Box<Card>>,
    /// The drop's plain-language fields, as written.
    plain: Option<item::Plain>,
    /// The command the writer runs if the recommended option is chosen.
    action: Option<String>,
    /// Asked again after an answer, and waiting again.
    back_to_you: bool,
    /// The answer the card came back from.
    previous_answer: Option<CardPrevious>,
}

/// 0-based, like `options`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CardPrevious {
    option: usize,
    label: String,
    /// None: the owner's own keys.
    by_name: Option<String>,
    answered_at: u64,
}

/// 0-based, like `options` and `bench_answer`'s `option`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CardAnswer {
    option: usize,
    answered_at: u64,
    by: String,
    /// A stamped answerer's name; None: the owner's own keys.
    by_name: Option<String>,
    /// Answered from the owner's keys (owner plus members.txt).
    by_owner: bool,
    source: item::AnswerSource,
}

fn card(i: &item::Item, owner_set: &[String]) -> Card {
    let stamp = i.stamp.as_ref();
    let posted = if i.request.is_some() {
        i.card.as_ref()
    } else {
        live_card(i)
    };
    Card {
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
        updated_at: i.updated_at.clone(),
        card_event_id: posted.map(|c| c.event_id.clone()),
        waits_for: None,
        answer: i.answer.as_ref().map(|a| CardAnswer {
            option: a.option,
            answered_at: a.answered_at,
            by: a.by.clone(),
            by_name: a.by_name.clone(),
            // "" is an answer from before step 1, when only these keys counted.
            by_owner: a.by.is_empty() || owner_set.contains(&a.by),
            source: a.source,
        }),
        recorded: i.recorded,
        channel_id: i.request.as_deref().unwrap_or(i).channel.clone(),
        project: i.project.clone(),
        scoped: i.scoped(),
        needs: i.needs,
        answerers: stamp.map(|st| st.answerers.clone()).unwrap_or_default(),
        owner_name: stamp
            .map(|st| st.owner_name.clone())
            .filter(|n| !n.is_empty()),
        recommend: i.recommend,
        recommender: stamp.and_then(|st| st.recommender.clone()),
        reply: i.reply.clone(),
        request: i.request.as_deref().map(|r| Box::new(card(r, owner_set))),
        plain: i.plain.clone(),
        action: i.action.clone(),
        back_to_you: i.back_to_you(),
        previous_answer: i.previous.as_ref().map(|p| CardPrevious {
            option: p.option,
            label: p.label.clone(),
            by_name: p.by_name.clone(),
            answered_at: p.answered_at,
        }),
    }
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
    // As the last tick resolved them: members.txt less any agent key.
    let owner_set = match s.owner_keys.as_slice() {
        [] => vec![s.owner_pubkey.clone()],
        keys => keys.to_vec(),
    };
    let items = s
        .items
        .values()
        .map(|i| Card {
            waits_for: if s.queued.contains(&i.id) {
                Some("slot")
            } else if i.kind == ItemKind::Pending
                && i.card.is_none()
                && !super::channels::channel_ready(&s, i)
            {
                Some("channel")
            } else {
                None
            },
            ..card(i, &owner_set)
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
            max_cards: super::MAX_CARDS,
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

/// What the owner is told when the card they answered is no longer the live
/// one: a re-drop may have changed its action or options since they looked.
pub(crate) const CARD_CHANGED: &str = "This card changed - look again";

/// bench_answer's relay half. Answers only the card the owner was shown
/// (`card_event_id`, the snapshot's cardEventId), and only while it is still
/// the live card. Reads the owner's own reactions on it, in the card's
/// channel, with the poll's /query, and plans against them alone: the
/// owner's answer always wins, so nothing else needs outranking. The add
/// goes before the deletes, so a failed delete never loses the new answer.
pub(crate) async fn answer_card(
    ctx: &Ctx<'_>,
    s: &BenchState,
    item_id: &str,
    card_event_id: &str,
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
    let card = match live_card(item) {
        Some(c) if c.event_id == card_event_id => c.event_id.clone(),
        _ => return Err(CARD_CHANGED.to_string()),
    };
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
        "kinds": [7], "authors": [s.owner_pubkey], "#h": [super::channels::card_channel(s, item)],
        "#e": [card], "limit": super::RECOVER_LIMIT,
    });
    let events = query(ctx, s, filter, now).await?;
    // A cut page could hide a keycap the plan must delete.
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
        let reaction =
            events::build_reaction(target, emoji)?.custom_created_at(nostr::Timestamp::from(now));
        post_as_owner(ctx, reaction).await?;
    }
    for id in &plan.delete {
        remove_reaction(ctx, id).await?;
    }
    Ok(())
}

/// The owner's kind 5 on one of the owner's own reactions.
async fn remove_reaction(ctx: &Ctx<'_>, id: &str) -> Result<(), String> {
    let id = nostr::EventId::from_hex(id).map_err(|e| e.to_string())?;
    post_as_owner(ctx, events::build_remove_reaction(id)?).await?;
    Ok(())
}

/// The longest reply `reply_card` sends: one line (the desktop's
/// BENCH_REASON_MAX).
pub(crate) const REPLY_MAX: usize = 300;

/// bench_reply's relay half: the owner's one-line reply in the thread of the
/// live card `card_event_id`, from the owner's keys on the Bench relay, where
/// the tick reads it as the card's Other and tells the writer's route target
/// (`item::pick_reply`, publish (a3)). Never an answer: text the tick would
/// read as one (`item::answer_option`) is refused, so a reason sent after a
/// tap never changes the tap. No p tags, so it is told (`item::tell_reply`).
pub(crate) async fn reply_card(
    ctx: &Ctx<'_>,
    s: &BenchState,
    item_id: &str,
    card_event_id: &str,
    text: &str,
    now: u64,
) -> Result<(), String> {
    let item = s
        .items
        .get(item_id)
        .ok_or_else(|| format!("no Bench item {item_id:?}"))?;
    if item.kind != ItemKind::Pending {
        return Err(format!("{item_id} is not a pending item"));
    }
    let card = match live_card(item) {
        Some(c) if c.event_id == card_event_id => c,
        _ => return Err(CARD_CHANGED.to_string()),
    };
    let text = text.trim();
    if text.is_empty() || text.chars().count() > REPLY_MAX || text.chars().any(char::is_control) {
        return Err(format!(
            "a reply here is one line of 1 to {REPLY_MAX} characters"
        ));
    }
    if s.owner_pubkey != ctx.owner.public_key().to_hex() {
        return Err("owner identity changed; refusing to reply".to_string());
    }
    let ch = super::channels::card_channel(s, item);
    let ch = uuid::Uuid::parse_str(&ch).map_err(|e| e.to_string())?;
    let thread = super::publish::thread_of(Some(card), item.reply_to.as_deref())?;
    let ev = events::build_message(
        ch,
        text,
        thread.as_ref(),
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        &ctx.base,
    )?
    .custom_created_at(nostr::Timestamp::from(now))
    .sign_with_keys(&ctx.owner)
    .map_err(|e| e.to_string())?;
    if item::answer_option(item, &card.event_id, &ev, None).is_some() {
        return Err(format!(
            "\"{text}\" reads as an answer on this card; say it in other words"
        ));
    }
    super::submit(ctx, &ev, &ctx.owner, None).await?;
    Ok(())
}

/// The Bench relay and keys for a desktop command, never the app's active
/// community, with the state the last tick saved.
fn desk_ctx(state: &AppState) -> Result<(Ctx<'_>, BenchState), String> {
    let env = std::env::var("BUZZ_BENCH_RELAY_URL").ok();
    let relay =
        super::relay_ws_from(env.as_deref()).ok_or("the Bench is disabled in this build")?;
    let root = root_dir()?;
    if !root.is_dir() {
        return Err("the Bench is not enabled".to_string());
    }
    let (s, _) = load_state(&root);
    let ctx = Ctx {
        state,
        base: crate::relay::relay_http_base_url(&relay),
        owner: state.signing_keys()?,
        writer: super::writer_keys()?,
        root,
        agents: Vec::new(),
        access: Vec::new(),
    };
    Ok((ctx, s))
}

/// `option`: Some(i) answers with option i (0-based), None clears the answer,
/// on the card `card_event_id` names.
#[tauri::command]
pub(crate) async fn bench_answer(
    item_id: String,
    card_event_id: String,
    option: Option<u8>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let (ctx, s) = desk_ctx(state.inner())?;
    let now = nostr::Timestamp::now().as_secs();
    answer_card(&ctx, &s, &item_id, &card_event_id, option, now).await
}

/// The owner's one-line reply (a hold's reason, say) on the card
/// `card_event_id` names, in its thread (`reply_card`).
#[tauri::command]
pub(crate) async fn bench_reply(
    item_id: String,
    card_event_id: String,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let (ctx, s) = desk_ctx(state.inner())?;
    let now = nostr::Timestamp::now().as_secs();
    reply_card(&ctx, &s, &item_id, &card_event_id, &text, now).await
}

/// Ask the running tick to post `item_id` next, past `MAX_CARDS` up to
/// `OVER_CAP` (`publish::bump`). Only an open card with no place is bumped;
/// one posted or coming back is left as it is. It never answers the card:
/// the owner taps again once it is live. Errors say why the card cannot
/// post, so a tap on it never waits on nothing.
pub(crate) fn bump(root: &Path, item_id: &str, now: u64) -> Result<(), String> {
    if !root.is_dir() {
        return Err("the Bench is not enabled".to_string());
    }
    let (s, _) = load_state(root);
    let item = s
        .items
        .get(item_id)
        .ok_or_else(|| format!("no Bench item {item_id:?}"))?;
    if item.kind != ItemKind::Pending || item.validity != Validity::Current || item.answer.is_some()
    {
        return Err(format!("{item_id} is no longer an open card"));
    }
    if !super::channels::channel_ready(&s, item) {
        return Err("this card's channel isn't ready for cards yet".to_string());
    }
    if super::publish::placed(item) {
        return Ok(());
    }
    let limit = super::MAX_CARDS + super::publish::OVER_CAP;
    if super::publish::in_use(&s, now) >= limit {
        return Err(format!(
            "all {limit} card slots are in use, the Bench's limit; answer or close a card first"
        ));
    }
    super::publish::bump(root, item_id, now);
    Ok(())
}

/// `bump` on the Bench's own folder.
#[tauri::command]
pub(crate) async fn bench_bump(item_id: String) -> Result<(), String> {
    bump(&root_dir()?, &item_id, nostr::Timestamp::now().as_secs())
}
