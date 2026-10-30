//! Bound project channels: reading roles.json, the owner's joins of the
//! Bench writer and its route targets, the admin check that refuses a channel
//! someone outside the owner's keys can moderate, and whether a card's
//! channel is ready for it. #bench itself stays bootstrap's.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{item, post_as_owner, query, roles, spend, BenchState, Ctx};
use crate::events;

/// A failed join, a refused admin list or a failed read is tried again this
/// soon; a list that passed is re-read every RECHECK_SECS.
const RETRY_SECS: u64 = 60;
const RECHECK_SECS: u64 = 600;
/// A passed list outlives failed reads this long: an admin added meanwhile
/// could delete the owner's winning tap unseen.
const TRUST_SECS: u64 = 3600;
const JOIN_FAILED: &str = "could not add";

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelCheck {
    pub checked_at: u64,
    /// When the admin list was last read; behind checked_at, that read failed.
    #[serde(default)]
    pub ok_at: u64,
    /// Why no card goes here: a fixed reason, printed on the board.
    pub refused: Option<String>,
}

impl ChannelCheck {
    fn join_failed(&self) -> bool {
        self.refused
            .as_deref()
            .is_some_and(|r| r.starts_with(JOIN_FAILED))
    }
}

/// roles.json; none means nothing is bound.
pub(super) fn read_roles(root: &Path) -> Result<Option<roles::Roles>, String> {
    match std::fs::read_to_string(root.join("roles.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("roles.json: {e}")),
        Ok(text) => roles::parse_roles(&text).map(Some),
    }
}

/// The channel an item's card lives in: a receipt's is its request's.
pub(super) fn card_channel(s: &BenchState, i: &item::Item) -> String {
    let asked = i.request.as_deref().unwrap_or(i);
    asked
        .channel
        .clone()
        .unwrap_or_else(|| s.channel_id.clone())
}

/// Who the owner has added to a channel for the Bench.
pub(super) fn joined<'a>(s: &'a BenchState, ch: &str) -> &'a [String] {
    if ch == s.channel_id {
        return &s.members_added;
    }
    s.joined.get(ch).map_or(&[], Vec::as_slice)
}

/// Whether thread replies on a card that is not scoped are read: while its
/// writer's route target is in the card's channel, as publish (a3) needs to
/// tell it, and for good once one was read, so one bad tick of routes.json
/// or the agent store never un-engages a card mid-conversation.
pub(super) fn reads_replies(
    s: &BenchState,
    i: &item::Item,
    routes: &BTreeMap<String, String>,
) -> bool {
    i.reply.is_some()
        || routes
            .get(&i.writer)
            .is_some_and(|to| joined(s, &card_channel(s, i)).contains(to))
}

/// #bench always; a bound channel once the writer is in it and its admin
/// list passed.
pub(super) fn channel_ready(s: &BenchState, i: &item::Item) -> bool {
    let ch = card_channel(s, i);
    ch == s.channel_id
        || (joined(s, &ch).contains(&s.writer_pubkey)
            && s.channels.get(&ch).is_some_and(|c| c.refused.is_none()))
}

/// The owner adds the writer, then the route targets of the folders bound to
/// the project, to each of its channels. A failure is never fatal: that
/// channel's cards wait and it is tried again in RETRY_SECS.
pub(super) async fn join_bound(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    budget: &mut usize,
    now: u64,
    roles: &roles::Roles,
    routes: &BTreeMap<String, String>,
) {
    for (slug, project) in &roles.projects {
        let joiners: Vec<String> = std::iter::once(s.writer_pubkey.clone())
            .chain(
                roles
                    .writers
                    .iter()
                    .filter(|(_, b)| b.projects.contains(slug))
                    .filter_map(|(w, _)| routes.get(w).cloned()),
            )
            .collect();
        for ch in &project.channels {
            let waiting = s
                .channels
                .get(ch)
                .is_some_and(|c| c.join_failed() && now < c.checked_at + RETRY_SECS);
            if *ch == s.channel_id || waiting {
                continue;
            }
            for hex in &joiners {
                if joined(s, ch).contains(hex) {
                    continue;
                }
                if !spend(budget, 1) {
                    return;
                }
                let add = match uuid::Uuid::parse_str(ch) {
                    Ok(uuid) => events::build_add_member(uuid, hex, None),
                    Err(e) => Err(e.to_string()),
                };
                let added = match add {
                    Ok(builder) => post_as_owner(ctx, builder).await.map(|_| ()),
                    Err(e) => Err(e),
                };
                if let Err(e) = added {
                    eprintln!("bench: add {hex} to {ch}: {e}");
                    let refused = Some(format!("{JOIN_FAILED} {}", &hex[..8]));
                    s.channels.insert(
                        ch.clone(),
                        ChannelCheck {
                            checked_at: now,
                            refused,
                            ..Default::default()
                        },
                    );
                    break;
                }
                s.joined.entry(ch.clone()).or_default().push(hex.clone());
                s.routes_held_until = now + item::ROUTE_SETTLE_SECS;
            }
            // Everyone is in: the admin check decides from here.
            let all_in = joiners.iter().all(|h| joined(s, ch).contains(h));
            if all_in && s.channels.get(ch).is_some_and(ChannelCheck::join_failed) {
                s.channels.remove(ch);
            }
        }
    }
}

/// One channel per tick, once the writer is in it. A channel owner or admin
/// can delete any event, the owner's winning tap included, so a channel with
/// one outside the owner's keys gets no cards. Only the relay's own signed
/// list counts. One that cannot be read refuses a first check and otherwise
/// keeps the last verdict for TRUST_SECS after the last read, retrying every
/// RETRY_SECS, so a relay hiccup never takes a channel's cards off the board.
pub(super) async fn check_admins(
    ctx: &Ctx<'_>,
    s: &mut BenchState,
    now: u64,
    owner_set: &[String],
) {
    let due = s
        .joined
        .iter()
        .filter(|(ch, m)| **ch != s.channel_id && m.contains(&s.writer_pubkey))
        .map(|(ch, _)| ch)
        .find(|ch| match s.channels.get(*ch) {
            None => true,
            Some(c) if c.join_failed() => false,
            Some(c) => {
                let wait = if c.refused.is_some() || c.ok_at < c.checked_at {
                    RETRY_SECS
                } else {
                    RECHECK_SECS
                };
                now >= c.checked_at + wait
            }
        })
        .cloned();
    let Some(ch) = due else { return };
    let (refused, ok_at) = match admin_refusal(ctx, s, &ch, owner_set, now).await {
        Ok(verdict) => (verdict.map(str::to_string), now),
        Err(why) => {
            eprintln!("bench: {ch}: {why}");
            match s.channels.get(&ch) {
                Some(c) if now <= c.ok_at + TRUST_SECS => (c.refused.clone(), c.ok_at),
                c => (Some(why.to_string()), c.map_or(0, |c| c.ok_at)),
            }
        }
    };
    s.channels.insert(
        ch,
        ChannelCheck {
            checked_at: now,
            ok_at,
            refused,
        },
    );
}

/// The verdict on a channel's admin list; Err: it could not be read.
async fn admin_refusal(
    ctx: &Ctx<'_>,
    s: &BenchState,
    ch: &str,
    owner_set: &[String],
    now: u64,
) -> Result<Option<&'static str>, &'static str> {
    let filter = serde_json::json!({ "kinds": [39001], "#d": [ch], "limit": 10 });
    let Ok(lists) = query(ctx, s, filter, now).await else {
        return Err("the admin list could not be read");
    };
    let Ok(Some(relay)) = crate::commands::fetch_relay_self_at(ctx.state, &ctx.base).await else {
        return Err("the relay's key is unknown");
    };
    let names_ch = |e: &&nostr::Event| {
        e.tags
            .iter()
            .any(|t| matches!(t.as_slice(), [k, d, ..] if k == "d" && d == ch))
    };
    let Some(list) = lists
        .iter()
        .filter(|e| e.kind.as_u16() == 39001 && e.pubkey.to_hex() == relay)
        .filter(names_ch)
        .max_by_key(|e| e.created_at)
    else {
        return Ok(Some("no admin list from the relay"));
    };
    // The list holds only owners and admins.
    let outside = list.tags.iter().any(|t| {
        matches!(t.as_slice(), [k, pk, ..] if k == "p" && !owner_set.contains(&pk.to_ascii_lowercase()))
    });
    Ok(outside.then_some("an owner or admin outside your keys"))
}
