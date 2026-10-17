//! Never silent again (factory resilience spec, M1).
//!
//! Decides what a failed turn means: which class, in whose words, and when it
//! can run again. `handle_prompt_result` uses it to park work on a limit that
//! only time or the owner can lift, instead of burning the retry budget and
//! dropping the request. Code sets these states, never an LLM: an agent that
//! is out of quota cannot report its own outage.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use crate::acp::AcpError;
use crate::scope::SessionScope;

/// What a failed turn means for the work it carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// A 5-hour, session or weekly allowance: waits for its reset.
    UsageLimit,
    /// A spend cap, or no usage credits left: needs the owner.
    SpendCap,
    /// A short provider rate limit (429): backs off.
    RateLimit,
    /// The adapter lost its sign-in: needs the owner.
    SignIn,
    /// Anything else (spec class 18): today's bounded retries.
    Other,
}

impl Class {
    /// Whether the batch is parked rather than retried. Waiting is never the
    /// batch's fault, so a parked batch never spends the retry budget.
    pub(crate) fn parks(self) -> bool {
        self != Class::Other
    }
}

/// One classified failure (spec section 3.1).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    pub class: Class,
    /// The STATE record's `class=` slug.
    pub slug: &'static str,
    /// The provider's words: verbatim when they start with one of the SDK's
    /// own limit prefixes, scrubbed of paths, tokens, emails and phone
    /// numbers otherwise. At most [`TEXT_MAX`] characters.
    pub text: String,
    /// When the limit lifts, read from the provider's text.
    pub resets_at: Option<DateTime<Utc>>,
}

/// Provider text is cut to this many characters wherever it is shown.
pub(crate) const TEXT_MAX: usize = 300;
/// A parked batch with no known reset time is retried this often: one probe.
pub(crate) const PROBE_EVERY: Duration = Duration::from_secs(30 * 60);
/// The first try after a reset lands this long after it.
const RESET_GRACE: Duration = Duration::from_secs(2 * 60);
/// A reset further away than this is a misread, so it is probed instead.
const RESET_MAX: Duration = Duration::from_secs(8 * 24 * 3600);
const RATE_BASE: Duration = Duration::from_secs(30);
/// "Back off as today, capped at 10 minutes" (spec 3.2, class 5).
const RATE_MAX: Duration = Duration::from_secs(10 * 60);

/// `USAGE_LIMIT_ERROR_PREFIXES` from claude-agent-sdk's `sdk.d.ts`: the texts
/// that mean a usage limit was genuinely reached. They carry no secrets and
/// no member text, so they are shown word for word.
const LIMIT_PREFIXES: [&str; 12] = [
    "You've hit your",
    "You've reached your",
    "You're out of usage credits",
    "Your org is out of usage · add funds to continue",
    "Your org is out of usage · contact your admin",
    "Your seat type doesn't include usage credits",
    "Your seat type doesn't include usage",
    "Your usage allocation has been disabled by your admin",
    "Your group's usage limit is set to $0",
    "Fable 5 requires usage credits",
    "You're out of extra usage",
    "Your seat type doesn't include extra usage",
];

/// Limit wording that only the owner can lift (lower case).
const NEEDS_OWNER: [&str; 8] = [
    "spend limit",
    "extra usage",
    "usage credits",
    "out of usage",
    "add funds",
    "seat type",
    "allocation",
    "set to $0",
];

/// Split a folded `AgentError` message (see `acp::agent_error_from_json`)
/// into the provider's words and the adapter's `errorKind` and
/// `codexErrorInfo` markers.
fn split(message: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut text = message;
    let mut marker = |name: &str| {
        let open = format!(" ({name}: ");
        let i = text.rfind(&open)?;
        let value = text[i + open.len()..].strip_suffix(')')?;
        text = &text[..i];
        Some(value)
    };
    // Appended in this order, so taken off in reverse.
    let codex = marker("codexErrorInfo");
    let kind = marker("errorKind");
    let text = text.strip_prefix("Internal error: ").unwrap_or(text);
    (text, kind, codex)
}

/// Classify a failed turn (spec section 3.3). The adapter's typed failures
/// (step 7) are not advertised yet, so this reads the folded `data` markers,
/// then the SDK's limit prefixes, then the sign-in and rate-limit wording.
pub(crate) fn classify(error: &AcpError, now: DateTime<Utc>) -> Failure {
    let message = match error {
        AcpError::AgentError { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let (text, kind, codex) = split(&message);
    let known = LIMIT_PREFIXES.iter().any(|p| text.starts_with(p));
    let lower = text.to_ascii_lowercase();
    let owner_must_act = NEEDS_OWNER.iter().any(|w| lower.contains(w));
    let class = if known || codex == Some("usageLimitExceeded") {
        if owner_must_act {
            Class::SpendCap
        } else {
            Class::UsageLimit
        }
    } else if kind == Some("billing_error") {
        Class::SpendCap
    } else if matches!(
        kind,
        Some("authentication_failed" | "oauth_org_not_allowed")
    ) || codex == Some("unauthorized")
        || crate::is_auth_error(error)
    {
        Class::SignIn
    } else if kind == Some("rate_limit")
        || codex == Some("rateLimitExceeded")
        || lower.contains("exceed your account's rate limit")
        || text.contains("API Error: 429")
    {
        Class::RateLimit
    } else {
        Class::Other
    };
    let slug = match class {
        Class::UsageLimit if lower.contains("weekly") => "weekly_limit",
        Class::UsageLimit if lower.contains("session limit") || lower.contains("5-hour") => {
            "session_limit"
        }
        Class::UsageLimit => "usage_limit",
        Class::SpendCap => "spend_cap",
        Class::RateLimit => "rate_limit",
        Class::SignIn => "sign_in",
        Class::Other => "error",
    };
    let shown = if known {
        text.to_string()
    } else if class == Class::Other && matches!(text.trim(), "" | "Internal error") {
        "Internal error (the adapter gave no detail)".to_string()
    } else {
        scrub(text)
    };
    let resets_at = if class == Class::UsageLimit {
        parse_reset(text, now)
    } else {
        None
    };
    Failure {
        class,
        slug,
        text: truncate(&shown, TEXT_MAX),
        resets_at,
    }
}

/// How long a parked batch waits before its next try (spec 8.2): two minutes
/// after a known reset, backoff for a short rate limit, otherwise one probe
/// every 30 minutes.
pub(crate) fn park_for(failure: &Failure, now: DateTime<Utc>, rate_streak: u32) -> Duration {
    match (failure.class, failure.resets_at) {
        (Class::RateLimit, _) => RATE_BASE
            .saturating_mul(1 << rate_streak.saturating_sub(1).min(5))
            .min(RATE_MAX),
        (Class::UsageLimit, Some(at)) => match (at - now).to_std() {
            Ok(wait) if wait <= RESET_MAX => wait + RESET_GRACE,
            Ok(_) => PROBE_EVERY,
            // Already past: the clock or the text is off by a little.
            Err(_) => RESET_GRACE,
        },
        _ => PROBE_EVERY,
    }
}

pub(crate) fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// Remove what a raw error can leak before it is shown: user-folder paths,
/// tokens (`sk-`, `ghp_`, `gho_`, `xox…`, `nsec1`, `Bearer …`, long base64 or
/// hex), email addresses and phone numbers.
pub(crate) fn scrub(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut after_bearer = false;
    for word in text.split_whitespace() {
        let bare = word.trim_matches(|c: char| "\"'()[]<>{},;.:!?".contains(c));
        let replaced = if std::mem::take(&mut after_bearer) || is_token(bare) {
            Some("[token]")
        } else if is_path(bare) {
            Some("[path]")
        } else if is_email(bare) {
            Some("[email]")
        } else if is_phone(bare) {
            Some("[phone]")
        } else {
            None
        };
        after_bearer = bare.eq_ignore_ascii_case("bearer");
        out.push(replaced.unwrap_or(word));
    }
    out.join(" ")
}

fn is_token(w: &str) -> bool {
    const PREFIXES: [&str; 7] = ["sk-", "ghp_", "gho_", "ghs_", "github_pat_", "xox", "nsec1"];
    PREFIXES.iter().any(|p| w.starts_with(p))
        || (w.len() >= 32
            && w.bytes().any(|b| b.is_ascii_digit())
            && w.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/=_-".contains(&b)))
}

fn is_path(w: &str) -> bool {
    let b = w.as_bytes();
    w.contains('\\')
        || w.starts_with("~/")
        || ["/Users/", "/home/", "/root/", "/c/Users/"]
            .iter()
            .any(|p| w.starts_with(p))
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/')
}

fn is_email(w: &str) -> bool {
    matches!(w.split_once('@'), Some((user, host)) if !user.is_empty() && host.contains('.') && !host.contains('@'))
}

fn is_phone(w: &str) -> bool {
    w.bytes().filter(u8::is_ascii_digit).count() >= 10
        && w.bytes()
            .all(|b| b.is_ascii_digit() || b"+-().".contains(&b))
}

/// The reset in a limit text such as "resets Oct 18, 7pm (America/Chicago)",
/// "resets 7pm" or "resets 3:50pm" (spec 3.1: reading the text is the
/// fallback; the SDK's numeric `resetsAt` arrives only with step 7). Only
/// America/Chicago, the account's zone, and UTC are understood; another zone
/// returns `None`, so the batch probes instead. No date means the next such
/// time.
pub(crate) fn parse_reset(text: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let rest = &text[text.find("resets ")? + "resets ".len()..];
    let zone = rest
        .split_once('(')
        .and_then(|(_, z)| z.split_once(')'))
        .map(|(z, _)| z.trim());
    let utc = match zone {
        None | Some("America/Chicago") => false,
        Some("UTC" | "Etc/UTC") => true,
        Some(_) => return None,
    };
    let head = rest.split(['(', '·']).next()?.trim().trim_end_matches('.');
    let (date, clock) = match head.rsplit_once(", ") {
        Some((d, c)) => (Some(d), c),
        None => (None, head),
    };
    let time = parse_clock(clock)?;
    let local_now = if utc {
        now.naive_utc()
    } else {
        utc_to_chicago(now.naive_utc())
    };
    let day = match date {
        Some(d) => {
            let (month, day) = d.trim().split_once(' ')?;
            let month = MONTHS.iter().position(|m| month.starts_with(m))? as u32 + 1;
            let day: u32 = day.trim().parse().ok()?;
            let this_year = NaiveDate::from_ymd_opt(local_now.year(), month, day)?;
            // "Oct 18" seen on 2 Jan is last year's; seen on 20 Dec, next year's.
            if this_year < local_now.date() - chrono::Days::new(180) {
                NaiveDate::from_ymd_opt(local_now.year() + 1, month, day)?
            } else {
                this_year
            }
        }
        None if local_now.time() < time => local_now.date(),
        None => local_now.date().succ_opt()?,
    };
    let local = day.and_time(time);
    let utc_time = if utc { local } else { chicago_to_utc(local) };
    Some(utc_time.and_utc())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn parse_clock(s: &str) -> Option<NaiveTime> {
    let s = s.trim().to_ascii_lowercase().replace(' ', "");
    let (num, pm) = match (s.strip_suffix("pm"), s.strip_suffix("am")) {
        (Some(n), _) => (n, Some(true)),
        (_, Some(n)) => (n, Some(false)),
        _ => (s.as_str(), None),
    };
    let (h, m) = match num.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (num.parse::<u32>().ok()?, 0),
    };
    let h = match pm {
        Some(_) if !(1..=12).contains(&h) => return None,
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// America/Chicago's daylight-saving span in local wall time (US rules since
/// 2007): the second Sunday of March 02:00 to the first Sunday of November
/// 02:00. shortcut: no tz database (none is a dependency); upgrade to
/// chrono-tz if the factory ever needs a second zone.
fn chicago_dst(year: i32) -> (NaiveDateTime, NaiveDateTime) {
    let sunday = |month: u32, nth: u64| {
        let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
        let to_sunday = (7 - u64::from(first.weekday().num_days_from_sunday())) % 7;
        let day = first + chrono::Days::new(to_sunday + 7 * (nth - 1));
        day.and_hms_opt(2, 0, 0).expect("valid time")
    };
    (sunday(3, 2), sunday(11, 1))
}

fn utc_to_chicago(t: NaiveDateTime) -> NaiveDateTime {
    let (start, end) = chicago_dst(t.year());
    // 02:00 CST is 08:00 UTC; 02:00 CDT is 07:00 UTC.
    let dst = t >= start + chrono::Duration::hours(6) && t < end + chrono::Duration::hours(5);
    t - chrono::Duration::hours(if dst { 5 } else { 6 })
}

fn chicago_to_utc(local: NaiveDateTime) -> NaiveDateTime {
    let (start, end) = chicago_dst(local.year());
    local + chrono::Duration::hours(if local >= start && local < end { 5 } else { 6 })
}

/// A time as the owner reads it: "Sun 18 Oct 7:00 PM CT".
pub(crate) fn chicago_label(t: DateTime<Utc>) -> String {
    format!(
        "{} CT",
        utc_to_chicago(t.naive_utc()).format("%a %-d %b %-I:%M %p")
    )
}

/// The one plain thread line for a parked request (spec 4.3 and D8): what
/// happened and when it resumes, never the provider's billing wording, and
/// no mention, so partner channels get a calm line instead of a ping.
pub(crate) fn pause_line(failure: &Failure, resumes: DateTime<Utc>) -> String {
    let saved = "Your request is saved and nothing is lost.";
    match failure.class {
        Class::UsageLimit => format!(
            "Paused: the AI service this agent uses hit its usage limit. {saved} It resumes about {}.",
            chicago_label(resumes)
        ),
        Class::SpendCap => format!(
            "Paused: the AI service this agent uses reached its spending cap. {saved} It runs once \
             the owner raises the cap; I check every 30 minutes."
        ),
        Class::SignIn => format!(
            "Paused: this agent lost its sign-in to the AI service. {saved} It runs once the owner \
             signs it in again; I check every 30 minutes."
        ),
        Class::RateLimit => format!(
            "Paused: the AI service this agent uses is limiting how fast it can work. {saved} It \
             tries again about {}.",
            chicago_label(resumes)
        ),
        Class::Other => format!("Paused. {saved}"),
    }
}

/// Harness-wide failure state, shared through [`Config`](crate::config::Config).
#[derive(Debug, Default)]
pub(crate) struct Factory {
    /// Consecutive rate-limit failures, for the backoff. A real answer clears it.
    rate_streak: u32,
    /// Scopes already told about this pause, so a probe that fails again does
    /// not post a second line (once per item per incident, spec 7.4).
    told: HashSet<SessionScope>,
}

impl Factory {
    /// Park `scope` on `failure`: how long it waits, and the thread line to
    /// post if this scope has not been told yet. Short rate limits stay quiet
    /// until the third in a row.
    pub(crate) fn park(
        &mut self,
        scope: &SessionScope,
        failure: &Failure,
        now: DateTime<Utc>,
    ) -> (Duration, Option<String>) {
        if failure.class == Class::RateLimit {
            self.rate_streak += 1;
        }
        let wait = park_for(failure, now, self.rate_streak);
        let worth_a_line = failure.class != Class::RateLimit || self.rate_streak >= 3;
        let line = (worth_a_line && self.told.insert(scope.clone())).then(|| {
            let resumes = now + chrono::Duration::from_std(wait).unwrap_or_default();
            pause_line(failure, resumes)
        });
        (wait, line)
    }

    /// A real answer ends the incident.
    pub(crate) fn on_success(&mut self) {
        self.rate_streak = 0;
        self.told.clear();
    }
}

/// [`Factory`] behind a lock, so `Config` can carry it to every call site.
#[derive(Debug, Clone, Default)]
pub(crate) struct FactoryHandle(Arc<Mutex<Factory>>);

impl FactoryHandle {
    pub(crate) fn lock(&self) -> MutexGuard<'_, Factory> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn agent_error(message: &str) -> AcpError {
        AcpError::AgentError {
            code: -32603,
            message: message.to_string(),
        }
    }

    /// 16 Oct 2026, 16:52 UTC: the first weekly-limit error.
    fn oct16() -> DateTime<Utc> {
        at(2026, 10, 16, 16, 52)
    }

    /// Every error the logs and adapters produce, as buzz-acp now records it
    /// (spec 1.2, 3.2), and its class and slug.
    #[test]
    fn classify_known_errors() {
        let cases: &[(&str, Class, &str)] = &[
            (
                "Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago) (errorKind: rate_limit)",
                Class::UsageLimit,
                "weekly_limit",
            ),
            (
                "Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)",
                Class::UsageLimit,
                "weekly_limit",
            ),
            (
                "Internal error: You've hit your session limit · resets 3:50pm",
                Class::UsageLimit,
                "session_limit",
            ),
            (
                "Internal error: You've hit your monthly spend limit · raise it at claude.ai/settings/usage?from=cc_cli_limit_message",
                Class::SpendCap,
                "spend_cap",
            ),
            (
                "Internal error: You're out of extra usage · resets Nov 1",
                Class::SpendCap,
                "spend_cap",
            ),
            (
                "Internal error: Your org is out of usage · add funds to continue",
                Class::SpendCap,
                "spend_cap",
            ),
            (
                "Internal error: API Error: This request would exceed your account's rate limit. Please try again later. (errorKind: rate_limit)",
                Class::RateLimit,
                "rate_limit",
            ),
            (
                "Internal error: API Error: 429 too many requests",
                Class::RateLimit,
                "rate_limit",
            ),
            (
                "Internal error: Request rejected (errorKind: rate_limit)",
                Class::RateLimit,
                "rate_limit",
            ),
            (
                "Internal error: credit balance is too low (errorKind: billing_error)",
                Class::SpendCap,
                "spend_cap",
            ),
            (
                "Internal error: Invalid API key (errorKind: authentication_failed)",
                Class::SignIn,
                "sign_in",
            ),
            (
                "Internal error: org not allowed (errorKind: oauth_org_not_allowed)",
                Class::SignIn,
                "sign_in",
            ),
            (
                "API Error: OAuth access token has expired. Re-authenticate to continue.",
                Class::SignIn,
                "sign_in",
            ),
            (
                "Internal error: API Error: 401 unauthorized",
                Class::SignIn,
                "sign_in",
            ),
            (
                "Internal error: You've hit your usage limit. Try again at 9:12 AM. (codexErrorInfo: usageLimitExceeded)",
                Class::UsageLimit,
                "usage_limit",
            ),
            (
                "Internal error: slow down (codexErrorInfo: rateLimitExceeded)",
                Class::RateLimit,
                "rate_limit",
            ),
            (
                "Internal error: token revoked (codexErrorInfo: unauthorized)",
                Class::SignIn,
                "sign_in",
            ),
            ("Internal error", Class::Other, "error"),
            (
                "Internal error: stream disconnected (codexErrorInfo: {\"httpConnectionFailed\":{\"httpStatusCode\":502}})",
                Class::Other,
                "error",
            ),
            (
                "Usage credits required for 1M context — turn on usage credits",
                Class::Other,
                "error",
            ),
        ];
        for (message, class, slug) in cases {
            let f = classify(&agent_error(message), oct16());
            assert_eq!((f.class, f.slug), (*class, *slug), "{message}");
        }
        let io = AcpError::Io(std::io::Error::other("pipe broke"));
        assert_eq!(classify(&io, oct16()).class, Class::Other);
    }

    #[test]
    fn classify_keeps_known_text_verbatim_and_scrubs_the_rest() {
        let weekly = classify(
            &agent_error("Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago) (errorKind: rate_limit)"),
            oct16(),
        );
        assert_eq!(
            weekly.text,
            "You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"
        );
        let other = classify(
            &agent_error(
                r"Internal error: failed to read C:\Users\PC owner\.claude\x.json with Bearer abc.def for me@example.com",
            ),
            oct16(),
        );
        assert_eq!(
            other.text,
            "failed to read [path] [path] with Bearer [token] for [email]"
        );
        assert_eq!(
            classify(&agent_error("Internal error"), oct16()).text,
            "Internal error (the adapter gave no detail)"
        );
        let long = classify(&agent_error(&"x ".repeat(400)), oct16());
        assert_eq!(long.text.chars().count(), TEXT_MAX + 1);
    }

    #[test]
    fn scrub_table() {
        let cases = [
            ("token sk-ant-oat01-abc here", "token [token] here"),
            ("ghp_abcdef and gho_xyz", "[token] and [token]"),
            ("xoxb-1-2-3", "[token]"),
            ("key nsec1qqqq", "key [token]"),
            ("hash 0123456789abcdef0123456789abcdef!", "hash [token]"),
            ("at /Users/russell/code", "at [path]"),
            ("at /home/me/x and ~/y", "at [path] and [path]"),
            ("at C:/Users/me", "at [path]"),
            ("call 312-555-0100 or 3125550100", "call [phone] or [phone]"),
            ("mail a.b@c.io now", "mail [email] now"),
            (
                "raise it at claude.ai/settings/usage?from=cc_cli",
                "raise it at claude.ai/settings/usage?from=cc_cli",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(scrub(input), want, "{input}");
        }
    }

    #[test]
    fn parse_reset_reads_chicago_time() {
        // The 16 Oct weekly limit resets 19 Oct 00:00 UTC (spec 1.1).
        let text = "You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)";
        assert_eq!(parse_reset(text, oct16()), Some(at(2026, 10, 19, 0, 0)));
        // No zone means the account's zone; no date means the next such time.
        assert_eq!(
            parse_reset("You've hit your session limit · resets 3:50pm", oct16()),
            Some(at(2026, 10, 16, 20, 50))
        );
        assert_eq!(
            parse_reset("resets 7am", oct16()),
            Some(at(2026, 10, 17, 12, 0))
        );
        // After 1 Nov the account is on CST (UTC-6).
        assert_eq!(
            parse_reset("resets Nov 8, 7pm", oct16()),
            Some(at(2026, 11, 9, 1, 0))
        );
        // A January date seen in December is next year's.
        assert_eq!(
            parse_reset("resets Jan 2, 12am", at(2026, 12, 30, 12, 0)),
            Some(at(2027, 1, 2, 6, 0))
        );
        assert_eq!(
            parse_reset("resets Oct 18, 7pm (UTC)", oct16()),
            Some(at(2026, 10, 18, 19, 0))
        );
        assert_eq!(
            parse_reset("resets Oct 18, 7pm (Europe/Paris)", oct16()),
            None
        );
        assert_eq!(
            parse_reset("You've hit your monthly spend limit", oct16()),
            None
        );
        assert_eq!(parse_reset("resets soon", oct16()), None);
    }

    #[test]
    fn park_for_waits_for_the_reset_never_the_retry_budget() {
        let weekly = classify(
            &agent_error("Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"),
            oct16(),
        );
        // 2 days 7 h 8 min to the reset, plus 2 minutes.
        assert_eq!(
            park_for(&weekly, oct16(), 0),
            Duration::from_secs((55 * 60 + 8) * 60 + 120)
        );
        let spend = classify(
            &agent_error("Internal error: You've hit your monthly spend limit · raise it"),
            oct16(),
        );
        assert_eq!(park_for(&spend, oct16(), 0), PROBE_EVERY);
        let rate = classify(&agent_error("Internal error: API Error: 429"), oct16());
        let waits: Vec<u64> = (1..=7)
            .map(|n| park_for(&rate, oct16(), n).as_secs())
            .collect();
        assert_eq!(waits, [30, 60, 120, 240, 480, 600, 600]);
        // A reset that already passed is tried right away; a misread one is probed.
        let mut late = weekly.clone();
        late.resets_at = Some(oct16() - chrono::Duration::minutes(1));
        assert_eq!(park_for(&late, oct16(), 0), RESET_GRACE);
        late.resets_at = Some(oct16() + chrono::Duration::days(300));
        assert_eq!(park_for(&late, oct16(), 0), PROBE_EVERY);
    }

    #[test]
    fn factory_tells_each_scope_once_per_incident() {
        let mut factory = Factory::default();
        let scope = SessionScope::Conversation {
            channel_id: uuid::Uuid::new_v4(),
        };
        let weekly = classify(
            &agent_error("Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"),
            oct16(),
        );
        let (_, first) = factory.park(&scope, &weekly, oct16());
        let line = first.expect("first park posts a line");
        assert_eq!(
            line,
            "Paused: the AI service this agent uses hit its usage limit. Your request is saved and \
             nothing is lost. It resumes about Sun 18 Oct 7:02 PM CT."
        );
        assert!(!line.contains("weekly limit"), "D8: no provider wording");
        assert_eq!(factory.park(&scope, &weekly, oct16()).1, None);
        factory.on_success();
        assert!(factory.park(&scope, &weekly, oct16()).1.is_some());

        // Short rate limits stay quiet until the third in a row.
        let mut factory = Factory::default();
        let rate = classify(&agent_error("Internal error: API Error: 429"), oct16());
        assert_eq!(factory.park(&scope, &rate, oct16()).1, None);
        assert_eq!(factory.park(&scope, &rate, oct16()).1, None);
        assert!(factory.park(&scope, &rate, oct16()).1.is_some());
    }
}
