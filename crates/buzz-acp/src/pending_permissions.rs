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

/// Ids of `session/request_permission` requests awaiting a response.
///
/// `serde_json::Value` because JSON-RPC 2.0 permits both numeric and string
/// ids. Order is preserved: requests are answered oldest first, so a cancel
/// resolves them in the order the agent asked.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct PendingPermissions {
    ids: Vec<serde_json::Value>,
}

impl PendingPermissions {
    /// Record a request as owed an answer.
    ///
    /// Idempotent: an id already present is not duplicated, so a retransmitted
    /// request cannot cause two responses to be written for it.
    pub(crate) fn record(&mut self, id: serde_json::Value) {
        if !self.ids.contains(&id) {
            self.ids.push(id);
        }
    }

    /// Forget one request, once its response is on the wire.
    ///
    /// Call this *after* a successful write, never before. Forgetting first
    /// means a failed write leaves this saying "answered" when nothing was
    /// sent, and the cancel path then skips the request — the deadlock this
    /// ordering exists to prevent. Siblings keep their own claim on an answer.
    pub(crate) fn forget(&mut self, id: &serde_json::Value) {
        self.ids.retain(|pending| pending != id);
    }

    /// The oldest request still owed an answer.
    pub(crate) fn next_owed(&self) -> Option<&serde_json::Value> {
        self.ids.first()
    }

    /// How many requests are still owed an answer.
    ///
    /// Test-only for now. The approval leg will read it for the concurrency
    /// cap; gated rather than `allow(dead_code)` so the day it has a
    /// production caller is the day the gate comes off.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether every request received has been answered.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::PendingPermissions;
    use serde_json::json;

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
}
