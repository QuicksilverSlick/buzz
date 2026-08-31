//! Ticket state: the fold from status events to "what is true now".
//!
//! A ticket exists so a request cannot be silently dropped. It is written down
//! when it arrives, and something outside the agent sweeps for tickets that
//! stopped moving. This module is the part with no I/O: given a ticket's status
//! events, decide the current state and when the next deadline expires.
//!
//! # Two axes, deliberately not one
//!
//! An actor's statement about *itself* and the *ticket's* outcome are different
//! kinds of fact, and collapsing them is the bug this module was rebuilt to
//! remove. Every mature tracker separates them — Temporal states it flatly
//! ("an Activity Failure will never directly cause a Workflow Failure"),
//! PagerDuty records a responder as joined or declined on the *responder*, and
//! ITIL has the requester confirm closure rather than the implementer.
//!
//! So one worker giving up records [`Participation::Abandoned`] against
//! **itself**, with a reason, and the ticket stays live for everyone else. Only
//! an [`Authority`] — the requester or the owner — may write an [`Outcome`].
//!
//! # Why a fold rather than a stored status column
//!
//! State is derived from [`crate::kind::KIND_TICKET_STATUS`] events. Deriving
//! it means the events stay canonical, and an actor can only ever speak for
//! themselves: a status row is addressed by `(ticket, actor)`, and NIP-33 binds
//! the author pubkey into the replacement key, so nobody can forge another
//! actor's row.
//!
//! # Known gap: rows are replaceable, so an actor can rewrite its own leaf
//!
//! `kind:30624` is addressable, so only the latest event per
//! `(pubkey, kind, d)` is guaranteed to be stored. An actor that recorded
//! `Abandoned` with a reason on Monday can replace that row on Wednesday, and
//! the evidence that triggered an escalation is gone. Nothing here can prevent
//! that: it is a property of the storage class, not of this fold. Closing it
//! needs status rows to become head pointers over a chain of *regular* (non
//! replaceable) transition events, which is a change to the event model rather
//! than to this module. Recorded here so it is not mistaken for solved.
//!
//! # Why deadlines are part of the state
//!
//! Every non-terminal state carries the instant it must be acted on by. The
//! sweeper does not need to understand *why* a ticket is stuck — only that its
//! deadline passed. That is what lets it catch failures nobody enumerated in
//! advance, which is the majority of the ones that matter.

use std::collections::BTreeMap;

/// Stand-in when a row that owes a reason arrives without one.
pub const MISSING_REASON: &str = "no reason was recorded";

/// What one actor says about its **own** involvement.
///
/// None of these end the ticket. An actor may write any of them about itself
/// at any time, which is deliberate: refusing an agent the ability to say it is
/// stuck would itself be a silent failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Participation {
    /// Accepted it. Proves the request was not lost in transit.
    Acknowledged,
    /// Doing it, renewed periodically. A stale `Working` means something hung.
    Working,
    /// Stuck and saying so. In-project only — this wakes nobody's phone. Which
    /// rung of the escalation ladder to use is the watchdog's decision alone,
    /// so no actor can page a human harder by asking.
    Blocked,
    /// Finished its part, awaiting confirmation. Deliberately **live**, not
    /// terminal: an unconfirmed delivery that nobody looks at must not sit
    /// unnoticed forever.
    Delivered,
    /// Gave up, with a reason. Removes this actor from the live set; it does
    /// **not** end the ticket.
    Abandoned,
}

/// Why a ticket was closed as done.
///
/// A close because a person said so and a close because nobody objected are
/// different facts, and the second must never render as the first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confirmation {
    /// The requester confirmed they got what they asked for.
    ByRequester,
    /// The project owner confirmed on their behalf.
    ByOwner,
    /// Nobody objected before the confirmation window closed. Visibly a
    /// second-class close: it names who delivered and how long the window was.
    Unopposed {
        /// Actor whose delivery went unopposed.
        delivered_by: String,
        /// How long the window was, in seconds.
        window_secs: i64,
    },
}

/// How the **ticket** ended. Only an [`Authority`] may write one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Outcome {
    /// The request was met.
    Done(Confirmation),
    /// Deliberately stopped, with a reason. Distinct from done, and still
    /// reopenable — this is "we are not doing this", not "this never happened".
    Archived,
}

/// What a row asserts: something about its author, or something about the
/// ticket.
///
/// A union rather than one flat enum so a fold rule cannot accidentally treat
/// an actor's exit as the ticket's outcome. That confusion was the bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// A statement about the author's own involvement.
    Participation(Participation),
    /// A statement about how the ticket ended.
    Outcome(Outcome),
}

/// Who is allowed to close a ticket.
///
/// The fold cannot answer "may this actor end this?" without being told, and
/// every authority rule depends on it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Authority {
    /// Hex pubkey of whoever made the request. Always allowed to close.
    pub requester: String,
    /// Hex pubkey of the owning project agent, if any.
    pub owner: Option<String>,
}

impl Authority {
    /// Whether `actor` may write an [`Outcome`] that the fold will honour.
    pub fn may_close(&self, actor: &str) -> bool {
        actor == self.requester || self.owner.as_deref() == Some(actor)
    }
}

/// One actor's latest word on a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    /// Hex pubkey of whoever published this row.
    pub actor: String,
    /// What this row asserts.
    pub kind: RowKind,
    /// Unix seconds the row was created.
    pub created_at: i64,
    /// Unix seconds by which this must change, while the row is live.
    pub deadline: Option<i64>,
    /// Why — required in spirit for `Abandoned` and `Archived`. Rows arriving
    /// off the wire without one fold to [`MISSING_REASON`] rather than an
    /// empty field the reader has to interpret.
    pub reason: Option<String>,
}

impl StatusRow {
    /// Whether this row keeps its author in the live set.
    fn is_live(&self) -> bool {
        matches!(
            self.kind,
            RowKind::Participation(
                Participation::Acknowledged
                    | Participation::Working
                    | Participation::Blocked
                    | Participation::Delivered
            )
        )
    }
}

/// The state a person sees. Ordered so that on a same-instant tie, completion
/// outranks abandonment — see [`fold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TicketState {
    /// Written down, nobody has picked it up.
    Open,
    /// Somebody is on it.
    Working,
    /// Somebody is stuck and said so.
    Blocked,
    /// Work is finished and waiting to be confirmed.
    Delivered,
    /// Everybody who was on it has left. Still live, and needs reassigning.
    Unowned,
    /// The request was met.
    Done,
    /// Deliberately stopped.
    Archived,
}

impl TicketState {
    /// Whether no further action is expected.
    ///
    /// Only an [`Outcome`] is terminal. `Blocked` and `Unowned` are not:
    /// raising a hand is not resolving, and a ticket everyone walked away from
    /// is the case this system exists to catch.
    pub const fn is_terminal(self) -> bool {
        matches!(self, TicketState::Done | TicketState::Archived)
    }

    /// Machine-readable form used in the `s` tag.
    pub const fn as_tag(self) -> &'static str {
        match self {
            TicketState::Open => "open",
            TicketState::Working => "working",
            TicketState::Blocked => "blocked",
            TicketState::Delivered => "delivered",
            TicketState::Unowned => "unowned",
            TicketState::Done => "done",
            TicketState::Archived => "archived",
        }
    }

    /// Parse an `s` tag value. Unknown values are rejected rather than guessed:
    /// a ticket in an unrecognised state must not silently look healthy.
    pub fn from_tag(s: &str) -> Option<Self> {
        Some(match s {
            "open" => TicketState::Open,
            "working" => TicketState::Working,
            "blocked" => TicketState::Blocked,
            "delivered" => TicketState::Delivered,
            "unowned" => TicketState::Unowned,
            "done" => TicketState::Done,
            "archived" => TicketState::Archived,
            _ => return None,
        })
    }
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
    /// Why it ended, carried through from the deciding row.
    pub reason: Option<String>,
    /// How a `Done` was reached, when it was.
    pub confirmation: Option<Confirmation>,
    /// Actors who gave up, with their reasons. Kept visible so a ticket three
    /// people walked away from does not look identical to a fresh one.
    pub abandoned: Vec<(String, String)>,
    /// Actors who tried to close the ticket without the authority to do so.
    /// Their rows are demoted, never obeyed — but obeying is the bug and
    /// dropping them silently is the other bug, so they surface here.
    pub unauthorized_close: Vec<String>,
    /// Actors still working when the ticket was closed. A close is deliberately
    /// not blocked by live work, so the orphaned work has to appear somewhere.
    pub interrupted: Vec<String>,
}

impl TicketView {
    /// A ticket nobody has said anything about yet.
    pub fn open(deadline: Option<i64>) -> Self {
        Self {
            state: TicketState::Open,
            deadline,
            decided_by: None,
            reason: None,
            confirmation: None,
            abandoned: Vec::new(),
            unauthorized_close: Vec::new(),
            interrupted: Vec::new(),
        }
    }

    /// Whether the ticket still expects action and that action is now due.
    ///
    /// A live ticket with **no** deadline counts as overdue. "Nobody said when
    /// to check this" is not "never check this", and reading an absent deadline
    /// as never-due is how a single malformed row silences the sweeper
    /// permanently. Sweeping early is noisy; not sweeping is the failure.
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

/// Whether `candidate` should replace `current` as an actor's latest word.
///
/// Recency decides it. The interesting case is a tie, which is ordinary rather
/// than exotic: Nostr timestamps are whole seconds, so two rows from one actor
/// in the same second is normal and [`StatusRow`] carries nothing finer.
///
/// On a tie the rule is "stay watched": prefer the live row, then the tighter
/// deadline. Deliberately NOT ordered by state rank — rank and deadline
/// tightness are independent, so preferring a rank would discard the tighter
/// deadline and produce exactly the silent ticket this is meant to prevent.
fn supersedes(candidate: &StatusRow, current: &StatusRow) -> bool {
    if candidate.created_at != current.created_at {
        return candidate.created_at > current.created_at;
    }
    match (candidate.is_live(), current.is_live()) {
        (true, false) => return true,
        (false, true) => return false,
        _ => {}
    }
    match (candidate.deadline, current.deadline) {
        (Some(c), Some(k)) => c < k,
        (Some(_), None) => true,
        _ => false,
    }
}

fn reason_or_placeholder(r: &Option<String>) -> String {
    r.clone().unwrap_or_else(|| MISSING_REASON.to_string())
}

/// Resolve a ticket's current state from every actor's status row.
///
/// Rules, in order:
///
/// 1. **Only an [`Authority`] can end it.** An [`Outcome`] row from anyone else
///    is *demoted* to the matching participation value (`Done` → `Delivered`,
///    `Archived` → `Abandoned`) and its author is named in
///    [`TicketView::unauthorized_close`]. Obeying it is the bug; dropping it
///    silently is the other bug.
/// 2. **Among authorised outcomes the latest wins**, and on a same-instant tie
///    **completion beats abandonment**. If work finished in the very second
///    someone gave up, the work happened — telling a person their request was
///    abandoned, with a reason that is not true, is worse than a late "done".
/// 3. **Otherwise the ticket is live.** Its state is the furthest-advanced live
///    participation; ties break on recency.
/// 4. **Every actor having abandoned means [`TicketState::Unowned`]** — still
///    live, still swept, and needing reassignment.
/// 5. **The deadline is the EARLIEST** among live rows. A watchdog must fire on
///    the first thing that should have happened; taking the latest would let
///    one long-running actor mask another's stall. A live row naming no
///    deadline falls back to `open_deadline`, so acknowledging can never make a
///    ticket less watched than silence.
pub fn fold(rows: &[StatusRow], open_deadline: Option<i64>, authority: &Authority) -> TicketView {
    if rows.is_empty() {
        return TicketView::open(open_deadline);
    }

    // One row per actor: the latest they published.
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

    let mut unauthorized_close = Vec::new();
    let mut abandoned = Vec::new();
    let mut authorized_outcomes: Vec<&StatusRow> = Vec::new();
    let mut live: Vec<&StatusRow> = Vec::new();

    for r in latest.values() {
        match &r.kind {
            RowKind::Outcome(_) if authority.may_close(&r.actor) => authorized_outcomes.push(r),
            RowKind::Outcome(o) => {
                // Demoted, not obeyed — and never silently.
                unauthorized_close.push(r.actor.clone());
                match o {
                    Outcome::Done(_) => live.push(r),
                    Outcome::Archived => {
                        abandoned.push((r.actor.clone(), reason_or_placeholder(&r.reason)))
                    }
                }
            }
            RowKind::Participation(Participation::Abandoned) => {
                abandoned.push((r.actor.clone(), reason_or_placeholder(&r.reason)))
            }
            RowKind::Participation(_) => live.push(r),
        }
    }

    abandoned.sort();
    unauthorized_close.sort();

    // Rule 2: latest authorised outcome, completion winning a same-instant tie.
    let closing = authorized_outcomes
        .iter()
        .max_by_key(|r| {
            let rank = match &r.kind {
                RowKind::Outcome(Outcome::Done(_)) => 0u8,
                _ => 1u8,
            };
            (r.created_at, std::cmp::Reverse(rank))
        })
        .copied();

    if let Some(c) = closing {
        let (state, confirmation) = match &c.kind {
            RowKind::Outcome(Outcome::Done(conf)) => (TicketState::Done, Some(conf.clone())),
            _ => (TicketState::Archived, None),
        };
        let interrupted: Vec<String> = live.iter().map(|r| r.actor.clone()).collect();
        return TicketView {
            state,
            deadline: None,
            decided_by: Some(c.actor.clone()),
            reason: match state {
                TicketState::Archived => Some(reason_or_placeholder(&c.reason)),
                _ => c.reason.clone(),
            },
            confirmation,
            abandoned,
            unauthorized_close,
            interrupted,
        };
    }

    // Rule 5: earliest live deadline, with the open deadline as a floor.
    let mut deadline: Option<i64> = None;
    for r in &live {
        let d = r.deadline.or(open_deadline);
        deadline = match (deadline, d) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    // Rule 4: everyone left.
    if live.is_empty() {
        return TicketView {
            state: TicketState::Unowned,
            deadline: deadline.or(open_deadline),
            decided_by: None,
            reason: None,
            confirmation: None,
            abandoned,
            unauthorized_close,
            interrupted: Vec::new(),
        };
    }

    // Rule 3: furthest-advanced live participation; ties on recency.
    let decided = live
        .iter()
        .max_by_key(|r| {
            let p = match &r.kind {
                RowKind::Participation(p) => *p,
                // A demoted unauthorised Done reads as Delivered.
                RowKind::Outcome(_) => Participation::Delivered,
            };
            (p, r.created_at)
        })
        .expect("live is non-empty");

    let state = match &decided.kind {
        RowKind::Participation(Participation::Acknowledged | Participation::Working) => {
            TicketState::Working
        }
        RowKind::Participation(Participation::Blocked) => TicketState::Blocked,
        RowKind::Participation(Participation::Delivered) => TicketState::Delivered,
        RowKind::Participation(Participation::Abandoned) => unreachable!("filtered into abandoned"),
        RowKind::Outcome(_) => TicketState::Delivered,
    };

    TicketView {
        state,
        deadline,
        decided_by: Some(decided.actor.clone()),
        reason: decided.reason.clone(),
        confirmation: None,
        abandoned,
        unauthorized_close,
        interrupted: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUESTER: &str = "craig";
    const OWNER: &str = "reset-agent";

    fn auth() -> Authority {
        Authority {
            requester: REQUESTER.to_string(),
            owner: Some(OWNER.to_string()),
        }
    }

    fn part(actor: &str, p: Participation, at: i64, deadline: Option<i64>) -> StatusRow {
        StatusRow {
            actor: actor.to_string(),
            kind: RowKind::Participation(p),
            created_at: at,
            deadline,
            reason: None,
        }
    }

    fn gave_up(actor: &str, at: i64, why: &str) -> StatusRow {
        StatusRow {
            actor: actor.to_string(),
            kind: RowKind::Participation(Participation::Abandoned),
            created_at: at,
            deadline: None,
            reason: Some(why.to_string()),
        }
    }

    fn closed(actor: &str, o: Outcome, at: i64, why: Option<&str>) -> StatusRow {
        StatusRow {
            actor: actor.to_string(),
            kind: RowKind::Outcome(o),
            created_at: at,
            deadline: None,
            reason: why.map(str::to_string),
        }
    }

    // ── The rule this module was rebuilt for ──────────────────────────────

    #[test]
    fn one_worker_giving_up_does_not_end_the_ticket() {
        // The bug: any actor writing a terminal state closed the ticket for
        // everyone. No mature tracker allows that — a worker's exit is an
        // input to a decision, never the decision.
        let v = fold(
            &[
                gave_up("claude-session", 10, "could not reach the repository"),
                part("reviewer", Participation::Working, 12, Some(500)),
            ],
            None,
            &auth(),
        );
        assert!(
            !v.state.is_terminal(),
            "one worker leaving must not close it"
        );
        assert_eq!(v.state, TicketState::Working);
        assert_eq!(
            v.deadline,
            Some(500),
            "the remaining worker's clock still runs"
        );
        assert_eq!(
            v.abandoned,
            vec![(
                "claude-session".to_string(),
                "could not reach the repository".to_string()
            )],
            "the giving-up is recorded permanently, with its reason"
        );
    }

    #[test]
    fn everyone_leaving_makes_it_unowned_and_still_watched() {
        // The state that had no name: nobody is working and nobody closed it.
        let v = fold(
            &[
                gave_up("a", 10, "out of scope for me"),
                gave_up("b", 12, "no capacity"),
            ],
            Some(100),
            &auth(),
        );
        assert_eq!(v.state, TicketState::Unowned);
        assert!(
            !v.state.is_terminal(),
            "a ticket everyone walked away from is not finished"
        );
        assert!(v.is_overdue(100), "it must still be swept, and reassigned");
        assert_eq!(v.abandoned.len(), 2);
    }

    #[test]
    fn an_unauthorized_close_is_demoted_and_named() {
        // Obeying it is the bug. Dropping it silently is the other bug.
        let v = fold(
            &[closed(
                "claude-session",
                Outcome::Done(Confirmation::ByRequester),
                10,
                None,
            )],
            Some(100),
            &auth(),
        );
        assert!(
            !v.state.is_terminal(),
            "a worker cannot close on the requester's behalf"
        );
        assert_eq!(
            v.state,
            TicketState::Delivered,
            "demoted to awaiting confirmation"
        );
        assert_eq!(v.unauthorized_close, vec!["claude-session".to_string()]);
    }

    #[test]
    fn the_requester_can_close_and_the_owner_can_too() {
        for who in [REQUESTER, OWNER] {
            let v = fold(
                &[closed(
                    who,
                    Outcome::Done(Confirmation::ByRequester),
                    10,
                    None,
                )],
                None,
                &auth(),
            );
            assert_eq!(v.state, TicketState::Done, "{who} may close");
            assert!(v.state.is_terminal());
        }
    }

    #[test]
    fn delivered_is_live_so_an_unconfirmed_delivery_is_still_swept() {
        // Two-phase terminal: finishing is not the same as being confirmed.
        let v = fold(
            &[part(
                "claude-session",
                Participation::Delivered,
                10,
                Some(200),
            )],
            None,
            &auth(),
        );
        assert_eq!(v.state, TicketState::Delivered);
        assert!(!v.state.is_terminal());
        assert!(
            v.is_overdue(200),
            "an unconfirmed delivery must not sit unnoticed"
        );
    }

    #[test]
    fn an_unopposed_close_never_renders_as_a_human_confirmation() {
        let v = fold(
            &[closed(
                REQUESTER,
                Outcome::Done(Confirmation::Unopposed {
                    delivered_by: "claude-session".into(),
                    window_secs: 7200,
                }),
                10,
                None,
            )],
            None,
            &auth(),
        );
        assert_eq!(v.state, TicketState::Done);
        match v.confirmation {
            Some(Confirmation::Unopposed { window_secs, .. }) => assert_eq!(window_secs, 7200),
            other => panic!("the kind of close must survive the fold: {other:?}"),
        }
    }

    #[test]
    fn closing_over_live_work_names_who_was_interrupted() {
        // A close is deliberately not blocked by live work, so the orphaned
        // work has to surface somewhere.
        let v = fold(
            &[
                part("claude-session", Participation::Working, 10, Some(500)),
                closed(REQUESTER, Outcome::Archived, 20, Some("no longer needed")),
            ],
            None,
            &auth(),
        );
        assert_eq!(v.state, TicketState::Archived);
        assert_eq!(v.interrupted, vec!["claude-session".to_string()]);
        assert_eq!(v.reason.as_deref(), Some("no longer needed"));
    }

    #[test]
    fn archiving_without_a_reason_still_says_something() {
        let v = fold(
            &[closed(REQUESTER, Outcome::Archived, 10, None)],
            None,
            &auth(),
        );
        assert_eq!(v.reason.as_deref(), Some(MISSING_REASON));
    }

    // ── Deadline and sweeping guarantees ──────────────────────────────────

    #[test]
    fn no_rows_is_open_and_carries_the_ack_deadline() {
        let v = fold(&[], Some(100), &auth());
        assert_eq!(v.state, TicketState::Open);
        assert!(v.is_overdue(100));
        assert!(!v.is_overdue(99));
    }

    #[test]
    fn a_live_row_with_no_deadline_is_swept_not_ignored() {
        let v = fold(
            &[part("a", Participation::Acknowledged, 10, None)],
            None,
            &auth(),
        );
        assert!(
            v.is_overdue(i64::MAX / 2),
            "unknown deadline means check now, not never"
        );
    }

    #[test]
    fn acknowledging_never_makes_a_ticket_less_watched_than_silence() {
        let with_row = fold(
            &[part("a", Participation::Acknowledged, 10, None)],
            Some(100),
            &auth(),
        );
        assert_eq!(with_row.deadline, fold(&[], Some(100), &auth()).deadline);
    }

    #[test]
    fn deadline_is_the_earliest_live_one_not_the_latest() {
        // A four-hour build must not mask a stalled reviewer.
        let v = fold(
            &[
                part("slow", Participation::Working, 10, Some(9_000)),
                part("stalled", Participation::Acknowledged, 10, Some(60)),
            ],
            None,
            &auth(),
        );
        assert_eq!(v.deadline, Some(60));
    }

    #[test]
    fn a_closed_ticket_is_never_overdue() {
        for o in [Outcome::Done(Confirmation::ByRequester), Outcome::Archived] {
            let v = fold(&[closed(REQUESTER, o, 10, Some("x"))], Some(1), &auth());
            assert!(!v.is_overdue(i64::MAX / 2));
        }
    }

    #[test]
    fn blocked_is_not_terminal_and_keeps_its_deadline() {
        // Raising a hand is not resolving. An ignored block keeps firing.
        let v = fold(
            &[part("a", Participation::Blocked, 5, Some(50))],
            None,
            &auth(),
        );
        assert_eq!(v.state, TicketState::Blocked);
        assert!(!v.state.is_terminal());
        assert!(v.is_overdue(60));
    }

    // ── Ordering ──────────────────────────────────────────────────────────

    #[test]
    fn completion_beats_abandonment_in_the_same_instant() {
        // If work finished in the very second someone gave up, the work
        // happened. A false "we stopped" is worse than a late "done".
        let done = closed(
            REQUESTER,
            Outcome::Done(Confirmation::ByRequester),
            100,
            None,
        );
        let arch = closed(OWNER, Outcome::Archived, 100, Some("timed out"));
        for rows in [vec![done.clone(), arch.clone()], vec![arch, done]] {
            let v = fold(&rows, None, &auth());
            assert_eq!(
                v.state,
                TicketState::Done,
                "completion must win, in either arrival order"
            );
        }
    }

    #[test]
    fn same_second_ties_keep_the_actor_watched() {
        let v = fold(
            &[
                gave_up("agent", 100, "giving up"),
                part("agent", Participation::Acknowledged, 100, Some(130)),
            ],
            None,
            &auth(),
        );
        assert_eq!(
            v.state,
            TicketState::Working,
            "prefer the row that stays watched"
        );
        assert!(v.is_overdue(200));
    }

    #[test]
    fn same_second_ties_keep_the_tighter_deadline() {
        // Ordering by rank would pick the loose deadline and go silent.
        let v = fold(
            &[
                part("agent", Participation::Acknowledged, 100, Some(1000)),
                part("agent", Participation::Working, 100, Some(130)),
            ],
            None,
            &auth(),
        );
        assert_eq!(v.deadline, Some(130));
        assert!(v.is_overdue(200));
    }

    #[test]
    fn only_the_latest_row_per_actor_counts() {
        let v = fold(
            &[
                part("agent", Participation::Working, 30, Some(900)),
                part("agent", Participation::Acknowledged, 10, Some(40)),
            ],
            None,
            &auth(),
        );
        assert_eq!(
            v.deadline,
            Some(900),
            "a superseded deadline must not linger"
        );
    }

    #[test]
    fn tag_round_trips() {
        for s in [
            TicketState::Open,
            TicketState::Working,
            TicketState::Blocked,
            TicketState::Delivered,
            TicketState::Unowned,
            TicketState::Done,
            TicketState::Archived,
        ] {
            assert_eq!(TicketState::from_tag(s.as_tag()), Some(s));
        }
        assert_eq!(TicketState::from_tag("finished"), None);
    }
}
