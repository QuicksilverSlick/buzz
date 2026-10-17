//! How long a parked batch may wait (spec 8.1, 8.6).
//!
//! "Park, never drop" is not "wait forever". A spend cap or a lost sign-in
//! only the owner can lift, and if they are away for a weekend the old code
//! re-prompted the same request every 30 minutes indefinitely: the harness
//! stayed Blocked, the dot stayed away, no second signal was ever raised, and
//! the person who sent the request was never told it would not run.
//!
//! So every park carries a budget. When it is spent the batch dead-letters
//! through [`Factory::dropped`](super::Factory::dropped) — the owner's
//! Critical card naming the class and the provider's own words — the thread
//! gets its notice, and the harness stops claiming to be stopped.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{Class, Factory, Failure, Health, RESET_MAX};
use crate::scope::SessionScope;

/// A park that no reset time bounds — a spend cap, a lost sign-in, a rate
/// limit that will not let up — waits at most this long.
pub(super) const PARK_MAX: Duration = Duration::from_secs(24 * 3600);

/// …or this many tries, whichever comes first — the try that spends the
/// budget included. With one probe every 30 minutes
/// ([`PROBE_EVERY`](super::PROBE_EVERY)) that is about six hours of waiting
/// before the sender is told the request will not run.
pub(crate) const PARK_MAX_PROBES: u32 = 12;

/// A usage limit's own ceiling: this long past the reset the provider named.
/// It is the hard bound for that class, so a weekly limit still gets its full
/// multi-day wait and a few probes after the reset before anything is given
/// up on.
pub(super) const RESET_CEILING_GRACE: Duration = Duration::from_secs(2 * 3600);

/// One scope's park: when it began, how many tries it has had, and the
/// furthest reset the provider has named for it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Park {
    since: DateTime<Utc>,
    probes: u32,
    until: Option<DateTime<Utc>>,
}

impl Park {
    fn waited(&self, now: DateTime<Utc>) -> Duration {
        (now - self.since).to_std().unwrap_or_default()
    }
}

/// Every scope currently parked behind a limit.
pub(super) type Parks = HashMap<SessionScope, Park>;

/// What the budget says about this try.
pub(super) enum Budget {
    /// Keep waiting.
    Keep,
    /// Spent: give the batch up, with this reason for the owner's card.
    Spent { why: String },
}

impl Factory {
    /// Count this failure against `scope`'s park budget.
    ///
    /// Called once per parked try, after a hand-off has been ruled out: a
    /// hand-off re-runs the batch at once on another login, which is not
    /// waiting and so spends nothing.
    pub(super) fn spend_park_budget(
        &mut self,
        scope: &SessionScope,
        failure: &Failure,
        now: DateTime<Utc>,
    ) -> Budget {
        let park = self.parks.entry(scope.clone()).or_insert(Park {
            since: now,
            probes: 0,
            until: None,
        });
        park.probes += 1;
        // A later reset extends the ceiling; an earlier one never shortens
        // it, so a misread probe cannot cut a real weekly limit short. The
        // ceiling is still capped at the park's own start plus RESET_MAX, so
        // a reset time that keeps moving out cannot postpone this forever.
        if let Some(at) = failure.resets_at {
            let cap = park.since + ceiling(RESET_MAX);
            let at = at.min(cap);
            park.until = Some(park.until.map_or(at, |had| had.max(at)));
        }
        let park = park.clone();
        let over = match park.until {
            // A timed limit: wait out the reset it named, plus the grace.
            Some(until) => now > until + ceiling(RESET_CEILING_GRACE),
            // Nothing bounds it but the budget.
            None => park.waited(now) >= PARK_MAX || park.probes >= PARK_MAX_PROBES,
        };
        if !over {
            return Budget::Keep;
        }
        let why = self.gave_up_reason(failure, &park, now);
        self.give_up(scope, now);
        Budget::Spent { why }
    }

    /// The batch is gone, so this scope is no longer parked. With nothing
    /// else waiting, the harness is moving again — new requests are tried as
    /// usual — and the dot and the STATE record have to say so rather than
    /// stay stuck at away and BLOCKED behind work that no longer exists.
    fn give_up(&mut self, scope: &SessionScope, now: DateTime<Utc>) {
        self.parks.remove(scope);
        self.told.remove(&(scope.clone(), false));
        self.told.remove(&(scope.clone(), true));
        self.gave_up = true;
        if self.parks.is_empty() {
            self.resumes = None;
            self.set_health(Health::Degraded, now);
        }
    }

    /// Why the owner's "dropped a request" card says it was given up on: the
    /// class, how long it waited, and the provider's own (scrubbed) words
    /// (spec 8.6). No trailing stop — [`Factory::dropped`] adds one.
    fn gave_up_reason(&self, failure: &Failure, park: &Park, now: DateTime<Utc>) -> String {
        let what = match failure.class {
            Class::UsageLimit => "the AI service's usage limit never lifted",
            Class::SpendCap => "the AI service's spending cap was never raised",
            Class::SignIn => "the agent was never signed in again",
            Class::RateLimit => "the AI service would not stop limiting how fast it can work",
            Class::Other => "the AI service stayed unavailable",
        };
        let quote = super::card_quote(&self.provider_name(), &failure.text);
        format!(
            "{what} after {} and {} {}; {}",
            plain_duration(park.waited(now)),
            park.probes,
            if park.probes == 1 { "try" } else { "tries" },
            quote.trim_end_matches('.')
        )
    }

    /// Nothing is parked behind a limit any more: every budget is forgotten,
    /// so the next incident starts from a full one.
    pub(super) fn clear_parks(&mut self) {
        self.parks.clear();
        self.gave_up = false;
    }

    #[cfg(test)]
    pub(super) fn parks(&self) -> &Parks {
        &self.parks
    }
}

/// `Duration` as a `chrono::Duration`, or zero when it will not fit.
fn ceiling(d: Duration) -> chrono::Duration {
    chrono::Duration::from_std(d).unwrap_or_default()
}

/// A wait as the owner reads it.
fn plain_duration(waited: Duration) -> String {
    let minutes = waited.as_secs() / 60;
    let plural = |n: u64, unit: &str| {
        if n == 1 {
            format!("about 1 {unit}")
        } else {
            format!("about {n} {unit}s")
        }
    };
    match minutes {
        0..=90 => plural(minutes.max(1), "minute"),
        91..=1439 => plural((minutes + 30) / 60, "hour"),
        _ => plural((minutes + 720) / 1440, "day"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpError;
    use crate::factory::{classify, Plan};
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn scope() -> SessionScope {
        SessionScope::Conversation {
            channel_id: uuid::Uuid::from_u128(7),
        }
    }

    fn other_scope() -> SessionScope {
        SessionScope::Conversation {
            channel_id: uuid::Uuid::from_u128(8),
        }
    }

    fn failure(message: &str, now: DateTime<Utc>) -> Failure {
        classify(&AcpError::agent_error(-32603, message), now)
    }

    fn spend_cap(now: DateTime<Utc>) -> Failure {
        failure(
            "Internal error: You've hit your monthly spend limit · raise it at \
             claude.ai/settings/usage",
            now,
        )
    }

    fn sign_in(now: DateTime<Utc>) -> Failure {
        failure(
            "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
            now,
        )
    }

    fn factory() -> Factory {
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting.agent = "Support Agent".to_string();
        f.reporting.short = "ade9d0e2".to_string();
        f
    }

    /// 17 Oct 13:15: the spend cap is hit and the owner is away for the
    /// weekend. The old code re-prompted the same request every 30 minutes
    /// for as long as the harness lived, stayed Blocked with the dot at
    /// away, and never told the sender. Now the park has a budget, and when
    /// it is spent the request dead-letters with the owner's Critical card.
    #[test]
    fn a_spend_cap_nobody_raises_ends_in_a_dead_letter() {
        let start = at(2026, 10, 17, 13, 15);
        let mut f = factory();
        let cap = spend_cap(start);
        let probe = |n: u32| start + chrono::Duration::minutes(30 * i64::from(n));
        for n in 0..PARK_MAX_PROBES - 1 {
            let plan = f.plan(0, &scope(), &cap, probe(n));
            assert!(matches!(plan, Plan::Park { .. }), "probe {n}: {plan:?}");
            assert_eq!(f.presence(), "away", "probe {n}");
        }
        let plan = f.plan(0, &scope(), &cap, probe(PARK_MAX_PROBES - 1));
        let Plan::DeadLetter { why } = plan else {
            panic!("the budget is spent, so expected a dead letter, got {plan:?}");
        };
        // The owner's card names the class, the wait, and Claude's own words.
        assert_eq!(
            why,
            "the AI service's spending cap was never raised after about 6 hours and 12 tries; \
             Claude said: \"You've hit your monthly spend limit · raise it at \
             claude.ai/settings/usage\""
        );

        // Nothing is parked behind the limit now, so the harness stops
        // claiming to be stopped: the dot is online and the record says why.
        assert!(f.parks().is_empty());
        assert_eq!(f.presence(), "online");
        assert_eq!(f.health, Health::Degraded);
        let record = f
            .take_report(probe(PARK_MAX_PROBES - 1))
            .expect("the recovery is reported");
        assert!(
            record.contains("state=DEGRADED class=spend_cap"),
            "{record}"
        );
        assert!(record.contains("gave up on a request"), "{record}");

        // A fresh request after that starts on a full budget again.
        let later = probe(PARK_MAX_PROBES);
        assert!(matches!(
            f.plan(0, &scope(), &cap, later),
            Plan::Park { .. }
        ));
    }

    /// A timed limit is bounded by the reset it named, not by the probe
    /// count: a weekly limit outlasts both the 24-hour and the 12-try bound
    /// and must still get its full wait, plus a few probes after the reset.
    #[test]
    fn a_usage_limit_is_bounded_by_its_own_reset_plus_a_grace() {
        let start = at(2026, 10, 16, 16, 52);
        let mut f = factory();
        let weekly = failure(
            "Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)",
            start,
        );
        let reset = weekly.resets_at.expect("the text names a reset");
        for (now, what) in [
            (start, "the first failure"),
            // 30 hours in: past PARK_MAX, but the reset is the ceiling here.
            (
                start + chrono::Duration::hours(30),
                "still before the reset",
            ),
            (
                reset + chrono::Duration::minutes(2),
                "the first probe after",
            ),
            (reset + chrono::Duration::minutes(90), "still in the grace"),
        ] {
            let plan = f.plan(0, &scope(), &weekly, now);
            assert!(matches!(plan, Plan::Park { .. }), "{what}: {plan:?}");
        }
        let past = reset + ceiling(RESET_CEILING_GRACE) + chrono::Duration::minutes(1);
        let plan = f.plan(0, &scope(), &weekly, past);
        let Plan::DeadLetter { why } = plan else {
            panic!("past the reset and its grace, expected a dead letter, got {plan:?}");
        };
        assert!(
            why.starts_with("the AI service's usage limit never lifted"),
            "{why}"
        );
        assert!(why.contains("You've hit your weekly limit"), "{why}");
        assert_eq!(f.presence(), "online");
    }

    /// With no reset to wait for, time bounds the park as well as tries: two
    /// probes a day apart spend the budget just as 12 probes in six hours do.
    #[test]
    fn a_park_with_no_reset_is_bounded_by_time_as_well_as_tries() {
        let start = at(2026, 10, 17, 13, 15);
        let mut f = factory();
        let lost = sign_in(start);
        assert!(matches!(
            f.plan(0, &scope(), &lost, start),
            Plan::Park { .. }
        ));
        assert_eq!(f.health, Health::Down, "a lost sign-in is Down");
        let next_day = start + chrono::Duration::hours(25);
        let plan = f.plan(0, &scope(), &lost, next_day);
        let Plan::DeadLetter { why } = plan else {
            panic!("past PARK_MAX, expected a dead letter, got {plan:?}");
        };
        assert!(
            why.starts_with("the agent was never signed in again"),
            "{why}"
        );
        assert!(why.contains("about 1 day and 2 tries"), "{why}");
        assert_eq!(f.presence(), "online");
        assert_eq!(f.health, Health::Degraded);
    }

    /// Giving up on one request does not pretend the others are moving: a
    /// scope still parked keeps the harness stopped and the dot away.
    #[test]
    fn another_parked_scope_keeps_the_harness_stopped() {
        let start = at(2026, 10, 17, 13, 15);
        let mut f = factory();
        let cap = spend_cap(start);
        f.plan(0, &other_scope(), &cap, start);
        for n in 0..PARK_MAX_PROBES {
            f.plan(
                0,
                &scope(),
                &cap,
                start + chrono::Duration::minutes(30 * i64::from(n)),
            );
        }
        assert_eq!(f.parks().len(), 1, "the other scope is still parked");
        assert_eq!(f.health, Health::Blocked);
        assert_eq!(f.presence(), "away");
    }

    /// A real answer clears every budget: the limit lifted, so the next one
    /// starts over rather than inheriting the last incident's tries.
    #[test]
    fn a_real_answer_clears_the_budget() {
        let start = at(2026, 10, 17, 13, 15);
        let mut f = factory();
        let cap = spend_cap(start);
        for n in 0..PARK_MAX_PROBES - 1 {
            f.plan(
                0,
                &scope(),
                &cap,
                start + chrono::Duration::minutes(30 * i64::from(n)),
            );
        }
        f.on_success(0, start + chrono::Duration::hours(7));
        assert!(f.parks().is_empty());
        assert!(!f.gave_up);
        let after = start + chrono::Duration::hours(8);
        assert!(matches!(
            f.plan(0, &scope(), &spend_cap(after), after),
            Plan::Park { .. }
        ));
    }

    #[test]
    fn plain_duration_reads_as_the_owner_would_say_it() {
        let cases = [
            (0, "about 1 minute"),
            (60, "about 1 minute"),
            (30 * 60, "about 30 minutes"),
            (90 * 60, "about 90 minutes"),
            (91 * 60, "about 2 hours"),
            (6 * 3600, "about 6 hours"),
            (23 * 3600, "about 23 hours"),
            (24 * 3600, "about 1 day"),
            (25 * 3600, "about 1 day"),
            (55 * 3600, "about 2 days"),
        ];
        for (secs, want) in cases {
            assert_eq!(plain_duration(Duration::from_secs(secs)), want, "{secs}s");
        }
    }
}
