//! A parked batch is exempt from the depth caps' oldest-first eviction
//! (spec 8.1 "park, never drop"), and what a full queue truly must shed is
//! handed back for a card and a notice rather than a log line (spec 8.6).
//!
//! `park` puts the batch back at the FRONT of its scope with its original
//! timestamps, so it is the oldest thing in the queue by construction —
//! exactly what oldest-first evicts. Before M1 the longest a batch could
//! wait was about 25 minutes of retries; a weekly limit parks for about 55
//! hours, which is long enough for a busy channel to reach its cap.

use std::time::Duration;

use nostr::{EventBuilder, Keys, Kind};
use uuid::Uuid;

use super::{
    BatchEvent, DedupMode, EventQueue, FlushBatch, Instant, QueuedEvent, SessionScope,
    MAX_PENDING_PER_CHANNEL, MAX_PENDING_PER_SCOPE,
};

fn event(content: &str) -> nostr::Event {
    EventBuilder::new(Kind::Custom(9), content)
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

fn conv(channel_id: Uuid) -> SessionScope {
    SessionScope::Conversation { channel_id }
}

fn thread(channel_id: Uuid, root: char) -> SessionScope {
    SessionScope::Thread {
        channel_id,
        root_event_id: std::iter::repeat_n(root, 64).collect(),
    }
}

fn queued(scope: &SessionScope, content: &str) -> QueuedEvent {
    QueuedEvent {
        channel_id: scope.channel_id(),
        scope: scope.clone(),
        event: event(content),
        received_at: Instant::now(),
        prompt_tag: "test".into(),
        edit: None,
        parked: false,
    }
}

/// A batch that was flushed `age` ago, as a parked one always is: the retry
/// deadline that released it is long past its arrival.
fn batch(scope: &SessionScope, events: usize, age: Duration) -> FlushBatch {
    FlushBatch {
        channel_id: scope.channel_id(),
        scope: scope.clone(),
        events: (0..events)
            .map(|i| BatchEvent {
                event: event(&format!("parked-{i}")),
                prompt_tag: "test".into(),
                received_at: Instant::now() - age,
                edit: None,
            })
            .collect(),
        cancelled_events: Vec::new(),
        cancel_reason: None,
    }
}

fn two_days() -> Duration {
    Duration::from_secs(2 * 24 * 3600)
}

fn an_hour() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

/// 16 Oct: a request parks until 19 Oct on the weekly limit, and its own
/// scope then takes a cap's worth of new events. The parked request is the
/// one thing in the queue that must not be evicted.
#[test]
fn the_per_scope_cap_passes_over_a_parked_batch() {
    let channel_id = Uuid::new_v4();
    let scope = conv(channel_id);
    let mut queue = EventQueue::new(DedupMode::Queue);
    queue.park(batch(&scope, 1, two_days()), an_hour());

    for i in 0..MAX_PENDING_PER_SCOPE {
        assert!(queue.push(queued(&scope, &format!("busy-{i}"))), "{i}");
    }

    let kept = &queue.queues[&scope];
    assert_eq!(kept.len(), MAX_PENDING_PER_SCOPE);
    assert!(
        kept.front().is_some_and(|e| e.parked),
        "the parked request is still at the front"
    );
    assert_eq!(
        kept.iter().filter(|e| e.parked).count(),
        1,
        "and it is still whole"
    );
    assert!(
        queue.take_shed_parked().is_empty(),
        "an ordinary event was given up instead"
    );
    assert_eq!(queue.drop_counts().queue_depth_cap, 1);
}

/// The aggregate per-channel cap is the one the defect's scenario trips: a
/// busy channel's other threads fill it while one thread's request waits.
#[test]
fn the_channel_cap_passes_over_a_parked_scope() {
    let channel_id = Uuid::new_v4();
    let waiting = thread(channel_id, 'a');
    let busy = thread(channel_id, 'b');
    let mut queue = EventQueue::new(DedupMode::Queue);
    queue.park(batch(&waiting, 1, two_days()), an_hour());

    for i in 0..MAX_PENDING_PER_CHANNEL {
        assert!(queue.push(queued(&busy, &format!("busy-{i}"))), "{i}");
    }

    assert_eq!(
        queue.channel_event_total(channel_id),
        MAX_PENDING_PER_CHANNEL,
        "the cap is still met"
    );
    assert_eq!(
        queue.queues[&waiting].len(),
        1,
        "the parked request outlived the busy thread's backlog"
    );
    assert!(queue.take_shed_parked().is_empty());

    // When the deadline fires the request is still there to flush.
    queue.retry_after.insert(waiting.clone(), Instant::now());
    let flushed = queue.flush_next().expect("the parked batch flushes");
    assert_eq!(flushed.scope, waiting);
    assert_eq!(flushed.events.len(), 1);
}

/// A queue holding nothing but parked work still has to shrink. What it
/// gives up is a request somebody sent, so it is handed back to be
/// dead-lettered — never dropped on a `warn!` (spec 8.6).
#[test]
fn a_shed_parked_request_is_handed_back_for_a_card() {
    let channel_id = Uuid::new_v4();
    let scope = conv(channel_id);
    let mut queue = EventQueue::new(DedupMode::Queue);
    queue.park(batch(&scope, MAX_PENDING_PER_SCOPE, two_days()), an_hour());
    assert_eq!(queue.queues[&scope].len(), MAX_PENDING_PER_SCOPE);

    assert!(queue.push(queued(&scope, "one more")));

    let shed = queue.take_shed_parked();
    assert_eq!(shed.len(), 1, "exactly what the cap needed");
    assert!(shed[0].parked);
    assert_eq!(shed[0].channel_id, channel_id);
    assert_eq!(
        queue.channel_event_total(channel_id),
        MAX_PENDING_PER_SCOPE,
        "the cap is met"
    );
    assert!(
        queue.take_shed_parked().is_empty(),
        "taken once, not handed out twice"
    );
}

/// Only `park` claims the exemption. A batch put back because no agent was
/// free is not waiting on a limit, so it is evicted as before.
#[test]
fn a_requeued_batch_claims_no_exemption() {
    let channel_id = Uuid::new_v4();
    let scope = conv(channel_id);
    let mut queue = EventQueue::new(DedupMode::Queue);
    queue.requeue_preserve_timestamps(batch(&scope, 1, two_days()));

    for i in 0..MAX_PENDING_PER_SCOPE {
        assert!(queue.push(queued(&scope, &format!("busy-{i}"))), "{i}");
    }
    assert!(
        queue.queues[&scope].iter().all(|e| !e.parked),
        "nothing was parked, so the oldest went as it always did"
    );
    assert!(queue.take_shed_parked().is_empty());
}
