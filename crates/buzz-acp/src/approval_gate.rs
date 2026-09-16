//! The pure half of parking a permission request until an approver answers.
//!
//! # Why this exists
//!
//! A guest turn used to refuse every permission request outright. Many of
//! those are requests the owner would allow, so a guest turn can now *park* a
//! request instead: write nothing to the agent yet, hand the request to the
//! main loop, and wait for a verdict. This module holds every rule that
//! decides how that goes (whether a request may park, how long it may wait,
//! which verdict counts, and how waiting bends the turn's deadlines) and none
//! of the I/O.
//!
//! It is its own module for two reasons.
//!
//! **Testable everywhere.** The read loop can only be exercised against a
//! spawned agent, and those tests shell out to bash, which the Windows build
//! cannot run. Kept pure, every rule here is a plain unit test on every
//! platform.
//!
//! **One owner of the contract.** The read loop and the main loop meet at the
//! types below: an [`ApprovalEvent`] goes out, an [`ApprovalVerdict`] comes
//! back, and a [`RequestDigest`] binds the two. Both sides import them from
//! here and nowhere else. Two digest implementations that disagreed by a
//! single byte would refuse every verdict as a mismatch, and two
//! [`DenyReason`]s would need a translation that could silently drop a case.
//!
//! # What makes a verdict trustworthy
//!
//! - It arrives only on the oneshot created for that one request. Neither the
//!   agent nor a guest can write to it, and it cannot reach another request.
//! - It is applied only when its digest matches the parked request's. The
//!   main loop binds its ticket to that digest, so a verdict cross-wired to
//!   the wrong request is refused instead of trusted.
//! - Every wait is bounded. An unanswered request expires to `reject_once`,
//!   and parking is capped per connection and per turn.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::time::Instant;

/// How long a parked request waits for an approver before it is refused.
///
/// Long enough for the owner to see a notification and answer from a phone,
/// short enough that an abandoned ask does not hold a turn open for long.
pub(crate) const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

/// Most requests one connection may have parked at once.
///
/// One assistant message can carry several tool calls, so more than one can
/// legitimately be waiting. Past a handful, the agent is asking faster than a
/// person can answer, and refusing is kinder than stacking up cards.
pub(crate) const MAX_PARKED_PER_CONNECTION: usize = 4;

/// Most asks one turn may put in front of a person.
///
/// Counts only asks that paged someone. A request a standing grant answers is
/// refunded, or a one-hour grant would stop working after six calls.
pub(crate) const MAX_ASKS_PER_TURN: u8 = 6;

/// How far inside the turn's hard deadline an ask must expire.
///
/// An expiry answers the agent `reject_once` and lets the turn carry on. The
/// margin leaves room for that answer to land before the hard deadline ends
/// the turn outright.
pub(crate) const APPROVAL_HARD_MARGIN: Duration = Duration::from_secs(10);

/// The shortest window worth asking in.
///
/// Nobody can read and answer a card faster, so a shorter window is refused up
/// front rather than posted and left to expire.
pub(crate) const MIN_ASK_WINDOW: Duration = Duration::from_secs(30);

/// How many approval timeouts of waiting one turn may be credited in total.
///
/// Waiting extends the turn's hard deadline (see [`TurnClock`]). The cap stops
/// a guest from stretching one turn's wall clock without limit by asking over
/// and over.
pub(crate) const CREDIT_CAP_TIMEOUTS: u32 = 3;

/// Domain separator for [`request_digest`], so a digest of a permission
/// request can never equal a SHA-256 taken over the same bytes for another
/// purpose.
const DIGEST_DOMAIN: &[u8] = b"buzz-acp/permission-digest/v1\0";

/// The caps and windows one turn's parking runs under.
///
/// A value rather than the constants themselves so tests can shrink the
/// windows. Production always builds it with [`GateLimits::production`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GateLimits {
    pub(crate) approval_timeout: Duration,
    pub(crate) max_parked: usize,
    pub(crate) max_asks_per_turn: u8,
    pub(crate) hard_margin: Duration,
    pub(crate) min_ask_window: Duration,
    pub(crate) credit_cap: Duration,
}

impl GateLimits {
    /// The production limits for a given approval timeout.
    ///
    /// The credit cap follows the timeout, so a longer window still buys the
    /// same number of full waits per turn.
    pub(crate) fn production(approval_timeout: Duration) -> Self {
        Self {
            approval_timeout,
            max_parked: MAX_PARKED_PER_CONNECTION,
            max_asks_per_turn: MAX_ASKS_PER_TURN,
            hard_margin: APPROVAL_HARD_MARGIN,
            min_ask_window: MIN_ASK_WINDOW,
            credit_cap: approval_timeout.saturating_mul(CREDIT_CAP_TIMEOUTS),
        }
    }
}

impl Default for GateLimits {
    fn default() -> Self {
        Self::production(DEFAULT_APPROVAL_TIMEOUT)
    }
}

/// SHA-256 over one permission request, binding a verdict to exactly the
/// request the approver was shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RequestDigest(pub(crate) [u8; 32]);

impl RequestDigest {
    /// Hex form, for logs and the observer feed.
    pub(crate) fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

/// Names one parked request for the life of the process.
///
/// Minted from a process-wide counter and never reused, so a late
/// [`ApprovalSettled`] can never be mistaken for a newer request's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ParkId(pub(crate) u64);

static NEXT_PARK_ID: AtomicU64 = AtomicU64::new(1);

impl ParkId {
    pub(crate) fn mint() -> Self {
        ParkId(NEXT_PARK_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Who allowed a request.
///
/// Carried through to the settlement because a standing grant's allow is
/// refunded against the turn's ask budget and a person's is not.
// `expect`, not `allow`: the day the main loop's approval route builds or
// reads these, the expectation fails the build until the attribute goes.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the main loop's approval route builds these; until it lands only tests do"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllowVia {
    /// A person answered this request.
    Approver,
    /// A grant the owner gave earlier covered it, and nobody was paged.
    StandingGrant,
}

impl AllowVia {
    /// The name the observer feed reports.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AllowVia::Approver => "approver",
            AllowVia::StandingGrant => "standing_grant",
        }
    }
}

/// Why the main loop refused a request.
///
/// Owned here with the rest of the contract. The main loop adds its reasons
/// to this enum rather than keeping its own, so no reason is ever lost in a
/// translation between two sets.
#[expect(
    dead_code,
    reason = "the main loop's approval route builds these; tests build only some"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DenyReason {
    /// An approver said no.
    Approver,
    /// Nobody holds the authority to approve for this agent.
    NoApprover,
    /// The ask could not be delivered to the approver.
    Undeliverable,
    /// The request could not be shown faithfully, so nobody can be asked to
    /// approve it.
    Unrenderable,
    /// The answer could not be tied to exactly one request.
    Ambiguous,
    /// The harness is shutting down.
    Shutdown,
}

/// The main loop's answer to one ask.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the main loop's approval route builds these; until it lands only tests do"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalDecision {
    Allow { via: AllowVia },
    Deny(DenyReason),
}

/// A verdict on its way back to the read loop.
///
/// Carries the digest the main loop bound its ticket to. The read loop
/// compares it with the parked request's own before acting, so a verdict can
/// only ever allow the request it was issued for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ApprovalVerdict {
    pub(crate) digest: RequestDigest,
    pub(crate) decision: ApprovalDecision,
}

/// How a parked request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settlement {
    /// `allow_once` was written to the agent.
    Allowed { via: AllowVia },
    /// The main loop refused it, and `reject_once` was written.
    Denied(DenyReason),
    /// Nobody answered in time, and `reject_once` was written.
    Expired,
    /// The main loop dropped the verdict sender without answering. Refused at
    /// once rather than left to wait out its expiry.
    VerdictChannelClosed,
    /// The verdict carried another request's digest. Refused, and a bug.
    DigestMismatch,
    /// The turn ended while it waited, and `reject_once` was written.
    TurnEnded,
}

impl Settlement {
    /// The name the observer feed reports.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Settlement::Allowed { .. } => "allowed",
            Settlement::Denied(_) => "denied",
            Settlement::Expired => "expired",
            Settlement::VerdictChannelClosed => "verdict_channel_closed",
            Settlement::DigestMismatch => "digest_mismatch",
            Settlement::TurnEnded => "turn_ended",
        }
    }
}

/// Why a permission request was refused, kept so the layer above can say so.
///
/// The first six are decided before anything is parked. The rest are the ways
/// a parked request can end without being allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefusalReason {
    /// The agent offered no `allow_once`, so there was nothing to approve.
    NoAllowOffered,
    /// The request needed an approver, and this turn had no way to reach one.
    NoApprovalPath,
    /// The request named a session other than the one this turn is running.
    SessionMismatch,
    /// This connection already had the most requests it may park.
    TooManyParked,
    /// This turn had already asked a person the most times it may.
    AskBudgetExhausted,
    /// Too little of the turn was left to give a person time to answer.
    NoTimeBudget,
    Denied(DenyReason),
    Expired,
    VerdictChannelClosed,
    DigestMismatch,
    TurnEnded,
}

/// One parked request, handed to the main loop to find an approver.
///
/// Sent exactly once per parked request. The read loop keeps the receiving
/// end of `verdict_tx`, so whatever the main loop sends on it can only ever
/// reach this request.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the main loop's approval route reads these; until it lands only tests do"
    )
)]
#[derive(Debug)]
pub(crate) struct ApprovalAsk {
    /// Pairs this ask with its later [`ApprovalSettled`].
    pub(crate) park_id: ParkId,
    /// The turn that parked it, so the main loop can drop the turn's asks
    /// when the turn ends.
    pub(crate) turn_id: String,
    pub(crate) session_id: String,
    /// What the main loop binds its ticket to, and echoes in the verdict.
    pub(crate) digest: RequestDigest,
    /// `params.toolCall` exactly as the agent sent it, to render the card.
    pub(crate) tool_call: serde_json::Value,
    pub(crate) parked_at: Instant,
    /// When the read loop refuses the request if no verdict has arrived.
    ///
    /// Authoritative. Anything shown to the approver must come from this, not
    /// from a configured timeout: the read loop may have clamped it to fit
    /// the turn's hard deadline.
    pub(crate) expires_at: Instant,
    /// The one way back in.
    pub(crate) verdict_tx: tokio::sync::oneshot::Sender<ApprovalVerdict>,
}

/// Tells the main loop how a parked request ended.
///
/// Sent only after the answer is on the wire, so `Allowed` means the agent
/// really was told `allow_once`. A successful `send` of a verdict promises no
/// such thing: the write that follows it can still fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalSettled {
    pub(crate) park_id: ParkId,
    pub(crate) turn_id: String,
    pub(crate) settlement: Settlement,
    pub(crate) waited: Duration,
}

/// Everything the read loop tells the main loop about parked requests.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the main loop's approval route reads these; until it lands only tests do"
    )
)]
#[derive(Debug)]
pub(crate) enum ApprovalEvent {
    Ask(ApprovalAsk),
    Settled(ApprovalSettled),
}

/// One turn's capability to park requests, built by the dispatcher.
///
/// Passed to the prompt call as a parameter rather than installed on the
/// connection. A link left on the connection would be picked up by whichever
/// read loop ran next, and a session's `initial_message` runs its own read
/// loop before the turn's, so the first guest turn of every new session could
/// never park.
#[derive(Debug, Clone)]
pub(crate) struct ApprovalLink {
    /// Unbounded so handing off an ask never awaits. A bounded send could
    /// stall the one loop that has to keep answering the agent.
    pub(crate) events_tx: tokio::sync::mpsc::UnboundedSender<ApprovalEvent>,
    pub(crate) turn_id: String,
    pub(crate) limits: GateLimits,
}

/// A turn's link plus the number of people it has asked so far.
///
/// Built only inside the gated prompt read loop. The setup loop and the
/// cancel drain never hold one, which is what makes parking impossible there:
/// neither is a turn, so neither has anyone to ask.
#[derive(Debug)]
pub(crate) struct ParkGate {
    link: ApprovalLink,
    asks_charged: u8,
}

impl ParkGate {
    pub(crate) fn new(link: ApprovalLink) -> Self {
        Self {
            link,
            asks_charged: 0,
        }
    }

    pub(crate) fn limits(&self) -> &GateLimits {
        &self.link.limits
    }

    pub(crate) fn turn_id(&self) -> &str {
        &self.link.turn_id
    }

    pub(crate) fn asks_charged(&self) -> u8 {
        self.asks_charged
    }

    pub(crate) fn charge(&mut self) {
        self.asks_charged = self.asks_charged.saturating_add(1);
    }

    pub(crate) fn refund(&mut self) {
        self.asks_charged = self.asks_charged.saturating_sub(1);
    }

    /// Hand an event to the main loop.
    ///
    /// `false` means the main loop is gone. The caller then refuses at once
    /// instead of waiting out an ask nobody will ever see.
    pub(crate) fn send(&self, event: ApprovalEvent) -> bool {
        self.link.events_tx.send(event).is_ok()
    }
}

/// Whether a request that needs an approver may park.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Park { expires_at: Instant },
    Refuse(RefusalReason),
}

/// Decide whether a request that needs an approver may park.
///
/// The session check comes first: a request for another session is never
/// parked, whatever the caps say. The time budget comes last, so a request
/// refused by a cap is reported as that cap.
pub(crate) fn admit(
    parked_now: usize,
    asks_charged: u8,
    session_matches: bool,
    ask_expiry: Option<Instant>,
    limits: &GateLimits,
) -> Admission {
    if !session_matches {
        return Admission::Refuse(RefusalReason::SessionMismatch);
    }
    if parked_now >= limits.max_parked {
        return Admission::Refuse(RefusalReason::TooManyParked);
    }
    if asks_charged >= limits.max_asks_per_turn {
        return Admission::Refuse(RefusalReason::AskBudgetExhausted);
    }
    match ask_expiry {
        Some(expires_at) => Admission::Park { expires_at },
        None => Admission::Refuse(RefusalReason::NoTimeBudget),
    }
}

/// What became of a parked request's verdict channel.
#[derive(Debug)]
pub(crate) enum VerdictOutcome {
    Delivered(ApprovalVerdict),
    /// The sender was dropped without a verdict.
    Closed,
    /// No verdict had arrived by the expiry.
    Expired,
    /// The turn ended first.
    TurnEnded,
}

/// What to write to the agent for one outcome, and what to report about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Resolution {
    /// Write `allow_once`. Otherwise write `reject_once`.
    pub(crate) allow: bool,
    pub(crate) settlement: Settlement,
    pub(crate) refusal: Option<RefusalReason>,
    /// Give the ask back to the turn's budget.
    pub(crate) refund_ask: bool,
}

/// Map an outcome to an answer.
///
/// Total, and deny by default: exactly one shape allows, a delivered `Allow`
/// whose digest matches. The digest is checked before the decision, so a
/// verdict for another request is refused whatever it says.
pub(crate) fn resolve(parked_digest: &RequestDigest, outcome: VerdictOutcome) -> Resolution {
    let deny = |settlement: Settlement, reason: RefusalReason| Resolution {
        allow: false,
        settlement,
        refusal: Some(reason),
        refund_ask: false,
    };
    match outcome {
        VerdictOutcome::Delivered(v) if v.digest != *parked_digest => {
            deny(Settlement::DigestMismatch, RefusalReason::DigestMismatch)
        }
        VerdictOutcome::Delivered(ApprovalVerdict {
            decision: ApprovalDecision::Allow { via },
            ..
        }) => Resolution {
            allow: true,
            settlement: Settlement::Allowed { via },
            refusal: None,
            refund_ask: via == AllowVia::StandingGrant,
        },
        VerdictOutcome::Delivered(ApprovalVerdict {
            decision: ApprovalDecision::Deny(r),
            ..
        }) => deny(Settlement::Denied(r), RefusalReason::Denied(r)),
        VerdictOutcome::Closed => deny(
            Settlement::VerdictChannelClosed,
            RefusalReason::VerdictChannelClosed,
        ),
        VerdictOutcome::Expired => deny(Settlement::Expired, RefusalReason::Expired),
        VerdictOutcome::TurnEnded => deny(Settlement::TurnEnded, RefusalReason::TurnEnded),
    }
}

/// Serialize `value` with object keys sorted at every depth.
///
/// Explicit rather than trusting serde_json's map order: the digest must not
/// change if some crate in the build turns on serde_json's `preserve_order`.
pub(crate) fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    out.insert(k.clone(), canonicalize(&map[k]));
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(canonicalize).collect())
            }
            other => other.clone(),
        }
    }
    serde_json::to_vec(&canonicalize(value)).expect("a serde_json::Value always serializes")
}

/// Digest one permission request, as the read loop saw it.
///
/// SHA-256 over a domain separator, then each field prefixed with its length,
/// so two different requests cannot hash the same bytes by moving a boundary
/// (`"ab"` + `"c"` against `"a"` + `"bc"`). A missing string field hashes as
/// `""`, and a missing `rawInput` as JSON `null`, which differs from `{}`.
pub(crate) fn request_digest(session_id: &str, tool_call: &serde_json::Value) -> RequestDigest {
    let text = |key: &str| tool_call.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let raw_input = canonical_json(
        tool_call
            .get("rawInput")
            .unwrap_or(&serde_json::Value::Null),
    );
    let mut hasher = Sha256::new();
    hasher.update(DIGEST_DOMAIN);
    for field in [
        session_id.as_bytes(),
        text("toolCallId").as_bytes(),
        text("kind").as_bytes(),
        text("title").as_bytes(),
        raw_input.as_slice(),
    ] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    let out = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&out);
    RequestDigest(digest)
}

/// A short name for the tool, for people: its title, else its kind, else
/// `(untitled)`.
pub(crate) fn tool_label(tool_call: &serde_json::Value) -> String {
    tool_call
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| tool_call.get("kind").and_then(|v| v.as_str()))
        .unwrap_or("(untitled)")
        .to_string()
}

/// Which of the read loop's deadlines comes due next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fire {
    /// The agent has been silent for the idle timeout.
    Idle,
    /// The turn reached its hard deadline, credit included.
    Hard,
    /// A parked request reached its expiry. It is refused and the turn
    /// carries on.
    Approval,
}

/// The read loop's deadlines, with waiting on an approver taken into account.
///
/// Replaces three loose locals (the idle deadline, the hard deadline and the
/// time of the last activity) because parking changes how all three behave,
/// and the rules are easier to get right, and to test, in one place that is
/// handed the time instead of reading it.
///
/// - **Idle is suspended while anything is parked.** The agent is silent
///   because it is waiting on us, not because it is stuck.
/// - **Idle restarts from the moment a wait ends,** not from the agent's last
///   line before it. Otherwise a turn that waited longer than the idle
///   timeout would die the instant the owner said yes.
/// - **Hard is credited for the time spent waiting,** continuously while the
///   wait is open, so an ask made just before the hard deadline still gets
///   its full window. The credit is capped per turn, so asking over and over
///   cannot buy unlimited wall clock.
#[derive(Debug, Clone)]
pub(crate) struct TurnClock {
    idle_timeout: Duration,
    idle_deadline: Instant,
    last_activity_at: Instant,
    hard_base: Instant,
    credit_cap: Duration,
    credit_banked: Duration,
    wait_started: Option<Instant>,
}

impl TurnClock {
    pub(crate) fn new(
        now: Instant,
        idle_timeout: Duration,
        hard_deadline: Instant,
        credit_cap: Duration,
    ) -> Self {
        Self {
            idle_timeout,
            idle_deadline: now + idle_timeout,
            last_activity_at: now,
            hard_base: hard_deadline,
            credit_cap,
            credit_banked: Duration::ZERO,
            wait_started: None,
        }
    }

    /// The agent wrote something: restart the idle window.
    pub(crate) fn on_activity(&mut self, now: Instant) {
        self.idle_deadline = now + self.idle_timeout;
        self.last_activity_at = now;
    }

    /// Record whether anything is parked right now.
    ///
    /// Call once per loop iteration, before reading any deadline. The change
    /// to nothing parked banks the wait's credit and restarts idle from `now`.
    pub(crate) fn sync(&mut self, now: Instant, any_parked: bool) {
        match (self.wait_started, any_parked) {
            (None, true) => self.wait_started = Some(now),
            (Some(started), false) => {
                self.credit_banked = self
                    .credit_banked
                    .saturating_add(now.saturating_duration_since(started))
                    .min(self.credit_cap);
                self.wait_started = None;
                self.idle_deadline = now + self.idle_timeout;
                self.last_activity_at = now;
            }
            _ => {}
        }
    }

    fn banked(&self) -> Duration {
        self.credit_banked.min(self.credit_cap)
    }

    /// Credit earned so far, a wait that is still open included.
    pub(crate) fn credit_at(&self, now: Instant) -> Duration {
        let open = self
            .wait_started
            .map(|started| now.saturating_duration_since(started))
            .unwrap_or_default();
        self.credit_banked.saturating_add(open).min(self.credit_cap)
    }

    /// The hard deadline with the credit earned so far.
    pub(crate) fn effective_hard(&self, now: Instant) -> Instant {
        self.hard_base + self.credit_at(now)
    }

    /// The hard deadline to aim at while a wait that began at `started` is
    /// open.
    ///
    /// Credit accrues as fast as time passes, so the deadline only comes due
    /// once the cap is reached. Aiming straight at that point gives the loop
    /// a fixed target instead of one that moves with every tick. The one
    /// exception is a deadline that had already passed when the wait began:
    /// no credit was owed, and it fires at once.
    fn hard_if_waiting_since(&self, started: Instant) -> Instant {
        let at_start = self.hard_base + self.banked();
        if at_start <= started {
            at_start
        } else {
            self.hard_base + self.credit_cap
        }
    }

    /// The next deadline to sleep until, and which one it is.
    ///
    /// While anything is parked, idle is not a candidate, and an expiry that
    /// ties with the hard deadline wins: refusing one request and carrying on
    /// is gentler than ending the turn. With nothing parked this is the rule
    /// the loop always had: idle when it is strictly earlier, otherwise hard.
    pub(crate) fn next_deadline(
        &self,
        now: Instant,
        earliest_expiry: Option<Instant>,
    ) -> (Instant, Fire) {
        match self.wait_started {
            Some(started) => {
                let hard = self.hard_if_waiting_since(started);
                match earliest_expiry {
                    Some(expiry) if expiry <= hard => (expiry, Fire::Approval),
                    _ => (hard, Fire::Hard),
                }
            }
            None => {
                let hard = self.effective_hard(now);
                if self.idle_deadline < hard {
                    (self.idle_deadline, Fire::Idle)
                } else {
                    (hard, Fire::Hard)
                }
            }
        }
    }

    /// Push the hard deadline out to at least `candidate`, keeping the credit
    /// already earned on top. Never moves it earlier. Returns whether it
    /// moved.
    pub(crate) fn renew_hard(&mut self, now: Instant, candidate: Instant) -> bool {
        let effective = self.effective_hard(now);
        if candidate > effective {
            self.hard_base += candidate - effective;
            true
        } else {
            false
        }
    }

    /// How long the agent has been silent, for the hard-timeout error.
    pub(crate) fn silence(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_activity_at)
    }

    /// When an ask made at `now` must expire.
    ///
    /// The full approval timeout, clamped to land `hard_margin` inside the
    /// hard deadline the ask would be waiting against. `None` when that
    /// leaves less than `min_ask_window`: asking a question nobody can answer
    /// in time is worse than refusing straight away.
    pub(crate) fn ask_expiry(&self, now: Instant, limits: &GateLimits) -> Option<Instant> {
        let hard = self.hard_if_waiting_since(self.wait_started.unwrap_or(now));
        let latest = hard.checked_sub(limits.hard_margin)?;
        let expiry = (now + limits.approval_timeout).min(latest);
        (expiry >= now + limits.min_ask_window).then_some(expiry)
    }

    #[cfg(test)]
    pub(crate) fn idle_deadline(&self) -> Instant {
        self.idle_deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn clock(now: Instant, idle: u64, hard_in: u64, cap: u64) -> TurnClock {
        TurnClock::new(now, S(idle), now + S(hard_in), S(cap))
    }

    // ── Deadlines ────────────────────────────────────────────────────────────

    #[test]
    fn idle_is_not_a_candidate_while_parked() {
        let t0 = Instant::now();
        let mut c = clock(t0, 1, 7200, 1800);
        c.sync(t0, true);
        let (at, fire) = c.next_deadline(t0 + S(5), Some(t0 + S(600)));
        assert_eq!(
            (at, fire),
            (t0 + S(600), Fire::Approval),
            "a 1s idle timeout must not fire while a person is being asked"
        );
    }

    /// The regression where a turn dies the instant the owner says yes: the
    /// idle window must restart when the wait ends, not run on from the
    /// agent's last line before it.
    #[test]
    fn resume_recomputes_idle_from_now_not_from_the_last_line() {
        let t0 = Instant::now();
        let mut c = clock(t0, 60, 7200, 1800);
        c.on_activity(t0);
        c.sync(t0 + S(1), true);
        c.sync(t0 + S(600), false);
        assert_eq!(c.idle_deadline(), t0 + S(660));
        assert_eq!(
            c.next_deadline(t0 + S(600), None),
            (t0 + S(660), Fire::Idle)
        );
    }

    /// Credit runs while the wait is open, not only once it closes, so an ask
    /// made just before the hard deadline is not killed partway through.
    #[test]
    fn hard_is_credited_for_the_wait_continuously() {
        let t0 = Instant::now();
        let mut c = clock(t0, 60, 100, 1800);
        c.sync(t0 + S(90), true);
        assert_eq!(c.effective_hard(t0 + S(500)), t0 + S(510));
        c.sync(t0 + S(390), false);
        assert_eq!(c.effective_hard(t0 + S(390)), t0 + S(400));
    }

    #[test]
    fn credit_is_capped_per_turn_across_waits() {
        let t0 = Instant::now();
        let mut c = clock(t0, 60, 7200, 1800);
        for i in 0..3u64 {
            let start = t0 + S(i * 2000);
            c.sync(start, true);
            c.sync(start + S(900), false);
        }
        assert_eq!(
            c.credit_at(t0 + S(10_000)),
            S(1800),
            "three 15-minute waits bank only the 30-minute cap"
        );
        assert_eq!(c.effective_hard(t0 + S(10_000)), t0 + S(7200 + 1800));
    }

    #[test]
    fn hard_target_is_stable_while_parked_so_the_loop_cannot_spin() {
        let t0 = Instant::now();
        let mut c = clock(t0, 60, 5, 1800);
        c.sync(t0 + S(1), true);
        let a = c.next_deadline(t0 + S(2), None);
        let b = c.next_deadline(t0 + S(900), None);
        assert_eq!(a, b, "sleep_until must not chase a moving deadline");
        assert_eq!(a, (t0 + S(5 + 1800), Fire::Hard));
    }

    /// A park that races past the hard deadline gets no credit it was never
    /// owed.
    #[test]
    fn hard_that_was_already_due_when_the_wait_began_fires_at_once() {
        let t0 = Instant::now();
        let mut c = clock(t0, 60, 5, 1800);
        c.sync(t0 + S(6), true);
        let (at, fire) = c.next_deadline(t0 + S(6), Some(t0 + S(600)));
        assert_eq!((at, fire), (t0 + S(5), Fire::Hard));
    }

    #[test]
    fn approval_wins_a_tie_with_hard() {
        let t0 = Instant::now();
        let mut c = clock(t0, 60, 100, 0);
        c.sync(t0, true);
        assert_eq!(
            c.next_deadline(t0, Some(t0 + S(100))),
            (t0 + S(100), Fire::Approval),
            "refusing one request and carrying on beats ending the turn"
        );
    }

    #[test]
    fn unparked_classification_is_todays_rule() {
        let t0 = Instant::now();
        let c = clock(t0, 60, 100, 1800);
        assert_eq!(c.next_deadline(t0, None), (t0 + S(60), Fire::Idle));
        let c = clock(t0, 100, 100, 1800);
        assert_eq!(
            c.next_deadline(t0, None),
            (t0 + S(100), Fire::Hard),
            "a tie with nothing parked goes to hard, as it always has"
        );
    }

    #[test]
    fn steer_renewal_is_monotonic_and_keeps_accrued_credit() {
        let t0 = Instant::now();
        let mut c = clock(t0, 60, 100, 1800);
        assert!(
            !c.renew_hard(t0, t0 + S(50)),
            "renewal never moves hard earlier"
        );
        assert_eq!(c.effective_hard(t0), t0 + S(100));
        c.sync(t0 + S(10), true);
        assert!(c.renew_hard(t0 + S(20), t0 + S(500)));
        assert_eq!(c.effective_hard(t0 + S(20)), t0 + S(500));
        assert_eq!(
            c.effective_hard(t0 + S(30)),
            t0 + S(510),
            "credit keeps accruing on top of a renewal"
        );
    }

    // ── Ask windows and admission ────────────────────────────────────────────

    #[test]
    fn ask_window_gets_full_timeout_near_the_hard_deadline_when_credit_remains() {
        let t0 = Instant::now();
        let c = clock(t0, 60, 100, 1800);
        let limits = GateLimits::default();
        assert_eq!(c.ask_expiry(t0 + S(90), &limits), Some(t0 + S(690)));
    }

    #[test]
    fn ask_window_is_clamped_inside_the_hard_margin_and_refused_when_too_small() {
        let t0 = Instant::now();
        let limits = GateLimits::default();
        let c = clock(t0, 60, 300, 0);
        assert_eq!(c.ask_expiry(t0, &limits), Some(t0 + S(290)));
        let c = clock(t0, 60, 35, 0);
        assert_eq!(
            c.ask_expiry(t0, &limits),
            None,
            "25s is too short to answer in, so no ask is made"
        );
    }

    #[test]
    fn admission_caps() {
        let t0 = Instant::now();
        let l = GateLimits::default();
        let e = Some(t0 + S(600));
        assert_eq!(
            admit(4, 0, true, e, &l),
            Admission::Refuse(RefusalReason::TooManyParked)
        );
        assert_eq!(
            admit(0, 6, true, e, &l),
            Admission::Refuse(RefusalReason::AskBudgetExhausted)
        );
        assert_eq!(
            admit(0, 0, false, e, &l),
            Admission::Refuse(RefusalReason::SessionMismatch)
        );
        assert_eq!(
            admit(0, 0, true, None, &l),
            Admission::Refuse(RefusalReason::NoTimeBudget)
        );
        assert_eq!(
            admit(3, 5, true, e, &l),
            Admission::Park {
                expires_at: t0 + S(600)
            }
        );
    }

    #[test]
    fn production_limits_credit_three_full_waits() {
        let l = GateLimits::default();
        assert_eq!(l.approval_timeout, DEFAULT_APPROVAL_TIMEOUT);
        assert_eq!(l.credit_cap, S(1800));
        assert_eq!(GateLimits::production(S(60)).credit_cap, S(180));
    }

    // ── Binding a verdict to its request ─────────────────────────────────────

    #[test]
    fn only_a_matching_allow_allows() {
        let d = RequestDigest([1; 32]);
        let other = RequestDigest([2; 32]);
        let v = |digest, decision| VerdictOutcome::Delivered(ApprovalVerdict { digest, decision });
        let allow = ApprovalDecision::Allow {
            via: AllowVia::Approver,
        };
        let person = resolve(&d, v(d, allow));
        assert!(person.allow && !person.refund_ask);
        let grant = resolve(
            &d,
            v(
                d,
                ApprovalDecision::Allow {
                    via: AllowVia::StandingGrant,
                },
            ),
        );
        assert!(
            grant.allow && grant.refund_ask,
            "a grant paged nobody, so it gives the ask back"
        );
        for o in [
            v(other, allow),
            v(d, ApprovalDecision::Deny(DenyReason::Approver)),
            VerdictOutcome::Closed,
            VerdictOutcome::Expired,
            VerdictOutcome::TurnEnded,
        ] {
            let r = resolve(&d, o);
            assert!(!r.allow && r.refusal.is_some() && !r.refund_ask, "{r:?}");
        }
    }

    #[test]
    fn digest_ignores_key_order_but_binds_every_field() {
        let a = json!({"toolCallId":"t1","kind":"execute","title":"Bash","rawInput":{"b":1,"a":{"y":2,"x":3}}});
        let b = json!({"rawInput":{"a":{"x":3,"y":2},"b":1},"title":"Bash","kind":"execute","toolCallId":"t1"});
        assert_eq!(request_digest("s", &a), request_digest("s", &b));
        let base = request_digest("s", &a);
        assert_ne!(base, request_digest("s2", &a), "the session must be bound");
        for (k, v) in [
            ("toolCallId", json!("t2")),
            ("kind", json!("edit")),
            ("title", json!("Bash2")),
            ("rawInput", json!({"b":1})),
        ] {
            let mut m = a.clone();
            m[k] = v;
            assert_ne!(base, request_digest("s", &m), "{k} must be bound");
        }
        let split1 = json!({"toolCallId":"ab","kind":"c"});
        let split2 = json!({"toolCallId":"a","kind":"bc"});
        assert_ne!(
            request_digest("s", &split1),
            request_digest("s", &split2),
            "fields are length-prefixed, so a moved boundary changes the digest"
        );
        let null_in = json!({"toolCallId":"t"});
        let empty_in = json!({"toolCallId":"t","rawInput":{}});
        assert_ne!(
            request_digest("s", &null_in),
            request_digest("s", &empty_in)
        );
    }

    /// The observer feed's `settlement` and `via` values are read by the
    /// desktop, so they are part of the wire and must not drift.
    #[test]
    fn settlement_names_are_the_observer_contract() {
        let names: Vec<&str> = [
            Settlement::Allowed {
                via: AllowVia::Approver,
            },
            Settlement::Denied(DenyReason::Approver),
            Settlement::Expired,
            Settlement::VerdictChannelClosed,
            Settlement::DigestMismatch,
            Settlement::TurnEnded,
        ]
        .iter()
        .map(Settlement::as_str)
        .collect();
        assert_eq!(
            names,
            [
                "allowed",
                "denied",
                "expired",
                "verdict_channel_closed",
                "digest_mismatch",
                "turn_ended"
            ]
        );
        assert_eq!(AllowVia::Approver.as_str(), "approver");
        assert_eq!(AllowVia::StandingGrant.as_str(), "standing_grant");
    }
}
