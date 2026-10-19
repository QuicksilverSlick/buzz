//! Checking the sign-in before a request has to (factory resilience spec,
//! M2 item 3).
//!
//! On 18 Oct Reset Worker's Claude sign-in expired at 3:32 PM CT and the
//! agent stayed down for three hours, because the only thing that ever looks
//! at a sign-in is a request that needs one. An idle agent's lost sign-in is
//! therefore discovered by the first person who asks it for something, which
//! is the worst possible way to find out.
//!
//! So the harness asks first. Every [`CHECK_SECS_DEFAULT`] seconds it runs
//! `claude auth status --json` for each login of the pool, in exactly the
//! environment the adapter would be launched with — a folder entry's own
//! `CLAUDE_CONFIG_DIR` and blanked token, or, for a `label=env` entry, the
//! agent's own environment and so its own long-lived token.
//!
//! ## Why that check
//!
//! It is the CLI's own answer about the one credential the adapter will
//! actually use, so it cannot disagree with the adapter about which login it
//! is judging. It costs no model usage of any kind: no prompt, no
//! `/v1/messages` call, nothing billable or metered. And `loggedIn` is a
//! boolean in a documented `--json` output rather than prose to guess at.
//!
//! Only a definite `loggedIn: false` is acted on. No CLI to ask, a timeout,
//! or output that will not parse are all [`SignIn::Unknown`] and change
//! nothing: a check that cannot tell must never be the reason a working
//! agent is marked down. The exit status is not one of those: the CLI exits
//! 1 to say the login is signed out, so the output is read whatever the
//! status, and only the output decides.
//!
//! What this check cannot prove — a token the server has revoked but the CLI
//! still holds locally — is caught instead by the failover of item 2, at
//! once, on the next real request. Proving it here would mean a billable or
//! undocumented API call, and a wrong verdict from one of those would park a
//! healthy agent's work.
//!
//! And a login that fails its check while the work is on another one is no
//! outage at all: it is marked out and said once, with no incident, no card
//! and no claim that an agent which never stopped has stopped.
//!
//! The CLI's output carries a path (`projectsDirectory`), so it is read for
//! that one boolean and dropped: never logged, never put in a record.

use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{Class, Factory, Failure, Health};

/// How often each login's sign-in is checked, in seconds; `0` turns the
/// check off.
pub(crate) const CHECK_SECS_ENV: &str = "BUZZ_FACTORY_SIGNIN_CHECK_SECS";

/// The default for [`CHECK_SECS_ENV`]: every 15 minutes.
pub(crate) const CHECK_SECS_DEFAULT: u64 = 900;

/// A check that has not answered in this long is no answer at all.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// Enough of `claude auth status --json` to read `loggedIn` from.
const MAX_STATUS_BYTES: u64 = 8 * 1024;

/// What a sign-in check found for one login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignIn {
    /// The CLI says this login is signed in.
    Valid,
    /// The CLI says it is not: no credential at all, or an OAuth session it
    /// can no longer refresh — which is exactly what Reset Worker hit.
    SignedOut,
    /// No answer, so nothing is concluded and nothing changes.
    Unknown,
}

/// What a login that failed its check puts on the record and the card. The
/// class is the one a real 401 gets, so the failure takes the same path:
/// mark the login, move the work, raise the Critical, text the owner.
fn check_failure() -> Failure {
    Failure {
        class: Class::SignIn,
        slug: "sign_in",
        text: "not signed in (its own auth check)".to_string(),
        resets_at: None,
    }
}

impl Factory {
    /// Every login the check asks about: its index, and the environment to
    /// ask in. Each pool entry's own launch environment, so the check sees
    /// exactly the credential the adapter would — or the agent's own
    /// environment, unchanged, when there is no pool.
    ///
    /// Empty for an adapter that is not Claude: there is no `claude auth
    /// status` to ask about a Codex sign-in.
    pub(crate) fn signin_check_targets(&self) -> Vec<(usize, Vec<(String, String)>)> {
        if self.provider != "claude" {
            return Vec::new();
        }
        if self.accounts.is_empty() {
            return vec![(0, Vec::new())];
        }
        (0..self.accounts.len())
            .map(|i| (i, self.account_env(i)))
            .collect()
    }

    /// Record what the check found for login `index`.
    ///
    /// Returns whether anything changed, so the caller knows to post the
    /// STATE record and send the owner's text.
    pub(crate) fn signin_checked(
        &mut self,
        index: usize,
        found: SignIn,
        now: DateTime<Utc>,
    ) -> bool {
        match found {
            SignIn::Unknown => false,
            SignIn::Valid => self.signin_restored(index, now),
            SignIn::SignedOut => self.signin_lost(index, now),
        }
    }

    /// The check says login `index` is not signed in, and no request was in
    /// flight to find that out the hard way.
    ///
    /// When the work is on that login, this takes the same path a real 401
    /// takes (M2 item 2): the login is marked signed out, the work moves to
    /// the next healthy login or parks, the Critical card is raised, the
    /// STATE record falls due and the owner is texted.
    ///
    /// When it is not — a spare login whose OAuth session quietly expired,
    /// which is the normal fate of a fallback nobody uses and the state of
    /// any fallback folder that was never signed in — nothing failed,
    /// nothing moved and nothing stopped, so none of that is true and none
    /// of it happens. See [`spare_signin_lost`](Factory::spare_signin_lost).
    ///
    /// A login already known to be out changes nothing either way: the check
    /// runs every 15 minutes and must not card or text on every one of them.
    fn signin_lost(&mut self, index: usize, now: DateTime<Utc>) -> bool {
        let known = match self.accounts.get(index) {
            Some(account) => account.signed_out,
            // No pool: the single login's state is the harness's own.
            None => self.health == Health::Down,
        };
        if known {
            return false;
        }
        if !self.accounts.is_empty() && !self.work_runs_on(index, now) {
            return self.spare_signin_lost(index);
        }
        self.record_limit_on(index, &check_failure(), now);
        true
    }

    /// A login no agent is using, and no request is going to, lost its
    /// sign-in.
    ///
    /// It is marked out, so the pool stops choosing it and a later check or
    /// a real answer is what clears it — and that is the whole of it. No
    /// incident opens: no Degraded, no Critical card claiming the agent
    /// "stopped on login fallback" when it did not stop, no STATE record,
    /// and no all-clear owed afterwards for an outage that never happened.
    /// The owner still hears once, because a pool quietly down to its last
    /// working login is how the next outage becomes a surprise.
    fn spare_signin_lost(&mut self, index: usize) -> bool {
        self.accounts[index].signed_out = true;
        let line = format!(
            "{}'s backup sign-in needs signing in again; the agent is unaffected.",
            self.who()
        );
        tracing::warn!("{line}");
        self.alert_aside(&line);
        true
    }

    /// A check proved login `index` works again, so it is used again — the
    /// way a reset returns the work to the primary.
    ///
    /// When that was the whole outage — nothing parked, nothing else wrong,
    /// and the first login healthy — it is also the end of it. The same
    /// check that proved the sign-in gone proves it back, so the dot, the
    /// desktop panel and the owner's all-clear do not wait on a request that
    /// may not arrive for hours. A parked batch is left to its own probe
    /// instead: a real answer is better proof than a check, and the probe is
    /// already due.
    fn signin_restored(&mut self, index: usize, now: DateTime<Utc>) -> bool {
        let cleared = match self.accounts.get_mut(index) {
            Some(account) => std::mem::take(&mut account.signed_out),
            None => false,
        };
        let over = matches!(self.health, Health::Down | Health::Degraded)
            && self.parks.is_empty()
            && self
                .failure
                .as_ref()
                .is_some_and(|f| f.class == Class::SignIn)
            && (self.accounts.is_empty() || self.preferred(now) == Some(0));
        if over {
            self.resumes = None;
            self.set_health(Health::Operational, now);
        }
        cleared || over
    }
}

/// Ask the Claude CLI whether `env` is signed in, spending no usage.
///
/// `env` is a login's launch environment: a folder entry's
/// `CLAUDE_CONFIG_DIR` and blanked token, or nothing at all for a
/// `label=env` entry, which leaves the agent's own token in place.
pub(crate) async fn probe(env: &[(String, String)]) -> SignIn {
    let cli = crate::acp::claude_cli(env);
    let mut cmd = tokio::process::Command::new(&cli);
    cmd.arg("auth")
        .arg("status")
        .arg("--json")
        .envs(env.iter().cloned());
    // Any exit status: the real CLI prints `{"loggedIn": false, …}` and
    // exits 1 when the login is signed out, which is the answer this whole
    // check exists to hear. A spawn that fails, a timeout and output that
    // will not parse are all still Unknown.
    match crate::acp::probe_stdout(&mut cmd, MAX_STATUS_BYTES, CHECK_TIMEOUT).await {
        Some(out) => read_status(&out),
        None => {
            tracing::debug!("the sign-in check got no answer from {cli:?} — concluding nothing");
            SignIn::Unknown
        }
    }
}

/// Read `claude auth status --json`.
///
/// Only `loggedIn` is read, and only when it really is a boolean. The rest
/// of the output carries a folder path and the account behind the login, so
/// it is dropped here and never logged, recorded or shown.
fn read_status(out: &str) -> SignIn {
    let logged_in = serde_json::from_str::<serde_json::Value>(out)
        .ok()
        .as_ref()
        .and_then(|status| status.get("loggedIn"))
        .and_then(serde_json::Value::as_bool);
    match logged_in {
        Some(true) => SignIn::Valid,
        Some(false) => SignIn::SignedOut,
        None => SignIn::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpError;
    use crate::factory::{classify, Plan, Reporting, PROBE_EVERY};
    use crate::scope::SessionScope;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    /// 18 Oct 2026, 20:32 UTC — 3:32 PM CT, when Reset Worker's sign-in went.
    fn oct18() -> DateTime<Utc> {
        at(2026, 10, 18, 20, 32)
    }

    fn scope() -> SessionScope {
        SessionScope::Conversation {
            channel_id: uuid::Uuid::from_u128(7),
        }
    }

    const POOL: &str = r"token=env;fallback=C:\Users\PC owner\.buzz-agents\factory\claude";

    /// The Critical sign-in card's path in a Bench inbox.
    fn card_path(bench: &std::path::Path) -> std::path::PathBuf {
        bench
            .join("inbox")
            .join(crate::factory::CARD_WRITER)
            .join("ade9d0e2-sign-in.json")
    }

    fn pool() -> Factory {
        let mut f = Factory::new(Some(POOL), "claude-agent-acp");
        f.reporting = Reporting {
            agent: "Reset Worker".to_string(),
            short: "ade9d0e2".to_string(),
            ..Reporting::default()
        };
        f
    }

    /// Only `loggedIn: false` is a verdict. Everything else — a missing
    /// field, a string where the boolean should be, truncated or empty
    /// output, an error page — says nothing, because a check that cannot
    /// tell must not take a working agent down.
    #[test]
    fn only_a_definite_answer_is_a_verdict() {
        assert_eq!(
            read_status(r#"{"loggedIn": true, "authMethod": "claudeai"}"#),
            SignIn::Valid
        );
        assert_eq!(
            read_status(r#"{"loggedIn": false, "authMethod": "none"}"#),
            SignIn::SignedOut
        );
        for unknown in [
            "",
            "{}",
            "not json at all",
            r#"{"loggedIn": "false"}"#,
            r#"{"loggedIn": null}"#,
            r#"{"loggedIn""#,
            r#"{"authMethod": "none"}"#,
        ] {
            assert_eq!(read_status(unknown), SignIn::Unknown, "{unknown:?}");
        }
    }

    /// A background check failure is the whole point of M2 item 3: nobody
    /// asked the agent anything, and the owner still hears about it. It
    /// takes the same path a real 401 takes — the login is marked signed
    /// out, the work moves on, the Critical is raised and the text is
    /// queued — and the 15-minute checks after it stay quiet.
    #[test]
    fn a_background_check_failure_raises_the_critical_once() {
        let bench = tempfile::tempdir().unwrap();
        let mut f = pool();
        f.reporting.bench = Some(bench.path().to_path_buf());
        f.launch_env(0, oct18());
        assert_eq!(f.label(0), "token");

        assert!(f.signin_checked(0, SignIn::SignedOut, oct18()));
        assert!(f.accounts()[0].signed_out, "the login is marked out");
        assert!(!f.accounts()[1].signed_out, "the other login is untouched");
        // Another login can take the work, so the agent keeps moving.
        assert_eq!(f.health, Health::Degraded);
        assert_eq!(f.launch_env(0, oct18()), f.account_env(1));

        let record = f.take_report(oct18()).expect("a STATE record");
        assert!(record.contains("state=DEGRADED class=sign_in"), "{record}");
        assert!(
            record.ends_with(
                "Reset Worker is continuing on fallback while token is signed out. Nothing is \
                 lost."
            ),
            "{record}"
        );
        let card = std::fs::read_to_string(
            bench
                .path()
                .join("inbox")
                .join(crate::factory::CARD_WRITER)
                .join("ade9d0e2-sign-in.json"),
        )
        .expect("the Critical card");
        assert!(
            card.contains("Reset Worker lost its Claude sign-in"),
            "{card}"
        );
        assert!(card.contains("now running on login fallback"), "{card}");
        assert_eq!(
            f.take_alerts(),
            [
                "Buzz: Reset Worker lost its Claude sign-in. It is now running on login fallback. \
              Claude said: \"not signed in (its own auth check)\"."
            ]
        );

        // The next four checks find the same thing: nothing changes, no
        // second card and no second text. The STATE record renews itself on
        // its own expiry, as a state that has not moved always has, and it
        // still says the same thing.
        std::fs::remove_file(
            bench
                .path()
                .join("inbox")
                .join(crate::factory::CARD_WRITER)
                .join("ade9d0e2-sign-in.json"),
        )
        .unwrap();
        for minute in [15, 30, 45, 60] {
            let later = oct18() + chrono::Duration::minutes(minute);
            assert!(!f.signin_checked(0, SignIn::SignedOut, later), "{minute}");
            assert!(f.take_alerts().is_empty(), "{minute}");
            if let Some(record) = f.take_report(later) {
                assert!(record.contains("state=DEGRADED class=sign_in"), "{record}");
            }
        }
        assert!(
            !bench
                .path()
                .join("inbox")
                .join(crate::factory::CARD_WRITER)
                .join("ade9d0e2-sign-in.json")
                .exists(),
            "one card per incident"
        );
    }

    /// A spare login's sign-in going is not an outage. The agent is working
    /// on `token`; the fallback is a folder nobody has signed in, which is
    /// the normal state of a login kept for emergencies. The check finds it
    /// and the honest answer is "your spare needs signing in", not "Reset
    /// Worker stopped on login fallback" — which is what the Critical path
    /// said, to the card and to the owner's phone, about an agent that had
    /// not stopped, had not moved and had lost nothing.
    #[test]
    fn a_check_failing_on_a_login_nobody_is_using_opens_no_incident() {
        let bench = tempfile::tempdir().unwrap();
        let mut f = pool();
        f.reporting.bench = Some(bench.path().to_path_buf());
        f.launch_env(0, oct18());
        assert_eq!(f.label(0), "token", "the agent is on the first login");

        assert!(f.signin_checked(1, SignIn::SignedOut, oct18()));
        assert!(f.accounts()[1].signed_out, "the spare is marked out");
        assert!(
            !f.accounts()[0].signed_out,
            "the working login is untouched"
        );
        // Nothing failed, so nothing is wrong with the agent.
        assert_eq!(f.health, Health::Operational);
        assert_eq!(f.presence(), "online");
        assert_eq!(f.take_report(oct18()), None, "no STATE record");
        assert!(
            !card_path(bench.path()).exists(),
            "no Critical card for a login nothing was running on"
        );
        // The work stays exactly where it was.
        assert_eq!(f.launch_env(0, oct18()), f.account_env(0));
        assert!(!f.wants_restart(0, oct18()));
        // One truthful line, to the owner, once.
        assert_eq!(
            f.take_alerts(),
            [
                "Buzz: Reset Worker's backup sign-in needs signing in again; the agent is \
              unaffected."
            ]
        );
        for minute in [15, 30, 45] {
            let later = oct18() + chrono::Duration::minutes(minute);
            assert!(!f.signin_checked(1, SignIn::SignedOut, later), "{minute}");
            assert!(f.take_alerts().is_empty(), "{minute}");
            assert_eq!(f.take_report(later), None, "{minute}");
        }

        // And no all-clear is owed for an incident that never opened: the
        // next passing check on the working login says nothing at all, and
        // the spare coming back is not an announcement either.
        let fixed = oct18() + chrono::Duration::hours(1);
        assert!(!f.signin_checked(0, SignIn::Valid, fixed));
        assert!(f.take_alerts().is_empty(), "nothing was ever wrong");
        assert!(
            f.signin_checked(1, SignIn::Valid, fixed),
            "the spare is back"
        );
        assert!(!f.accounts()[1].signed_out);
        assert!(f.take_alerts().is_empty());
        assert_eq!(f.health, Health::Operational);
    }

    /// Whatever a sign-in state's plain words are, they are never "hit an
    /// error and is retrying it": nothing retries a lost sign-in, and the
    /// owner's next move is to sign it in, not to wait.
    #[test]
    fn a_sign_in_state_never_reads_as_a_retry() {
        let mut f = pool();
        f.launch_env(0, oct18());
        // Degraded on a sign-in, with no login standing by — the first
        // login is the one that can take work, so there is nothing to say
        // the agent is "continuing on" instead. This is the combination
        // that fell through to the retry words.
        f.signin_checked(1, SignIn::SignedOut, oct18());
        f.health = Health::Degraded;
        f.failure = Some(check_failure());
        assert_eq!(f.preferred(oct18()), Some(0), "nothing is standing by");
        let words = f.plain_words(oct18());
        assert!(!words.contains("retrying"), "{words}");
        assert!(
            words.contains("signed out until someone signs it in"),
            "{words}"
        );

        // And the ordinary hand-off words, which were always right, are
        // left as they were.
        let mut f = pool();
        f.launch_env(0, oct18());
        f.signin_checked(0, SignIn::SignedOut, oct18());
        let record = f.take_report(oct18()).expect("a STATE record");
        assert!(record.contains("state=DEGRADED class=sign_in"), "{record}");
        assert!(!record.contains("retrying"), "{record}");
        assert!(record.contains("continuing on fallback"), "{record}");
    }

    /// Every login signed out parks the work, exactly as every login limited
    /// does, and the harness says it is down rather than pretending.
    #[test]
    fn every_login_signed_out_parks() {
        let mut f = pool();
        f.launch_env(0, oct18());
        assert!(f.signin_checked(0, SignIn::SignedOut, oct18()));
        assert!(f.signin_checked(1, SignIn::SignedOut, oct18()));
        assert_eq!(f.health, Health::Down);
        assert!(f.accounts().iter().all(|a| a.signed_out));

        let lost = classify(
            &AcpError::agent_error(-32000, "API Error: 401 OAuth access token has expired."),
            oct18(),
        );
        assert!(matches!(
            f.plan(0, &scope(), &lost, oct18()),
            Plan::Park {
                wait: PROBE_EVERY,
                ..
            }
        ));
        assert_eq!(f.presence(), "away");
    }

    /// A check that proves the login works clears it, and the agent goes
    /// back to it. With nothing parked that is the end of the incident, so
    /// the owner gets exactly one all-clear instead of waiting for a request
    /// to prove what the check already proved.
    #[test]
    fn a_passing_check_clears_the_login_and_closes_the_incident() {
        let mut f = pool();
        f.launch_env(0, oct18());
        f.signin_checked(0, SignIn::SignedOut, oct18());
        assert_eq!(f.take_alerts().len(), 1, "the owner was told");
        f.launch_env(0, oct18());
        assert_eq!(f.label(0), "fallback");

        let fixed = oct18() + chrono::Duration::minutes(15);
        assert!(f.signin_checked(0, SignIn::Valid, fixed));
        assert!(!f.accounts()[0].signed_out, "the check cleared it");
        assert_eq!(f.health, Health::Operational);
        assert_eq!(f.presence(), "online");
        assert!(f.wants_restart(0, fixed), "back to the first login");
        assert_eq!(f.launch_env(0, fixed), f.account_env(0));
        assert_eq!(
            f.take_alerts(),
            ["Buzz: all clear — Reset Worker is working normally again."]
        );
        // Nothing left to say: the next passing check is silent.
        assert!(!f.signin_checked(0, SignIn::Valid, fixed));
        assert_eq!(f.take_report(fixed), None);
        assert!(f.take_alerts().is_empty());
    }

    /// Without a pool there is one login, and the check still works on it:
    /// the agent goes DOWN when it fails and comes back when it passes.
    #[test]
    fn the_check_works_without_a_pool() {
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting.agent = "Reset Worker".to_string();
        assert_eq!(
            f.signin_check_targets(),
            vec![(0, Vec::new())],
            "the agent's own environment, unchanged"
        );
        assert!(f.signin_checked(0, SignIn::SignedOut, oct18()));
        assert_eq!(f.health, Health::Down);
        assert_eq!(f.take_alerts().len(), 1);
        // Still down 15 minutes later, and still only one text about it.
        let later = oct18() + chrono::Duration::minutes(15);
        assert!(!f.signin_checked(0, SignIn::SignedOut, later));
        assert!(f.take_alerts().is_empty());

        assert!(f.signin_checked(0, SignIn::Valid, later));
        assert_eq!(f.health, Health::Operational);
        assert_eq!(f.take_alerts().len(), 1, "the all-clear");

        // A Codex agent has no `claude auth status` to ask.
        assert!(Factory::new(None, "codex-acp")
            .signin_check_targets()
            .is_empty());
    }

    /// Unknown is not a verdict: whatever the harness was doing, it keeps
    /// doing it.
    #[test]
    fn an_unknown_check_changes_nothing() {
        let mut f = pool();
        f.launch_env(0, oct18());
        assert!(!f.signin_checked(0, SignIn::Unknown, oct18()));
        assert_eq!(f.health, Health::Operational);
        assert!(f.accounts().iter().all(|a| !a.signed_out));
        assert_eq!(f.take_report(oct18()), None);
        assert!(f.take_alerts().is_empty());

        // And it does not un-mark a login that is known to be out.
        f.signin_checked(0, SignIn::SignedOut, oct18());
        assert!(!f.signin_checked(0, SignIn::Unknown, oct18()));
        assert!(f.accounts()[0].signed_out);
    }

    /// The probe never spends usage and never marks a login down on its own
    /// failure: a CLI that is not there, or one that hangs, is Unknown.
    #[tokio::test]
    async fn a_probe_that_cannot_answer_is_unknown() {
        let env = vec![(
            "CLAUDE_CODE_EXECUTABLE".to_string(),
            "buzz-no-such-claude-binary".to_string(),
        )];
        assert_eq!(probe(&env).await, SignIn::Unknown);
    }
}
