//! Ticket state: the fold from status events to "what is true now".
//!
//! A ticket exists so a request cannot be silently dropped. It is written down
//! when it arrives, and something outside the agent sweeps for tickets that
//! stopped moving. This module is the part with no I/O: given a ticket's status
//! events, decide the current state and when the next deadline expires.
//!
//! # Why a fold rather than a stored status column
//!
//! State is derived from [`crate::kind::KIND_TICKET_STATUS`] events, one live
//! row per (ticket, actor). Deriving it means the events stay canonical — there
//! is no second copy to drift, and a replayed log always reproduces the same
//! answer. It also means an actor can only ever speak for themselves: a status
//! row is addressed by `(ticket, actor)`, and NIP-33 binds the author pubkey
//! into the replacement key, so nobody can forge another actor's row.
//!
//! # Why deadlines are part of the state
//!
//! Every non-terminal state carries the instant it must be acted on by. The
//! sweeper does not need to understand *why* a ticket is stuck — only that its
//! deadline passed. That is what lets it catch failures nobody enumerated in
//! advance, which is the majority of the ones that matter.

use std::collections::BTreeMap;

/// What an actor last said about a ticket.
///
/// Ordering is deliberate: [`Ord`] ranks terminal states above live ones so a
/// fold can take the max without special-casing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TicketState {
    /// Written down, nobody has picked it up.
    Open,
    /// Someone accepted it. Proves the request was not lost in transit.
    Acknowledged,
    /// Work is happening, renewed periodically. A stale Progress is the signal
    /// that something hung — silence, not an error.
    Progress,
    /// Raised to a human because a deadline passed or the agent gave up.
    Escalated,
    /// Terminal: finished.
    Done,
    /// Terminal: gave up, with a reason the requester can read.
    Failed,
}

impl TicketState {
    /// Whether no further action is expected.
    ///
    /// `Escalated` is deliberately NOT terminal: raising something to a human
    /// is not the same as resolving it. A ticket that escalates and is then
    /// ignored is exactly the case this system exists to catch.
    pub const fn is_terminal(self) -> bool {
        matches!(self, TicketState::Done | TicketState::Failed)
    }

    /// Machine-readable form used in the `s` tag.
    pub const fn as_tag(self) -> &'static str {
        match self {
            TicketState::Open => "open",
            TicketState::Acknowledged => "ack",
            TicketState::Progress => "progress",
            TicketState::Escalated => "escalated",
            TicketState::Done => "done",
            TicketState::Failed => "failed",
        }
    }

    /// Parse an `s` tag value. Unknown values are rejected rather than guessed:
    /// a ticket in an unrecognised state must not silently look healthy.
    pub fn from_tag(s: &str) -> Option<Self> {
        Some(match s {
            "open" => TicketState::Open,
            "ack" => TicketState::Acknowledged,
            "progress" => TicketState::Progress,
            "escalated" => TicketState::Escalated,
            "done" => TicketState::Done,
            "failed" => TicketState::Failed,
            _ => return None,
        })
    }
}

/// One actor's latest word on a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    /// Hex pubkey of whoever published this row.
    pub actor: String,
    /// What this actor last said about the ticket.
    pub state: TicketState,
    /// Unix seconds the row was created.
    pub created_at: i64,
    /// Unix seconds by which this state must change, if it is not terminal.
    pub not_before: Option<i64>,
    /// Required when `state` is [`TicketState::Failed`] — a failure a person
    /// cannot read is a failure they cannot act on.
    pub reason: Option<String>,
}

/// The resolved state of one ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketView {
    /// The state the fold resolved to.
    pub state: TicketState,
    /// The deadline the sweeper watches. `None` once terminal.
    pub deadline: Option<i64>,
    /// Which actor's row decided `state`.
    pub decided_by: Option<String>,
    /// Why it failed, carried through from the deciding row.
    pub reason: Option<String>,
}

impl TicketView {
    /// A ticket nobody has said anything about yet.
    pub fn open(deadline: Option<i64>) -> Self {
        Self {
            state: TicketState::Open,
            deadline,
            decided_by: None,
            reason: None,
        }
    }

    /// Whether `now` is past the deadline and the ticket still expects action.
    pub fn is_overdue(&self, now: i64) -> bool {
        !self.state.is_terminal() && self.deadline.is_some_and(|d| now >= d)
    }
}

/// Resolve a ticket's current state from every actor's status row.
///
/// Rules, in order:
///
/// 1. **Terminal wins.** If any actor says done or failed, the ticket is
///    finished. Anyone working on it has finished or given up, and a stale
///    Progress row from another actor must not keep it alive forever.
/// 2. **Otherwise the furthest-advanced live state wins**, so one actor still
///    reporting Progress keeps the ticket live even if another only ever
///    acknowledged it.
/// 3. **Ties break on recency**, so the most recent word wins.
/// 4. **The deadline is the EARLIEST** among live rows, not the latest. A
///    watchdog must fire on the first thing that should have happened; taking
///    the latest would let one long-running actor mask another's stall.
///
/// `open_deadline` applies when there are no rows at all — the acknowledgement
/// deadline, which is what catches a request that never reached anyone.
pub fn fold(rows: &[StatusRow], open_deadline: Option<i64>) -> TicketView {
    if rows.is_empty() {
        return TicketView::open(open_deadline);
    }

    // One row per actor: the latest they published. Rows arriving out of order
    // must not resurrect an older state.
    let mut latest: BTreeMap<&str, &StatusRow> = BTreeMap::new();
    for r in rows {
        latest
            .entry(r.actor.as_str())
            .and_modify(|cur| {
                if (r.created_at, r.state) > (cur.created_at, cur.state) {
                    *cur = r;
                }
            })
            .or_insert(r);
    }

    let terminal = latest
        .values()
        .filter(|r| r.state.is_terminal())
        .max_by_key(|r| (r.created_at, r.state));

    if let Some(t) = terminal {
        return TicketView {
            state: t.state,
            deadline: None,
            decided_by: Some(t.actor.clone()),
            reason: t.reason.clone(),
        };
    }

    let decided = latest
        .values()
        .max_by_key(|r| (r.state, r.created_at))
        .expect("non-empty");

    // Earliest live deadline: fire on the first thing that should have happened.
    let deadline = latest
        .values()
        .filter(|r| !r.state.is_terminal())
        .filter_map(|r| r.not_before)
        .min();

    TicketView {
        state: decided.state,
        deadline,
        decided_by: Some(decided.actor.clone()),
        reason: decided.reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(actor: &str, state: TicketState, at: i64, not_before: Option<i64>) -> StatusRow {
        StatusRow {
            actor: actor.to_string(),
            state,
            created_at: at,
            not_before,
            reason: None,
        }
    }

    #[test]
    fn no_rows_is_open_and_carries_the_ack_deadline() {
        // The case that matters most: a request nobody ever picked up. It must
        // still have a deadline, or the commonest failure is the invisible one.
        let v = fold(&[], Some(100));
        assert_eq!(v.state, TicketState::Open);
        assert_eq!(v.deadline, Some(100));
        assert!(v.is_overdue(100), "overdue exactly at the deadline");
        assert!(!v.is_overdue(99));
    }

    #[test]
    fn terminal_beats_a_live_row_from_another_actor() {
        // Otherwise one actor's stale Progress keeps a finished ticket alive.
        let v = fold(
            &[
                row("agent", TicketState::Progress, 10, Some(500)),
                row("reviewer", TicketState::Done, 20, None),
            ],
            None,
        );
        assert_eq!(v.state, TicketState::Done);
        assert_eq!(
            v.deadline, None,
            "a finished ticket has nothing to wait for"
        );
        assert!(!v.is_overdue(9_999));
    }

    #[test]
    fn escalated_is_not_terminal() {
        // Raising something to a human is not resolving it. An escalation that
        // is then ignored is the exact case this system exists to catch.
        let v = fold(
            &[row("watchdog", TicketState::Escalated, 5, Some(50))],
            None,
        );
        assert!(!v.state.is_terminal());
        assert_eq!(v.deadline, Some(50));
        assert!(v.is_overdue(60));
    }

    #[test]
    fn deadline_is_the_earliest_live_one_not_the_latest() {
        // Taking the latest would let a long-running actor mask another's stall.
        let v = fold(
            &[
                row("slow", TicketState::Progress, 10, Some(9_000)),
                row("stalled", TicketState::Acknowledged, 10, Some(60)),
            ],
            None,
        );
        assert_eq!(v.deadline, Some(60));
        assert!(v.is_overdue(60), "the earlier deadline must fire");
    }

    #[test]
    fn only_the_latest_row_per_actor_counts() {
        // Out-of-order delivery must not resurrect a superseded state.
        let v = fold(
            &[
                row("agent", TicketState::Progress, 30, Some(900)),
                row("agent", TicketState::Acknowledged, 10, Some(40)),
            ],
            None,
        );
        assert_eq!(v.state, TicketState::Progress);
        assert_eq!(
            v.deadline,
            Some(900),
            "the superseded row's deadline must not linger"
        );
    }

    #[test]
    fn furthest_advanced_live_state_wins() {
        let v = fold(
            &[
                row("a", TicketState::Acknowledged, 10, Some(100)),
                row("b", TicketState::Progress, 10, Some(200)),
            ],
            None,
        );
        assert_eq!(v.state, TicketState::Progress);
    }

    #[test]
    fn failure_reason_survives_the_fold() {
        // A failure a person cannot read is a failure they cannot act on.
        let mut r = row("agent", TicketState::Failed, 10, None);
        r.reason = Some("could not reach the repository".to_string());
        let v = fold(&[r], None);
        assert_eq!(v.state, TicketState::Failed);
        assert_eq!(v.reason.as_deref(), Some("could not reach the repository"));
    }

    #[test]
    fn unknown_state_tags_are_rejected_not_guessed() {
        // A ticket in an unrecognised state must not silently look healthy.
        assert_eq!(
            TicketState::from_tag("ack"),
            Some(TicketState::Acknowledged)
        );
        assert_eq!(TicketState::from_tag("finished"), None);
        assert_eq!(TicketState::from_tag(""), None);
    }

    #[test]
    fn tag_round_trips() {
        for s in [
            TicketState::Open,
            TicketState::Acknowledged,
            TicketState::Progress,
            TicketState::Escalated,
            TicketState::Done,
            TicketState::Failed,
        ] {
            assert_eq!(TicketState::from_tag(s.as_tag()), Some(s));
        }
    }
}
