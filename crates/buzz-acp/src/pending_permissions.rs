//! The permission requests this connection has received and still owes an
//! answer to.
//!
//! # Why this is its own type
//!
//! It replaced a `pending_permission_id: Option<Value>` plus a companion
//! `permission_responded: bool`. That pair had two problems, and this type
//! exists to make both unrepresentable.
//!
//! **One slot.** A single assistant message can carry several tool calls, and
//! their `session/request_permission` requests may be in flight together. With
//! one slot the second request evicted the first, and the first was never
//! answered — the agent blocks forever on a reply that can no longer be
//! addressed.
//!
//! **Two sources of truth.** "Which request is outstanding" and "has it been
//! answered" were separate fields that could disagree. Here membership *is* the
//! outstanding flag: an id is present exactly while an answer is owed.
//!
//! # Why the owning field is a field
//!
//! The read loop's most dangerous exit is not a `return`. `pool.rs` wraps the
//! prompt future in a `select!` whose control arm drops it mid-poll, so no
//! cleanup inside the loop runs. This collection therefore lives on the
//! connection, outliving the dropped future, so `cancel_with_cleanup` can still
//! answer every outstanding request.
//!
//! # A parked request is still an owed request
//!
//! A guest turn may *park* a request instead of answering it: nothing is
//! written to the agent until an approver's verdict arrives or the wait
//! expires. Parking is recorded on the request's existing entry, not in a
//! second collection, because the exits above do not care why a request is
//! unanswered. A parked id comes back from `next_owed` like any other, so a
//! dropped prompt future still gets it answered by `cancel_with_cleanup`. A
//! second collection would give every exit two places to look, and the exit
//! that missed one would leave the agent blocked forever.
//!
//! The parked state holds the only receiver a verdict for that request can
//! arrive on. Dropping the entry drops the receiver, and that is how the main
//! loop learns an ask is dead: its `send` fails.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::approval_gate::{ApprovalVerdict, ParkId, RequestDigest};

/// A request waiting on an approver.
///
/// Holds everything needed to answer it later without the original message.
/// Both option ids are the agent's own, looked up by kind when the request
/// arrived, because the ids are the agent's to choose.
#[derive(Debug)]
pub(crate) struct ParkedAsk {
    pub(crate) park_id: ParkId,
    /// Written only for a delivered allow whose digest matches.
    pub(crate) allow_option_id: String,
    /// Written for every other outcome.
    pub(crate) reject_option_id: String,
    /// Compared with every verdict before it is applied.
    pub(crate) digest: RequestDigest,
    /// For the refusal notice and the logs.
    pub(crate) tool: String,
    pub(crate) parked_at: Instant,
    pub(crate) expires_at: Instant,
    /// The only way a verdict for this request can arrive.
    pub(crate) verdict_rx: oneshot::Receiver<ApprovalVerdict>,
}

/// A parked request taken out of waiting, with what is needed to refuse it.
///
/// Its receiver is already gone by the time anyone holds one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetiredAsk {
    pub(crate) rpc_id: serde_json::Value,
    pub(crate) park_id: ParkId,
    pub(crate) digest: RequestDigest,
    pub(crate) reject_option_id: String,
    pub(crate) tool: String,
    pub(crate) parked_at: Instant,
}

/// Why a request could not be parked.
///
/// Carries no payload. Handing the rejected ask back would make the `Result`
/// large enough to trip `clippy::result_large_err`, and the caller has no use
/// for it beyond refusing the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParkRejected {
    /// The id is not owed an answer, so there is nothing to park.
    NotOwed,
    /// The id is already parked. One request waits on one verdict.
    AlreadyParked,
}

/// One request owed an answer, and the approver it is waiting on, if any.
#[derive(Debug)]
struct OwedAnswer {
    rpc_id: serde_json::Value,
    parked: Option<ParkedAsk>,
}

/// Ids of `session/request_permission` requests awaiting a response.
///
/// `serde_json::Value` because JSON-RPC 2.0 permits both numeric and string
/// ids. Order is preserved: requests are answered oldest first, so a cancel
/// resolves them in the order the agent asked.
///
/// Neither `Clone` nor `PartialEq`: a parked entry owns a oneshot receiver,
/// which has neither, and a copy of a receiver would be a second way in.
#[derive(Debug, Default)]
pub(crate) struct PendingPermissions {
    entries: Vec<OwedAnswer>,
}

impl PendingPermissions {
    /// Record a request as owed an answer.
    ///
    /// Idempotent: an id already present is not duplicated, so a retransmitted
    /// request cannot cause two responses to be written for it.
    pub(crate) fn record(&mut self, id: serde_json::Value) {
        if !self.contains(&id) {
            self.entries.push(OwedAnswer {
                rpc_id: id,
                parked: None,
            });
        }
    }

    /// Whether `id` is owed an answer, parked or not.
    ///
    /// The permission handler checks this first, so a retransmit of a request
    /// it still owes is ignored rather than answered twice or parked twice.
    pub(crate) fn contains(&self, id: &serde_json::Value) -> bool {
        self.entries.iter().any(|entry| entry.rpc_id == *id)
    }

    /// Forget one request, once its response is on the wire.
    ///
    /// Call this *after* a successful write, never before. Forgetting first
    /// means a failed write leaves this saying "answered" when nothing was
    /// sent, and the cancel path then skips the request — the deadlock this
    /// ordering exists to prevent. Siblings keep their own claim on an answer.
    ///
    /// Forgetting a parked request drops its verdict receiver, so the main
    /// loop's `send` fails from then on.
    pub(crate) fn forget(&mut self, id: &serde_json::Value) {
        self.entries.retain(|entry| entry.rpc_id != *id);
    }

    /// The oldest request still owed an answer, parked or not.
    pub(crate) fn next_owed(&self) -> Option<&serde_json::Value> {
        self.entries.first().map(|entry| &entry.rpc_id)
    }

    /// Park a request that is owed an answer.
    ///
    /// The id must already be recorded and not yet parked. It stays owed:
    /// parking changes how it will be answered, never whether.
    pub(crate) fn park(
        &mut self,
        id: &serde_json::Value,
        ask: ParkedAsk,
    ) -> Result<(), ParkRejected> {
        match self.entries.iter_mut().find(|entry| entry.rpc_id == *id) {
            Some(entry) if entry.parked.is_none() => {
                entry.parked = Some(ask);
                Ok(())
            }
            Some(_) => Err(ParkRejected::AlreadyParked),
            None => Err(ParkRejected::NotOwed),
        }
    }

    /// How many requests are waiting on an approver.
    ///
    /// The per-connection cap counts these rather than every owed id: an id
    /// that is not parked is answered within the same loop iteration.
    pub(crate) fn parked_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.parked.is_some())
            .count()
    }

    /// The soonest any parked request expires, for the read loop's next
    /// deadline.
    pub(crate) fn earliest_expiry(&self) -> Option<Instant> {
        self.entries
            .iter()
            .filter_map(|entry| entry.parked.as_ref().map(|ask| ask.expires_at))
            .min()
    }

    /// The parked request that expired first, if any has expired by `now`.
    pub(crate) fn first_expired(&self, now: Instant) -> Option<ParkId> {
        self.entries
            .iter()
            .filter_map(|entry| entry.parked.as_ref())
            .filter(|ask| ask.expires_at <= now)
            .min_by_key(|ask| ask.expires_at)
            .map(|ask| ask.park_id)
    }

    /// Stop waiting on one parked request and hand back what is needed to
    /// answer it.
    ///
    /// The id stays owed until its answer is written and [`forget`] is called:
    /// the same write-then-forget order as every other answer, so a failed
    /// write leaves it for `cancel_with_cleanup`.
    ///
    /// [`forget`]: Self::forget
    pub(crate) fn take_parked(
        &mut self,
        park_id: ParkId,
    ) -> Option<(serde_json::Value, ParkedAsk)> {
        let entry = self.entries.iter_mut().find(|entry| {
            entry
                .parked
                .as_ref()
                .is_some_and(|ask| ask.park_id == park_id)
        })?;
        let ask = entry.parked.take()?;
        Some((entry.rpc_id.clone(), ask))
    }

    /// Stop waiting on every parked request at once.
    ///
    /// Every receiver is dropped inside this call, before the caller writes
    /// anything, so the main loop's `send` fails from here on even if the
    /// writes that follow never happen. A receiver left alive across those
    /// writes would let the main loop believe it had delivered a verdict to a
    /// request that is about to be refused. The ids stay owed.
    pub(crate) fn retire_all_parked(&mut self) -> Vec<RetiredAsk> {
        self.entries
            .iter_mut()
            .filter_map(|entry| {
                let ask = entry.parked.take()?;
                Some(RetiredAsk {
                    rpc_id: entry.rpc_id.clone(),
                    park_id: ask.park_id,
                    digest: ask.digest,
                    reject_option_id: ask.reject_option_id,
                    tool: ask.tool,
                    parked_at: ask.parked_at,
                })
            })
            .collect()
    }

    /// Resolves with the first parked request whose verdict channel is ready:
    /// a verdict, or `Err` when the sender was dropped.
    ///
    /// Pending forever when nothing is parked. Lazy, because `tokio::select!`
    /// builds the future for a disabled branch too, and building it must not
    /// touch any receiver.
    ///
    /// The caller MUST `take_parked` the returned id before polling again: a
    /// tokio oneshot receiver panics if it is polled after it has completed.
    pub(crate) fn next_verdict(
        &mut self,
    ) -> impl Future<Output = (ParkId, Result<ApprovalVerdict, oneshot::error::RecvError>)> + '_
    {
        std::future::poll_fn(move |cx| {
            for entry in self.entries.iter_mut() {
                if let Some(ask) = entry.parked.as_mut() {
                    if let Poll::Ready(result) = Pin::new(&mut ask.verdict_rx).poll(cx) {
                        return Poll::Ready((ask.park_id, result));
                    }
                }
            }
            Poll::Pending
        })
    }

    /// How many requests are still owed an answer.
    ///
    /// Test-only. The per-connection cap counts parked requests
    /// ([`parked_count`](Self::parked_count)), not every owed id; gated rather
    /// than `allow(dead_code)` so the day it has a production caller is the
    /// day the gate comes off.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether every request received has been answered.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{ParkedAsk, PendingPermissions};
    use crate::approval_gate::{
        AllowVia, ApprovalDecision, ApprovalVerdict, ParkId, RequestDigest,
    };
    use serde_json::json;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::Instant;

    #[test]
    fn a_recorded_request_is_owed_until_it_is_forgotten() {
        let mut pending = PendingPermissions::default();
        assert!(
            pending.is_empty(),
            "nothing is owed before a request arrives"
        );

        pending.record(json!(1));
        assert_eq!(pending.next_owed(), Some(&json!(1)));

        pending.forget(&json!(1));
        assert!(pending.is_empty(), "an answered request is no longer owed");
    }

    /// The regression this type exists for. Under the previous single slot the
    /// second request overwrote the first, and the first was never answered —
    /// the agent blocked forever on a reply that could no longer be addressed.
    #[test]
    fn a_concurrent_request_does_not_evict_its_sibling() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(1));
        pending.record(json!(2));

        assert_eq!(pending.len(), 2, "both requests are owed an answer");

        pending.forget(&json!(2));
        assert_eq!(
            pending.next_owed(),
            Some(&json!(1)),
            "answering one request must leave its sibling still owed"
        );
    }

    /// JSON-RPC 2.0 permits string ids, so the registry must not assume numbers.
    #[test]
    fn string_and_numeric_ids_are_distinct_and_both_supported() {
        let mut pending = PendingPermissions::default();
        pending.record(json!("perm-a"));
        pending.record(json!(1));

        pending.forget(&json!("perm-a"));
        assert_eq!(
            pending.next_owed(),
            Some(&json!(1)),
            "forgetting a string id must not disturb a numeric one"
        );
    }

    #[test]
    fn recording_the_same_id_twice_owes_only_one_answer() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(7));
        pending.record(json!(7));

        assert_eq!(
            pending.len(),
            1,
            "a retransmit must not owe a second response"
        );
        pending.forget(&json!(7));
        assert!(pending.is_empty(), "one forget clears one recorded request");
    }

    #[test]
    fn forgetting_an_unknown_id_changes_nothing() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(1));
        pending.forget(&json!(99));

        assert_eq!(
            pending.next_owed(),
            Some(&json!(1)),
            "a stray forget must not drop a request that is still owed"
        );
    }

    /// A cancel drains oldest-first, so the agent sees its requests resolved in
    /// the order it asked.
    #[test]
    fn requests_are_owed_in_arrival_order() {
        let mut pending = PendingPermissions::default();
        for id in 1..=3 {
            pending.record(json!(id));
        }
        for id in 1..=3 {
            assert_eq!(pending.next_owed(), Some(&json!(id)));
            pending.forget(&json!(id));
        }
        assert!(pending.is_empty());
    }

    // ── Parked requests ──────────────────────────────────────────────────────

    fn ask(expires_in: u64) -> (ParkedAsk, oneshot::Sender<ApprovalVerdict>) {
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        (
            ParkedAsk {
                park_id: ParkId::mint(),
                allow_option_id: "allow".into(),
                reject_option_id: "reject".into(),
                digest: RequestDigest([7; 32]),
                tool: "Bash".into(),
                parked_at: now,
                expires_at: now + Duration::from_secs(expires_in),
                verdict_rx: rx,
            },
            tx,
        )
    }

    /// The dropped-future exit: `cancel_with_cleanup` answers whatever
    /// `next_owed` returns, so a parked request must be among them.
    #[test]
    fn a_parked_request_is_still_owed_so_cancel_answers_it() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(1));
        let (a, _tx) = ask(600);
        pending.park(&json!(1), a).unwrap();
        assert_eq!(pending.next_owed(), Some(&json!(1)));
        assert_eq!(pending.parked_count(), 1);
        assert!(pending.contains(&json!(1)));
    }

    #[test]
    fn park_requires_a_recorded_unparked_id() {
        let mut pending = PendingPermissions::default();
        let (a, _tx) = ask(600);
        assert!(
            pending.park(&json!(1), a).is_err(),
            "an id nobody owes cannot park"
        );
        pending.record(json!(1));
        let (a, _tx1) = ask(600);
        pending.park(&json!(1), a).unwrap();
        let (b, _tx2) = ask(600);
        assert!(
            pending.park(&json!(1), b).is_err(),
            "never park one id twice"
        );
    }

    /// Parked requests keep the write-then-forget order: a failed write after
    /// taking the parked state still leaves the id for cancel.
    #[test]
    fn taking_a_parked_state_leaves_the_id_owed_until_forgotten() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(1));
        let (a, _tx) = ask(600);
        let id = a.park_id;
        pending.park(&json!(1), a).unwrap();
        let (rpc, _) = pending.take_parked(id).unwrap();
        assert_eq!(rpc, json!(1));
        assert_eq!(pending.parked_count(), 0);
        assert_eq!(pending.next_owed(), Some(&json!(1)));
        pending.forget(&json!(1));
        assert!(pending.next_owed().is_none());
    }

    /// The exit drain retires before it writes, so the main loop sees every
    /// ask die even when the writes to the agent then fail.
    #[test]
    fn retiring_drops_every_receiver_and_keeps_ids_owed() {
        let mut pending = PendingPermissions::default();
        let mut txs = Vec::new();
        for id in 1..=2 {
            pending.record(json!(id));
            let (a, tx) = ask(600);
            pending.park(&json!(id), a).unwrap();
            txs.push(tx);
        }
        let retired = pending.retire_all_parked();
        assert_eq!(retired.len(), 2);
        assert!(
            txs.iter().all(|tx| tx.is_closed()),
            "the main loop must see the asks die"
        );
        assert_eq!(pending.next_owed(), Some(&json!(1)));
    }

    #[test]
    fn forgetting_a_parked_request_closes_its_verdict_channel() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(1));
        let (a, tx) = ask(600);
        pending.park(&json!(1), a).unwrap();
        pending.forget(&json!(1));
        assert!(
            tx.is_closed(),
            "a forgotten request's verdict must have nowhere to land"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn expiry_queries() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(1));
        pending.record(json!(2));
        let (a, _t1) = ask(600);
        let (b, _t2) = ask(300);
        let b_id = b.park_id;
        pending.park(&json!(1), a).unwrap();
        pending.park(&json!(2), b).unwrap();
        let now = Instant::now();
        assert_eq!(
            pending.earliest_expiry(),
            Some(now + Duration::from_secs(300))
        );
        assert_eq!(pending.first_expired(now), None);
        assert_eq!(
            pending.first_expired(now + Duration::from_secs(300)),
            Some(b_id)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn next_verdict_returns_the_ready_one_and_reports_a_dropped_sender() {
        let mut pending = PendingPermissions::default();
        pending.record(json!(1));
        pending.record(json!(2));
        let (a, tx_a) = ask(600);
        let (b, tx_b) = ask(600);
        let (a_id, b_id) = (a.park_id, b.park_id);
        pending.park(&json!(1), a).unwrap();
        pending.park(&json!(2), b).unwrap();
        tx_b.send(ApprovalVerdict {
            digest: RequestDigest([7; 32]),
            decision: ApprovalDecision::Allow {
                via: AllowVia::Approver,
            },
        })
        .unwrap();
        let (id, res) = pending.next_verdict().await;
        assert_eq!(id, b_id);
        assert!(res.is_ok());
        pending.take_parked(id).unwrap();
        drop(tx_a);
        let (id, res) = pending.next_verdict().await;
        assert_eq!(id, a_id);
        assert!(
            res.is_err(),
            "a dropped sender is a closed channel, never a verdict"
        );
        pending.take_parked(id).unwrap();
        let idle = tokio::time::timeout(Duration::from_secs(1), pending.next_verdict()).await;
        assert!(
            idle.is_err(),
            "nothing parked: pending, and no poll-after-complete panic"
        );
    }
}
