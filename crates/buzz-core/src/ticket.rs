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

    /// Whether the ticket still expects action and that action is now due.
    ///
    /// A live ticket with **no** deadline counts as overdue. "Nobody said when
    /// to check this" is not the same as "never check this", and reading an
    /// absent deadline as never-due is how a single malformed row silences the
    /// sweeper permanently. Sweeping it immediately is noisy; not sweeping it
    /// is the failure this whole system exists to prevent.
    pub fn is_overdue(&self, now: i64) -> bool {
        if self.state.is_terminal() {
            return false;
        }
        match self.deadline {
            Some(d) => now >= d,
            None => true,
        }
    }
}

/// Stand-in when a `failed` row carries no reason.
pub const MISSING_REASON: &str = "no reason was recorded";

/// Whether `candidate` should replace `current` as an actor's latest word.
///
/// Recency decides it. The interesting case is a tie, which is ordinary rather
/// than exotic: Nostr timestamps are whole seconds, so two rows from one actor
/// in the same second is a normal occurrence and `StatusRow` carries nothing
/// finer to order them by.
///
/// On a tie the rule is "stay watched", applied in two steps:
///
/// 1. **Prefer the non-terminal row.** An extra escalation on a finished
///    ticket is noise; a dropped request is not recoverable.
/// 2. **Then prefer the tighter deadline**, treating "no deadline" as the
///    loosest. Sweeping early is survivable; sweeping never is the failure.
///
/// Deliberately NOT ordered by [`TicketState`] rank. Rank and deadline
/// tightness are independent: an `Acknowledged` row can carry a looser
/// deadline than a `Progress` row from the same second, so preferring the
/// lower rank would discard the tighter deadline and produce exactly the
/// silent ticket this is meant to prevent.
fn supersedes(candidate: &StatusRow, current: &StatusRow) -> bool {
    if candidate.created_at != current.created_at {
        return candidate.created_at > current.created_at;
    }
    match (candidate.state.is_terminal(), current.state.is_terminal()) {
        (false, true) => return true,
        (true, false) => return false,
        _ => {}
    }
    // Tighter deadline wins; a row that names one beats a row that does not.
    match (candidate.not_before, current.not_before) {
        (Some(c), Some(k)) => c < k,
        (Some(_), None) => true,
        _ => false,
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
                if supersedes(r, cur) {
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
            // A failure a person cannot read is a silent failure wearing a
            // loud label. If the reason is missing, say so rather than
            // rendering an empty field the reader has to interpret.
            reason: match (t.state, &t.reason) {
                (TicketState::Failed, None) => Some(MISSING_REASON.to_string()),
                _ => t.reason.clone(),
            },
        };
    }

    let decided = latest
        .values()
        .max_by_key(|r| (r.state, r.created_at))
        .expect("non-empty");

    // Earliest live deadline: fire on the first thing that should have
    // happened. A live row that named no deadline falls back to
    // `open_deadline` rather than contributing nothing — otherwise an actor
    // who acknowledges without a deadline makes the ticket LESS watched than
    // if they had said nothing at all.
    let live = latest.values().filter(|r| !r.state.is_terminal());
    let mut deadline: Option<i64> = None;
    for r in live {
        let d = r.not_before.or(open_deadline);
        deadline = match (deadline, d) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

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

    // ── Regressions from the adversarial review ───────────────────────
    //
    // Each of these reproduced a real hole found by independently compiling
    // and running this module. They are the cases where the fold used to go
    // quiet, which is the one thing a watchdog must never do.

    #[test]
    fn a_live_row_with_no_deadline_is_swept_not_ignored() {
        // Was: an Acknowledged row with no deadline gave deadline=None, and
        // is_overdue returned false at EVERY representable timestamp — so one
        // lazy row silenced the sweeper forever. Worse than posting nothing,
        // since a ticket with no rows at least keeps its ack deadline.
        let v = fold(&[row("agent", TicketState::Acknowledged, 10, None)], None);
        assert!(
            v.is_overdue(i64::MAX / 2),
            "a live ticket nobody gave a deadline must be checked, not ignored"
        );
        assert!(
            v.is_overdue(0),
            "unknown deadline means check now, not never"
        );
    }

    #[test]
    fn a_live_row_with_no_deadline_falls_back_to_the_ack_deadline() {
        // Acknowledging must never make a ticket LESS watched than silence.
        let with_row = fold(
            &[row("agent", TicketState::Acknowledged, 10, None)],
            Some(100),
        );
        let without_row = fold(&[], Some(100));
        assert_eq!(
            with_row.deadline, without_row.deadline,
            "an ack that names no deadline must not discard the ack deadline"
        );
        assert!(with_row.is_overdue(100));
    }

    #[test]
    fn a_terminal_ticket_with_no_deadline_is_still_not_overdue() {
        // The fix above must not make finished tickets sweep forever.
        for st in [TicketState::Done, TicketState::Failed] {
            let v = fold(&[row("agent", st, 10, None)], Some(1));
            assert!(!v.is_overdue(i64::MAX / 2), "{st:?} is finished");
        }
    }

    #[test]
    fn a_failure_without_a_reason_still_says_something() {
        // A failure a person cannot read is a silent failure wearing a loud
        // label. Nothing enforced the reason, so an empty field reached the
        // reader.
        let v = fold(&[row("agent", TicketState::Failed, 10, None)], None);
        assert_eq!(v.state, TicketState::Failed);
        assert_eq!(v.reason.as_deref(), Some(MISSING_REASON));
    }

    #[test]
    fn same_second_ties_keep_the_ticket_watched_not_terminal() {
        // Was: a same-second Done swallowed the actor's own newer live row
        // via state-rank ordering, terminalising the ticket permanently.
        // Nostr timestamps are whole seconds, so this is ordinary.
        let v = fold(
            &[
                row("agent", TicketState::Done, 100, None),
                row("agent", TicketState::Acknowledged, 100, Some(130)),
            ],
            None,
        );
        assert_eq!(
            v.state,
            TicketState::Acknowledged,
            "on a tie, prefer the row that keeps the ticket watched"
        );
        assert!(v.is_overdue(200));
    }

    #[test]
    fn same_second_ties_keep_the_tighter_deadline() {
        // The counterexample that disproved the reviewer's proposed fix:
        // ordering by state RANK would pick Acknowledged (loose, 1000) over
        // Progress (tight, 130) and go silent at now=200. Rank and deadline
        // tightness are independent, so tightness is what must decide.
        let v = fold(
            &[
                row("agent", TicketState::Acknowledged, 100, Some(1000)),
                row("agent", TicketState::Progress, 100, Some(130)),
            ],
            None,
        );
        assert_eq!(v.deadline, Some(130), "the tighter deadline must survive");
        assert!(
            v.is_overdue(200),
            "must still fire — this is the silent-ticket case"
        );
    }

    #[test]
    fn same_second_tie_break_is_order_independent() {
        // Whichever order the relay hands them to us, the answer must match.
        let a = row("agent", TicketState::Done, 100, None);
        let b = row("agent", TicketState::Acknowledged, 100, Some(130));
        assert_eq!(fold(&[a.clone(), b.clone()], None), fold(&[b, a], None));
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
