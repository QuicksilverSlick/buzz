//! Never silent again (factory resilience spec, M1).
//!
//! Decides what a failed turn means: which class, in whose words, and when it
//! can run again. `handle_prompt_result` uses it to park work on a limit that
//! only time or the owner can lift, instead of burning the retry budget and
//! dropping the request. Code sets these states, never an LLM: an agent that
//! is out of quota cannot report its own outage.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use crate::acp::{self, AcpError};
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

/// The texts that mean a short provider rate limit, as the provider opens
/// them (spec 3.2, class 5). Anchored like [`LIMIT_PREFIXES`]: a substring
/// test would let any error that happens to quote one of these — a tool's own
/// output, a member's message echoed back — escape the retry budget.
const RATE_PREFIXES: [&str; 3] = [
    "API Error: This request would exceed your account's rate limit",
    "API Error: 429",
    "API Error: Request would exceed your account's rate limit",
];

/// The texts that mean the sign-in is gone (spec 3.2, class 7), anchored the
/// same way.
const AUTH_PREFIXES: [&str; 3] = [
    "API Error: 401",
    "API Error: OAuth",
    "OAuth access token has expired",
];

/// The provider's own words from a folded `AgentError` message (see
/// `acp::agent_error_from_json`): the adapter's display markers taken back
/// off and the JSON-RPC prefix dropped, so the prefix lists match what the
/// provider actually opened with.
///
/// A marker is removed only when it repeats this error's own structured
/// field, so provider prose that merely ends in `(errorKind: …)` is left
/// alone — it is text, and text is never evidence of a category.
pub(crate) fn provider_text<'a>(message: &'a str, error: &AcpError) -> &'a str {
    let mut text = message;
    for (name, value) in [
        (acp::CODEX_ERROR_INFO_MARKER, codex_error_info(error)),
        (acp::ERROR_KIND_MARKER, error_kind(error)),
    ] {
        if let Some(value) = value {
            text = text
                .strip_suffix(&format!(" ({name}: {value})"))
                .unwrap_or(text);
        }
    }
    text.strip_prefix("Internal error: ").unwrap_or(text)
}

fn error_kind(error: &AcpError) -> Option<&str> {
    match error {
        AcpError::AgentError { error_kind, .. } => error_kind.as_deref(),
        _ => None,
    }
}

fn codex_error_info(error: &AcpError) -> Option<&str> {
    match error {
        AcpError::AgentError {
            codex_error_info, ..
        } => codex_error_info.as_deref(),
        _ => None,
    }
}

/// Classify a failed turn (spec section 3.3). The adapter's typed failures
/// (step 7) are not advertised yet, so this reads the adapter's own
/// `errorKind` and `codexErrorInfo` fields, then the SDK's limit prefixes,
/// then the anchored sign-in and rate-limit openings. Structured fields and
/// anchored prefixes only: free text in the middle of a message decides
/// nothing (spec 3.1).
pub(crate) fn classify(error: &AcpError, now: DateTime<Utc>) -> Failure {
    let message = match error {
        AcpError::AgentError { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let text = provider_text(&message, error);
    let kind = error_kind(error);
    let codex = codex_error_info(error);
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
        || AUTH_PREFIXES.iter().any(|p| text.starts_with(p))
    {
        Class::SignIn
    } else if kind == Some("rate_limit")
        || codex == Some("rateLimitExceeded")
        || RATE_PREFIXES.iter().any(|p| text.starts_with(p))
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
/// happened and when it resumes, and no mention, so partner channels get a
/// calm line instead of a ping.
///
/// A spend cap and a lost sign-in get the same neutral sentence as a usage
/// limit. The service being unavailable is all the sender needs; whose money
/// or whose sign-in it is belongs to the owner, in the #factory STATE record
/// and the card (D8, spec 4.4) — these lines also land in partner channels.
pub(crate) fn pause_line(failure: &Failure, resumes: DateTime<Utc>) -> String {
    let saved = "Your request is saved and nothing is lost.";
    match failure.class {
        Class::UsageLimit => format!(
            "Paused: the AI service this agent uses hit its usage limit. {saved} It resumes about {}.",
            chicago_label(resumes)
        ),
        Class::SpendCap | Class::SignIn => format!(
            "Paused: the AI service this agent uses is unavailable. {saved} I check every 30 \
             minutes."
        ),
        Class::RateLimit => format!(
            "Paused: the AI service this agent uses is limiting how fast it can work. {saved} It \
             tries again about {}.",
            chicago_label(resumes)
        ),
        Class::Other => format!("Paused. {saved}"),
    }
}

/// The thread line when the work moves to another login of the pool.
pub(crate) fn hand_off_line(label: &str) -> String {
    format!(
        "The AI service this agent uses hit its usage limit on one login, so this request is \
         continuing on {label}. Nothing is lost."
    )
}

/// Whether `s` passes the Bench's card text rules (`bench/item.rs`
/// `valid_text`): no control or format characters, no secret-shaped or
/// autolink words. The Bench refuses a card that fails them, so such text is
/// left off the card instead.
fn card_safe(s: &str) -> bool {
    let format_char = |c: char| {
        (c.is_control() && c != '\n' && c != '\t')
            || matches!(
                c,
                '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}'
            )
    };
    let lower = s.to_lowercase();
    // Assembled, as the Bench does, to keep the source word scan confined.
    let encrypted = ["ncrypt", "sec1"].concat();
    !s.chars().any(format_char)
        && !["nsec1", "nostr:", "www.", encrypted.as_str()]
            .iter()
            .any(|n| lower.contains(n))
}

/// The agent as a card names it: its display name, or its short id when the
/// name breaks the card rules.
fn card_name(name: &str, short: &str) -> String {
    if card_safe(name) { name } else { short }.to_string()
}

fn card_quote(provider: &str, text: &str) -> String {
    if card_safe(text) {
        format!("{provider} said: \"{text}\".")
    } else {
        format!("{provider}'s words can't be shown here; they are in the agent log.")
    }
}

/// The env var naming a Claude agent's ordered pool of logins: `;`-separated
/// `CLAUDE_CONFIG_DIR` folders, each signed in to its own subscription, the
/// first one the primary. An entry may be written `label=folder` to name that
/// login in records and cards; otherwise the folder's own name is used, and a
/// folder sitting directly under the home directory is `primary`, never the
/// OS account name.
pub(crate) const ACCOUNT_POOL_ENV: &str = "BUZZ_ACP_CLAUDE_ACCOUNT_POOL";

/// One login of the pool.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Account {
    /// The `CLAUDE_CONFIG_DIR` folder. Never shown: it holds a user path.
    dir: String,
    /// What records and lines call it: the owner's own `label=`, or the
    /// folder's name — never an email, a token or the OS account name. A
    /// `…/factory/claude` folder is `factory`; `~/.claude` is `primary`.
    pub label: String,
    /// The limit this login last hit, until it lifts.
    pub limited: Option<Limit>,
}

/// A limit one login of the pool hit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Limit {
    pub until: DateTime<Utc>,
    pub slug: &'static str,
    pub text: String,
}

/// What to do with a batch that failed on a limit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Plan {
    /// Run the same batch again at once, on another login of the pool: the
    /// agent restarts on it, and context is rebuilt from the relay as on
    /// every turn.
    HandOff { label: String, line: Option<String> },
    /// Keep the batch until `wait` has passed.
    Park {
        wait: Duration,
        line: Option<String>,
    },
}

/// The harness's own health (spec 2.2), set by code from what its turns
/// return. It decides the presence dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Health {
    #[default]
    Operational,
    /// Moving, with a catch: retrying, rate-limited, or on another login.
    Degraded,
    /// Stopped safely: work is parked until a reset or the owner.
    Blocked,
    /// The sign-in is lost: work is parked until the owner signs in.
    Down,
    /// The first real answers after Blocked or Down.
    Recovering,
}

/// Real answers in a row that turn Recovering into Operational (spec 2.3).
const ANSWERS_TO_RECOVER: u32 = 3;
/// A STATE record with no parked work behind it is current this long.
const RECORD_TTL: Duration = Duration::from_secs(20 * 60);
/// A parked state's record outlives its next try by this much, so the try
/// can finish and refresh it.
const RECORD_SLACK: Duration = Duration::from_secs(5 * 60);
/// The Bench inbox folder every harness drops its cards in.
pub(crate) const CARD_WRITER: &str = "factory";

/// Who a harness speaks for in STATE records and cards: names and ids only,
/// never an email, a token or a path.
#[derive(Debug, Clone, Default)]
pub(crate) struct Reporting {
    /// The agent's display name, or its short id.
    pub agent: String,
    /// The first 8 hex characters of the agent's pubkey.
    pub short: String,
    /// The #factory channel STATE records are posted to; `None` logs them
    /// only.
    pub channel: Option<uuid::Uuid>,
    /// The Bench folder cards are dropped in; `None` when the Bench never ran
    /// on this PC, so no cards are written.
    pub bench: Option<PathBuf>,
}

/// Harness-wide failure state, shared through [`Config`](crate::config::Config).
#[derive(Debug, Default)]
pub(crate) struct Factory {
    health: Health,
    /// Real answers since Recovering began.
    answers: u32,
    /// `claude`, `codex`, or the adapter's own name.
    provider: String,
    pub(crate) reporting: Reporting,
    /// The failure behind the current state, for its record.
    failure: Option<Failure>,
    /// The login the last turn ran on.
    account: String,
    /// When the current state began.
    since: Option<DateTime<Utc>>,
    /// When parked work next runs.
    resumes: Option<DateTime<Utc>>,
    /// From leaving Operational until back: `inc-<yyyymmdd>-<hhmmss>`.
    incident: Option<String>,
    /// The last record's state and expiry.
    reported: Option<(Health, DateTime<Utc>)>,
    /// Cards dropped this incident.
    carded: HashSet<String>,
    /// The account pool; empty when none is configured, which is exactly the
    /// single-login behaviour.
    accounts: Vec<Account>,
    /// Which login each agent slot was last started on.
    started_on: HashMap<usize, usize>,
    /// Consecutive rate-limit failures, for the backoff. A real answer clears it.
    rate_streak: u32,
    /// Scopes already told about this pause or hand-off, so a probe that fails
    /// again does not post a second line (once per item per incident, 7.4).
    told: HashSet<(SessionScope, bool)>,
}

impl Factory {
    /// The harness's factory state. `pool` is [`ACCOUNT_POOL_ENV`]; it only
    /// applies to the Claude adapter, whose login it switches.
    pub(crate) fn new(pool: Option<&str>, agent_command: &str) -> Self {
        let dirs: Vec<&str> = pool
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .collect();
        let identity = crate::config::normalize_agent_command_identity(agent_command);
        let claude = matches!(
            identity.as_str(),
            "claude-agent-acp" | "claude-code-acp" | "claude-code" | "claudecode"
        );
        let provider = match identity.as_str() {
            _ if claude => "claude".to_string(),
            "codex" | "codex-acp" => "codex".to_string(),
            _ => identity,
        };
        if !dirs.is_empty() && !claude {
            tracing::warn!("{ACCOUNT_POOL_ENV} is set but the agent is not Claude — ignoring it");
        }
        let mut taken: HashSet<String> = HashSet::new();
        let accounts = dirs
            .into_iter()
            .filter(|_| claude)
            .map(|entry| {
                let (mut label, dir) = parse_entry(entry);
                // Two folders can slug to one name — two home-directory
                // entries both read as `primary` — and a record still has to
                // say which login it means.
                if !taken.insert(label.clone()) {
                    for n in 2.. {
                        let candidate = format!("{label}-{n}");
                        if taken.insert(candidate.clone()) {
                            label = candidate;
                            break;
                        }
                    }
                }
                Account {
                    dir,
                    label,
                    limited: None,
                }
            })
            .collect();
        Self {
            accounts,
            provider,
            ..Self::default()
        }
    }

    /// Move to `health`. Leaving Operational opens an incident; coming back
    /// ends its cards' dedupe, so the next incident cards again.
    fn set_health(&mut self, health: Health, now: DateTime<Utc>) {
        if health == self.health {
            return;
        }
        if self.health == Health::Operational {
            self.incident = Some(now.format("inc-%Y%m%d-%H%M%S").to_string());
        }
        if health == Health::Operational {
            self.failure = None;
            self.carded.clear();
        }
        self.since = Some(now);
        self.health = health;
    }

    /// The first login not known to be limited.
    fn preferred(&self, now: DateTime<Utc>) -> Option<usize> {
        self.accounts
            .iter()
            .position(|a| a.limited.as_ref().is_none_or(|l| l.until <= now))
    }

    /// Env for starting agent `slot`: the preferred login's folder, and an
    /// empty `CLAUDE_CODE_OAUTH_TOKEN` so a PC-wide token cannot win over the
    /// folder's own login. Empty without a pool. When every login is limited
    /// it starts on the one that frees up first.
    pub(crate) fn launch_env(&mut self, slot: usize, now: DateTime<Utc>) -> Vec<(String, String)> {
        let Some(soonest) = (0..self.accounts.len())
            .min_by_key(|&i| self.accounts[i].limited.as_ref().map(|l| l.until))
        else {
            return Vec::new();
        };
        let i = self.preferred(now).unwrap_or(soonest);
        self.started_on.insert(slot, i);
        vec![
            ("CLAUDE_CONFIG_DIR".into(), self.accounts[i].dir.clone()),
            ("CLAUDE_CODE_OAUTH_TOKEN".into(), String::new()),
        ]
    }

    /// The label of the login `slot` runs on; `default` without a pool.
    pub(crate) fn label(&self, slot: usize) -> &str {
        self.started_on
            .get(&slot)
            .and_then(|&i| self.accounts.get(i))
            .map_or("default", |a| &a.label)
    }

    /// Decide what happens to `scope`'s batch after agent `slot` failed on
    /// `failure`. A usage or spend limit marks the slot's login limited and
    /// hands the batch to the next free login; with no pool, or every login
    /// limited, the batch parks until the first one frees up. Other limits
    /// never rotate. Each scope is told once per incident; short rate limits
    /// stay quiet until the third in a row.
    pub(crate) fn plan(
        &mut self,
        slot: usize,
        scope: &SessionScope,
        failure: &Failure,
        now: DateTime<Utc>,
    ) -> Plan {
        if failure.class == Class::RateLimit {
            self.rate_streak += 1;
        }
        self.account = self.label(slot).to_string();
        self.failure = Some(failure.clone());
        let mut wait = park_for(failure, now, self.rate_streak);
        if matches!(failure.class, Class::UsageLimit | Class::SpendCap) && !self.accounts.is_empty()
        {
            let on = self.started_on.get(&slot).copied().unwrap_or(0);
            self.accounts[on].limited = Some(Limit {
                until: now + chrono::Duration::from_std(wait).unwrap_or_default(),
                slug: failure.slug,
                text: failure.text.clone(),
            });
            if let Some(next) = self.preferred(now) {
                let label = self.accounts[next].label.clone();
                let line = self
                    .told
                    .insert((scope.clone(), true))
                    .then(|| hand_off_line(&label));
                self.resumes = None;
                self.set_health(Health::Degraded, now);
                return Plan::HandOff { label, line };
            }
            wait = self
                .accounts
                .iter()
                .filter_map(|a| a.limited.as_ref())
                .filter_map(|l| (l.until - now).to_std().ok())
                .min()
                .unwrap_or(wait);
        }
        let resumes = now + chrono::Duration::from_std(wait).unwrap_or_default();
        self.resumes = Some(resumes);
        // Three short rate limits in a row are Blocked (short), spec 3.2.
        let blocked = failure.class != Class::RateLimit || self.rate_streak >= 3;
        let line = (blocked && self.told.insert((scope.clone(), false)))
            .then(|| pause_line(failure, resumes));
        let health = match failure.class {
            Class::SignIn => Health::Down,
            _ if blocked => Health::Blocked,
            _ => Health::Degraded,
        };
        self.set_health(health, now);
        match failure.class {
            Class::SpendCap => self.limit_card(failure, true),
            Class::SignIn => self.limit_card(failure, false),
            _ => {}
        }
        Plan::Park { wait, line }
    }

    /// An ordinary failure (class 18): retried as today, but no longer silent.
    pub(crate) fn on_error(&mut self, slot: usize, failure: &Failure, now: DateTime<Utc>) {
        // Parked work is the bigger story: its cause and resume time stay on
        // the record.
        if matches!(self.health, Health::Blocked | Health::Down) {
            return;
        }
        self.account = self.label(slot).to_string();
        self.failure = Some(failure.clone());
        self.resumes = None;
        if matches!(self.health, Health::Operational | Health::Recovering) {
            self.set_health(Health::Degraded, now);
        }
    }

    /// A real answer from agent `slot` ends the incident and proves its login.
    /// After Blocked or Down it takes [`ANSWERS_TO_RECOVER`] in a row to be
    /// Operational again; work on a login other than the primary stays
    /// Degraded until it returns ("on standby" is sticky, spec 2.3).
    pub(crate) fn on_success(&mut self, slot: usize, now: DateTime<Utc>) {
        self.rate_streak = 0;
        self.told.clear();
        self.resumes = None;
        self.account = self.label(slot).to_string();
        if let Some(&i) = self.started_on.get(&slot) {
            self.accounts[i].limited = None;
        }
        let settled = if self.preferred(now).is_some_and(|p| p > 0) {
            Health::Degraded
        } else {
            Health::Operational
        };
        let health = match self.health {
            Health::Blocked | Health::Down => {
                self.answers = 1;
                Health::Recovering
            }
            Health::Recovering => {
                self.answers += 1;
                if self.answers >= ANSWERS_TO_RECOVER {
                    settled
                } else {
                    Health::Recovering
                }
            }
            _ => settled,
        };
        self.set_health(health, now);
    }

    /// The STATE record (spec 4.2) when the state changed, or when the last
    /// record expired while work is still stopped; `None` otherwise, and
    /// never for a harness that has been fine all along. The first line is
    /// for machines, the rest for the owner. Names, ids and the provider's
    /// own (scrubbed) words only: never member text, an email or a token.
    pub(crate) fn take_report(&mut self, now: DateTime<Utc>) -> Option<String> {
        let stopped = self.health != Health::Operational;
        let due = match self.reported {
            Some((was, expires)) => was != self.health || (stopped && now >= expires),
            None => stopped,
        };
        if !due {
            return None;
        }
        let ttl = |d: Duration| chrono::Duration::from_std(d).unwrap_or_default();
        let expires = match self.resumes {
            Some(at) => at.max(now) + ttl(RECORD_SLACK),
            None => now + ttl(RECORD_TTL),
        };
        self.reported = Some((self.health, expires));
        let iso = |t: DateTime<Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let state = match self.health {
            Health::Operational => "OPERATIONAL",
            Health::Degraded => "DEGRADED",
            Health::Blocked => "BLOCKED",
            Health::Down => "DOWN",
            Health::Recovering => "RECOVERING",
        };
        let mut record = format!(
            "STATE v1 scope=account:{}/{} agent={} state={state}",
            self.provider,
            self.account(),
            self.reporting.short
        );
        let failure = self.failure.as_ref().filter(|_| stopped);
        if let Some(f) = failure {
            record.push_str(&format!(" class={}", f.slug));
        }
        record.push_str(&format!(" since={}", iso(self.since.unwrap_or(now))));
        if let Some(at) = failure.and_then(|f| f.resets_at) {
            record.push_str(&format!(" resets={}", iso(at)));
        }
        record.push_str(&format!(" expires={}", iso(expires)));
        if let Some(incident) = &self.incident {
            record.push_str(&format!(" incident={incident}"));
        }
        if let Some(f) = failure {
            record.push_str(&format!("\n{} said: \"{}\"", self.provider_name(), f.text));
        }
        record.push_str(&format!("\nPlain words: {}", self.plain_words(now)));
        if !stopped {
            self.incident = None;
        }
        Some(record)
    }

    fn account(&self) -> &str {
        match self.account.as_str() {
            "" => "default",
            label => label,
        }
    }

    /// "Claude", "Codex", or the adapter's name with a capital.
    fn provider_name(&self) -> String {
        let mut chars = self.provider.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => "The agent".to_string(),
        }
    }

    fn who(&self) -> &str {
        match self.reporting.agent.as_str() {
            "" => "This agent",
            name => name,
        }
    }

    /// What the state means for the owner, in one or two sentences.
    fn plain_words(&self, now: DateTime<Utc>) -> String {
        let who = self.who();
        let saved = "Its requests are saved; nothing is lost.";
        let class = self.failure.as_ref().map(|f| f.class);
        let standby = self.preferred(now).filter(|&p| p > 0);
        match (self.health, class, standby) {
            (Health::Operational, ..) => format!("{who} is working normally again."),
            (Health::Recovering, ..) => format!(
                "{who} answered again; {ANSWERS_TO_RECOVER} answers in a row make it fully \
                 recovered."
            ),
            (Health::Down, ..) => format!(
                "{who} lost its sign-in. Its work is paused until it is signed in again; it \
                 checks every 30 minutes. {saved}"
            ),
            (Health::Blocked, Some(Class::SpendCap), _) => format!(
                "{who}'s work is paused until the spending cap is raised; it checks every 30 \
                 minutes. {saved}"
            ),
            (Health::Blocked, ..) => format!(
                "{who}'s work is paused until about {}. {saved}",
                self.resumes
                    .map_or_else(|| "its next check".to_string(), chicago_label)
            ),
            (Health::Degraded, _, Some(on)) => format!(
                "{who} is continuing on {} while {} is limited. Nothing is lost.",
                self.accounts[on].label, self.accounts[0].label
            ),
            (Health::Degraded, Some(Class::RateLimit), None) => {
                format!("{who} is slowed by a rate limit and tries again shortly.")
            }
            (Health::Degraded, ..) => {
                format!("{who} hit an error and is retrying it (up to 10 tries).")
            }
        }
    }

    /// The spend-cap or sign-in card (spec 3.2, rows 2 and 7): the work
    /// stopped and only the owner can lift it.
    fn limit_card(&mut self, failure: &Failure, spend_cap: bool) {
        let who = card_name(self.who(), &self.reporting.short);
        let provider = self.provider_name();
        let (what, title, then) = if spend_cap {
            (
                "spend-cap",
                format!("{provider} spend cap reached"),
                "the cap is raised",
            )
        } else {
            (
                "sign-in",
                format!("{who} lost its {provider} sign-in"),
                "it is signed in again",
            )
        };
        // The time is in the text so a later incident is a new card, not
        // the same Drop the Bench already holds.
        let since = chicago_label(self.since.unwrap_or_else(Utc::now));
        let body = format!(
            "{who} stopped on login {} at {since}. {} Its requests are parked, not dropped.",
            self.account(),
            card_quote(&provider, &failure.text)
        );
        let links = if spend_cap && self.provider == "claude" {
            serde_json::json!([
                {"label": "Claude usage settings", "url": "https://claude.ai/settings/usage"}
            ])
        } else {
            serde_json::json!([])
        };
        let wait = format!(
            "It checks every 30 minutes and resumes by itself once {then}. Tapping Seen changes \
             nothing else."
        );
        let id = format!("{}-{what}", self.reporting.short);
        self.card(&id, &title, &body, &wait, links);
    }

    /// A dropped request is never silent (spec 8.6): a Critical card naming
    /// why, with a link to the request. `event_id` is its last event's hex id.
    pub(crate) fn dropped(&mut self, channel_id: uuid::Uuid, event_id: &str, why: &str) {
        let who = card_name(self.who(), &self.reporting.short);
        let why = if card_safe(why) {
            why
        } else {
            "the reason can't be shown here; it is in the agent log"
        };
        let body = format!(
            "{who} gave up on a request: {why}. The request is not saved, so whoever sent it \
             needs to send it again. Some of the work may already be done, so check where \
             things stand first."
        );
        let id = format!(
            "{}-dropped-{}",
            self.reporting.short,
            event_id.get(..12).unwrap_or(event_id)
        );
        let link = format!("buzz://message?channel={channel_id}&id={event_id}");
        self.card(
            &id,
            &format!("{who} dropped a request"),
            &body,
            "Nothing happens by itself: the request stays dropped.",
            serde_json::json!([{"label": "Open request", "url": link}]),
        );
    }

    /// Drop a Critical, needs-you card in the Bench inbox (spec 7.1) in the
    /// Bench's drop format, once per id per incident. Written beside its name
    /// and renamed into place, so the Bench never reads half a file.
    fn card(
        &mut self,
        id: &str,
        title: &str,
        body: &str,
        if_you_wait: &str,
        links: serde_json::Value,
    ) {
        let Some(bench) = self.reporting.bench.clone() else {
            return;
        };
        if !self.carded.insert(id.to_string()) {
            return;
        }
        let drop = serde_json::json!({
            "id": id,
            "kind": "pending",
            "title": title,
            "body": body,
            "area": CARD_WRITER,
            "severity": "critical",
            "options": ["Seen"],
            "links": links,
            "plain": {"ifYouWait": if_you_wait},
        });
        let dir = bench.join("inbox").join(CARD_WRITER);
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            let part = dir.join(format!("{id}.json.part"));
            std::fs::write(&part, drop.to_string())?;
            std::fs::rename(&part, dir.join(format!("{id}.json")))
        });
        match written {
            Ok(()) => tracing::warn!(card = id, "factory card dropped in the Bench inbox"),
            Err(e) => tracing::warn!(card = id, "factory card not written: {e}"),
        }
    }

    /// The presence dot: "away" while work is parked behind a limit or a lost
    /// sign-in, "online" otherwise (spec 2.4, Agent).
    pub(crate) fn presence(&self) -> &'static str {
        match self.health {
            Health::Blocked | Health::Down => "away",
            _ => "online",
        }
    }

    /// Whether agent `slot` should restart on another login now: a login
    /// earlier in the pool (the primary, after its reset) is free again.
    pub(crate) fn wants_restart(&self, slot: usize, now: DateTime<Utc>) -> bool {
        match (self.preferred(now), self.started_on.get(&slot)) {
            (Some(p), Some(&on)) => p != on,
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn accounts(&self) -> &[Account] {
        &self.accounts
    }
}

/// The label for a login whose folder name cannot be shown: a home folder is
/// named after the OS account, and no record may carry that (spec 3.1).
pub(crate) const PRIMARY_LABEL: &str = "primary";

/// One pool entry: a folder, or `label=folder` when the owner names it.
/// Naming it is the way to keep the label off the path altogether.
fn parse_entry(entry: &str) -> (String, String) {
    match entry.split_once('=') {
        Some((label, dir)) if is_plain_label(label) && !dir.trim().is_empty() => {
            (slug(label.trim()), dir.trim().to_string())
        }
        _ => (account_label(entry), entry.to_string()),
    }
}

/// Whether `s` is a label the owner wrote rather than the head of a path: at
/// most 32 characters of `[A-Za-z0-9_-]`, so a Windows drive or a `~` entry
/// never parses as one.
fn is_plain_label(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Lower-case `[a-z0-9-]` only; `account` when nothing is left.
fn slug(name: &str) -> String {
    let label: String = name
        .trim_start_matches('.')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if label.is_empty() {
        "account".to_string()
    } else {
        label
    }
}

/// A pool folder's name for records: the last path component, or its parent
/// when that is just `claude` / `.claude`. Never a home folder's own name, so
/// a login that sits directly under the home directory is [`PRIMARY_LABEL`].
/// Use the `label=folder` form to name it yourself.
fn account_label(dir: &str) -> String {
    let parts: Vec<&str> = dir
        .trim()
        .trim_end_matches(['/', '\\'])
        .split(['/', '\\'])
        .filter(|p| !p.is_empty())
        .collect();
    let claude_leaf = parts
        .last()
        .is_some_and(|last| matches!(last.to_ascii_lowercase().as_str(), "claude" | ".claude"));
    // A leaf `claude` folder names no login, so its parent does.
    let Some(i) = parts.len().checked_sub(if claude_leaf { 2 } else { 1 }) else {
        return "account".to_string();
    };
    if is_home_component(&parts, i) {
        return PRIMARY_LABEL.to_string();
    }
    slug(parts[i])
}

/// Whether `parts[i]` is a home directory, whose name is the OS account's.
/// Read from the path itself — a component under `home` or `Users`, a bare
/// `~`, `/root` — and from this process's own home, so a home directory in an
/// unusual place is caught too.
fn is_home_component(parts: &[&str], i: usize) -> bool {
    let name = parts[i];
    if name == "~" || name.eq_ignore_ascii_case("%userprofile%") {
        return true;
    }
    if i == 0 && name.eq_ignore_ascii_case("root") {
        return true;
    }
    if i > 0 && matches!(parts[i - 1].to_ascii_lowercase().as_str(), "home" | "users") {
        return true;
    }
    home_folder_name().is_some_and(|home| home.eq_ignore_ascii_case(name))
}

/// This process's own home folder name.
fn home_folder_name() -> Option<String> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let name = PathBuf::from(home)
        .file_name()?
        .to_string_lossy()
        .trim()
        .to_string();
    (!name.is_empty()).then_some(name)
}

/// [`Factory`] behind a lock, so `Config` can carry it to every call site.
#[derive(Debug, Clone, Default)]
pub(crate) struct FactoryHandle(Arc<Mutex<Factory>>);

impl FactoryHandle {
    pub(crate) fn new(factory: Factory) -> Self {
        Self(Arc::new(Mutex::new(factory)))
    }

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
        AcpError::agent_error(-32603, message)
    }

    /// 16 Oct 2026, 16:52 UTC: the first weekly-limit error.
    fn oct16() -> DateTime<Utc> {
        at(2026, 10, 16, 16, 52)
    }

    /// Every error the logs and adapters produce, as buzz-acp now records it
    /// (spec 1.2, 3.2), and its class and slug. Built from the JSON-RPC error
    /// object the adapter actually sends, through the same
    /// `agent_error_from_json` the read loop uses, so the categories under
    /// test are the ones the adapter really set.
    #[test]
    fn classify_known_errors() {
        let cases: &[(serde_json::Value, Class, &str)] = &[
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)",
                    "data": {"errorKind": "rate_limit"}}),
                Class::UsageLimit,
                "weekly_limit",
            ),
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"}),
                Class::UsageLimit,
                "weekly_limit",
            ),
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: You've hit your session limit · resets 3:50pm"}),
                Class::UsageLimit,
                "session_limit",
            ),
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: You've hit your monthly spend limit · raise it at claude.ai/settings/usage?from=cc_cli_limit_message"}),
                Class::SpendCap,
                "spend_cap",
            ),
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: You're out of extra usage · resets Nov 1"}),
                Class::SpendCap,
                "spend_cap",
            ),
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: Your org is out of usage · add funds to continue"}),
                Class::SpendCap,
                "spend_cap",
            ),
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: API Error: This request would exceed your account's rate limit. Please try again later.",
                    "data": {"errorKind": "rate_limit"}}),
                Class::RateLimit,
                "rate_limit",
            ),
            // No category sent: the provider's own opening words decide.
            (
                serde_json::json!({"code": -32603,
                    "message": "Internal error: API Error: 429 too many requests"}),
                Class::RateLimit,
                "rate_limit",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "Request rejected", "errorKind": "rate_limit"}}),
                Class::RateLimit,
                "rate_limit",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "credit balance is too low", "errorKind": "billing_error"}}),
                Class::SpendCap,
                "spend_cap",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "Invalid API key", "errorKind": "authentication_failed"}}),
                Class::SignIn,
                "sign_in",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "org not allowed", "errorKind": "oauth_org_not_allowed"}}),
                Class::SignIn,
                "sign_in",
            ),
            (
                serde_json::json!({"code": -32000,
                    "message": "API Error: OAuth access token has expired. Re-authenticate to continue."}),
                Class::SignIn,
                "sign_in",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error: API Error: 401 unauthorized"}),
                Class::SignIn,
                "sign_in",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "You've hit your usage limit. Try again at 9:12 AM.",
                             "codexErrorInfo": "usageLimitExceeded"}}),
                Class::UsageLimit,
                "usage_limit",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "slow down", "codexErrorInfo": "rateLimitExceeded"}}),
                Class::RateLimit,
                "rate_limit",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "token revoked", "codexErrorInfo": "unauthorized"}}),
                Class::SignIn,
                "sign_in",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error"}),
                Class::Other,
                "error",
            ),
            (
                serde_json::json!({"code": -32603, "message": "Internal error",
                    "data": {"message": "stream disconnected",
                             "codexErrorInfo": {"httpConnectionFailed": {"httpStatusCode": 502}}}}),
                Class::Other,
                "error",
            ),
            (
                serde_json::json!({"code": -32601,
                    "message": "Usage credits required for 1M context — turn on usage credits"}),
                Class::Other,
                "error",
            ),
        ];
        for (error, class, slug) in cases {
            let f = classify(&crate::acp::agent_error_from_json(error), oct16());
            assert_eq!((f.class, f.slug), (*class, *slug), "{error}");
        }
        let io = AcpError::Io(std::io::Error::other("pipe broke"));
        assert_eq!(classify(&io, oct16()).class, Class::Other);
    }

    /// Only the adapter's own `errorKind` / `codexErrorInfo` fields name a
    /// class; a message that merely reads like one is text, and text is
    /// shown, not believed (spec 3.1). codex-acp folds `data.message`
    /// verbatim, so a provider or tool can put anything in it — and a
    /// mistaken spend-cap verdict parks a request for the owner and raises a
    /// Critical card about money that was never spent.
    #[test]
    fn classify_trusts_only_the_adapters_own_fields() {
        let spoofs = [
            // The defect's own case: no real errorKind, just those words.
            serde_json::json!({"code": -32603, "message": "Internal error",
                "data": {"message": "request rejected (errorKind: billing_error)"}}),
            serde_json::json!({"code": -32603, "message": "Internal error",
                "data": {"message": "no capacity (codexErrorInfo: usageLimitExceeded)"}}),
            serde_json::json!({"code": -32603,
                "message": "Internal error: tool printed \"(errorKind: authentication_failed)\""}),
            // A rate-limit sentence quoted mid-message is not a rate limit.
            serde_json::json!({"code": -32603,
                "message": "Internal error: the run log says API Error: 429 on line 7"}),
            serde_json::json!({"code": -32603,
                "message": "Internal error: a doc mentions exceed your account's rate limit"}),
        ];
        for spoof in &spoofs {
            let f = classify(&crate::acp::agent_error_from_json(spoof), oct16());
            assert_eq!(f.class, Class::Other, "{spoof}");
            assert!(!f.class.parks(), "{spoof}: must not park");
        }
        // The real field, with the same words, is believed.
        let real = serde_json::json!({"code": -32603, "message": "Internal error",
            "data": {"message": "request rejected", "errorKind": "billing_error"}});
        assert_eq!(
            classify(&crate::acp::agent_error_from_json(&real), oct16()).class,
            Class::SpendCap
        );
        // The marker is still appended for display, and taken back off for
        // the prefix match, because it repeats the field's own value.
        let limit = serde_json::json!({"code": -32603,
            "message": "Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)",
            "data": {"errorKind": "rate_limit"}});
        let error = crate::acp::agent_error_from_json(&limit);
        assert!(error.to_string().ends_with("(errorKind: rate_limit)"));
        assert_eq!(
            classify(&error, oct16()).text,
            "You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"
        );
    }

    #[test]
    fn classify_keeps_known_text_verbatim_and_scrubs_the_rest() {
        let weekly = classify(
            &crate::acp::agent_error_from_json(&serde_json::json!({"code": -32603,
                "message": "Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)",
                "data": {"errorKind": "rate_limit"}})),
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
        let scope = scope();
        let weekly = weekly();
        let Plan::Park { line, .. } = factory.plan(0, &scope, &weekly, oct16()) else {
            panic!("no pool parks");
        };
        let line = line.expect("first park posts a line");
        assert_eq!(
            line,
            "Paused: the AI service this agent uses hit its usage limit. Your request is saved and \
             nothing is lost. It resumes about Sun 18 Oct 7:02 PM CT."
        );
        assert!(!line.contains("weekly limit"), "D8: no provider wording");
        assert_eq!(park_line(&mut factory, &weekly), None);
        factory.on_success(0, oct16());
        assert!(park_line(&mut factory, &weekly).is_some());

        // Short rate limits stay quiet until the third in a row.
        let mut factory = Factory::default();
        let rate = classify(&agent_error("Internal error: API Error: 429"), oct16());
        assert_eq!(park_line(&mut factory, &rate), None);
        assert_eq!(park_line(&mut factory, &rate), None);
        assert!(park_line(&mut factory, &rate).is_some());
    }

    /// A partner's thread never learns whose money or sign-in stopped the
    /// work: a spend cap and a lost sign-in read exactly as a usage limit
    /// does (D8, spec 4.4). The owner's own words stay in the STATE record
    /// and the card.
    #[test]
    fn partner_lines_carry_no_billing_or_sign_in_wording() {
        let now = oct16();
        let spend = classify(
            &agent_error(
                "Internal error: You've hit your monthly spend limit · raise it at \
                 claude.ai/settings/usage?from=cc_cli_limit_message",
            ),
            now,
        );
        let sign_in = classify(
            &agent_error(
                "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
            ),
            now,
        );
        let neutral = "Paused: the AI service this agent uses is unavailable. Your request is \
                       saved and nothing is lost. I check every 30 minutes.";
        for failure in [&spend, &sign_in] {
            let line = pause_line(failure, now);
            assert_eq!(line, neutral, "{}", failure.slug);
            let lower = line.to_lowercase();
            for word in [
                "spend",
                "spending",
                "cap",
                "billing",
                "sign-in",
                "sign in",
                "raise it",
                "claude",
                "owner",
                "pay",
            ] {
                assert!(!lower.contains(word), "{word} in the partner line: {line}");
            }
        }
        // The owner still gets the words, in #factory and on the card.
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting = reporting("Support Agent");
        f.plan(0, &scope(), &spend, now);
        let record = f.take_report(now).expect("a STATE record");
        assert!(record.contains("class=spend_cap"), "{record}");
        assert!(record.contains("monthly spend limit"), "{record}");
    }

    /// The dot is away exactly while work waits on a limit or a lost
    /// sign-in, and online again from the first real answer.
    #[test]
    fn presence_follows_health() {
        let now = oct16();
        let seen = |f: &Factory| (f.health, f.presence());
        let mut f = Factory::default();
        assert_eq!(seen(&f), (Health::Operational, "online"));
        f.plan(0, &scope(), &weekly(), now);
        assert_eq!(seen(&f), (Health::Blocked, "away"));
        // An ordinary error elsewhere keeps the parked cause and resume time.
        let resumes = f.resumes;
        f.on_error(0, &classify(&agent_error("Internal error"), now), now);
        assert_eq!(f.failure.as_ref().map(|x| x.slug), Some("weekly_limit"));
        assert_eq!((seen(&f), f.resumes), ((Health::Blocked, "away"), resumes));
        f.on_success(0, now);
        assert_eq!(seen(&f), (Health::Recovering, "online"));
        f.on_success(0, now);
        assert_eq!(seen(&f), (Health::Recovering, "online"));
        f.on_success(0, now);
        assert_eq!(seen(&f), (Health::Operational, "online"));

        let sign_in = classify(&agent_error("API Error: 401 expired"), now);
        f.plan(0, &scope(), &sign_in, now);
        assert_eq!(seen(&f), (Health::Down, "away"));

        let mut f = Factory::default();
        let rate = classify(&agent_error("Internal error: API Error: 429"), now);
        f.plan(0, &scope(), &rate, now);
        f.plan(0, &scope(), &rate, now);
        assert_eq!(seen(&f), (Health::Degraded, "online"));
        f.plan(0, &scope(), &rate, now);
        assert_eq!(seen(&f), (Health::Blocked, "away"));

        let mut f = Factory::default();
        f.on_error(0, &classify(&agent_error("Internal error"), now), now);
        assert_eq!(seen(&f), (Health::Degraded, "online"));
        f.on_success(0, now);
        assert_eq!(seen(&f), (Health::Operational, "online"));

        // On the second login the dot stays online, and Degraded is sticky
        // until the primary is free again.
        let mut f = pool();
        f.launch_env(0, now);
        f.plan(0, &scope(), &weekly(), now);
        assert_eq!(seen(&f), (Health::Degraded, "online"));
        f.launch_env(0, now);
        f.on_success(0, now);
        assert_eq!(seen(&f), (Health::Degraded, "online"));
        f.on_success(0, at(2026, 10, 19, 0, 3));
        assert_eq!(seen(&f), (Health::Operational, "online"));
    }

    fn scope() -> SessionScope {
        SessionScope::Conversation {
            channel_id: uuid::Uuid::from_u128(7),
        }
    }

    fn weekly() -> Failure {
        classify(
            &agent_error("Internal error: You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"),
            oct16(),
        )
    }

    fn park_line(factory: &mut Factory, failure: &Failure) -> Option<String> {
        match factory.plan(0, &scope(), failure, oct16()) {
            Plan::Park { line, .. } => line,
            other => panic!("expected a park, got {other:?}"),
        }
    }

    const POOL: &str = r"C:\Users\PC owner\.buzz-agents\factory\claude; C:\Users\PC owner\.buzz-agents\account-test\claude ;";

    fn pool() -> Factory {
        Factory::new(Some(POOL), "claude-agent-acp")
    }

    fn env(dir: &str) -> Vec<(String, String)> {
        vec![
            ("CLAUDE_CONFIG_DIR".to_string(), dir.to_string()),
            ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), String::new()),
        ]
    }

    const FACTORY_DIR: &str = r"C:\Users\PC owner\.buzz-agents\factory\claude";
    const SECOND_DIR: &str = r"C:\Users\PC owner\.buzz-agents\account-test\claude";

    #[test]
    fn pool_parses_folders_and_labels_them_by_name() {
        let factory = pool();
        let labels: Vec<_> = factory
            .accounts()
            .iter()
            .map(|a| a.label.as_str())
            .collect();
        assert_eq!(labels, ["factory", "account-test"]);
        assert_eq!(account_label("/srv/Claude Two/"), "claude-two");
        // Only the Claude adapter switches logins.
        assert!(Factory::new(Some(POOL), "codex-acp").accounts().is_empty());
        let mut none = Factory::new(None, "claude-agent-acp");
        assert!(
            none.launch_env(0, oct16()).is_empty(),
            "no pool: today's env"
        );
        assert_eq!(none.label(0), "default");
    }

    /// A login that sits in the owner's home folder is `primary`: the folder
    /// is named after the OS account, and spec 3.1 keeps that out of every
    /// record, label and card. Naming it `label=folder` wins over the path.
    #[test]
    fn account_label_never_shows_a_home_folder_name() {
        let home_shaped = [
            "~/.claude",
            "/home/craig/.claude",
            "/Users/Russell/.claude",
            "C:/Users/PC owner/.claude",
            r"C:\Users\PC owner\.claude",
            r"C:\Users\PC owner",
            "/root/.claude",
            "%USERPROFILE%/.claude",
        ];
        for dir in home_shaped {
            assert_eq!(account_label(dir), PRIMARY_LABEL, "{dir}");
        }
        // A login in a folder of its own still reads by that folder's name.
        assert_eq!(account_label("/opt/agents/factory/claude"), "factory");
        assert_eq!(
            account_label(r"D:\agents\account-test\claude"),
            "account-test"
        );

        // The owner's own label wins, and is not read as a path.
        let pool = Factory::new(
            Some(r"owner=C:\Users\PC owner\.claude; /home/craig/.claude ; ~/.claude"),
            "claude-agent-acp",
        );
        let labels: Vec<_> = pool.accounts().iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["owner", "primary", "primary-2"]);
        // The folder itself is still the one the owner wrote.
        assert_eq!(pool.accounts()[0].dir, r"C:\Users\PC owner\.claude");
        // Nothing derived from a path carries an account or folder name.
        for label in &labels[1..] {
            for leak in ["pc", "craig", "users", "home"] {
                assert!(!label.contains(leak), "{label} leaks a user folder name");
            }
        }
    }

    /// A weekly limit on the primary hands the same batch to the second
    /// login, whose folder login is used (empty PC-wide token).
    #[test]
    fn limit_on_the_primary_hands_off_to_the_next_login() {
        let mut factory = pool();
        assert_eq!(factory.launch_env(0, oct16()), env(FACTORY_DIR));
        let plan = factory.plan(0, &scope(), &weekly(), oct16());
        let Plan::HandOff { label, line } = plan else {
            panic!("expected a hand-off, got {plan:?}");
        };
        assert_eq!(label, "account-test");
        let line = line.expect("the thread is told once");
        assert_eq!(
            line,
            "The AI service this agent uses hit its usage limit on one login, so this request is \
             continuing on account-test. Nothing is lost."
        );
        assert!(!line.contains("Users"), "no folder paths");
        // The restart lands on the second login, and the primary's limit is kept.
        assert_eq!(factory.launch_env(0, oct16()), env(SECOND_DIR));
        assert_eq!(factory.label(0), "account-test");
        let limit = factory.accounts()[0].limited.clone().unwrap();
        assert_eq!(limit.slug, "weekly_limit");
        assert_eq!(limit.until, at(2026, 10, 19, 0, 2));
        assert_eq!(
            limit.text,
            "You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)"
        );
    }

    #[test]
    fn every_login_limited_parks_until_the_first_frees_up() {
        let mut factory = pool();
        factory.launch_env(0, oct16());
        factory.plan(0, &scope(), &weekly(), oct16());
        factory.launch_env(0, oct16());
        let spend = classify(
            &agent_error("Internal error: You've hit your monthly spend limit · raise it"),
            oct16(),
        );
        let plan = factory.plan(0, &scope(), &spend, oct16());
        assert_eq!(
            plan,
            Plan::Park {
                wait: PROBE_EVERY,
                line: Some(pause_line(&spend, oct16() + chrono::Duration::minutes(30))),
            }
        );
        // The probe restarts on the login that frees up first: the spend cap's.
        let later = oct16() + chrono::Duration::minutes(31);
        assert_eq!(factory.launch_env(0, later), env(SECOND_DIR));
    }

    #[test]
    fn rate_limits_and_sign_in_never_rotate() {
        for message in [
            "Internal error: API Error: 429",
            "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
        ] {
            let mut factory = pool();
            factory.launch_env(0, oct16());
            let failure = classify(&agent_error(message), oct16());
            assert!(
                matches!(
                    factory.plan(0, &scope(), &failure, oct16()),
                    Plan::Park { .. }
                ),
                "{message}"
            );
            assert!(factory.accounts().iter().all(|a| a.limited.is_none()));
            assert_eq!(factory.launch_env(0, oct16()), env(FACTORY_DIR));
        }
    }

    #[test]
    fn returns_to_the_primary_after_its_reset() {
        let mut factory = pool();
        factory.launch_env(0, oct16());
        factory.plan(0, &scope(), &weekly(), oct16());
        factory.launch_env(0, oct16());
        factory.on_success(0, oct16());
        assert!(
            !factory.wants_restart(0, oct16()),
            "the primary is still limited"
        );
        let after_reset = at(2026, 10, 19, 0, 3);
        assert!(factory.wants_restart(0, after_reset));
        assert_eq!(factory.launch_env(0, after_reset), env(FACTORY_DIR));
        assert!(!factory.wants_restart(0, after_reset));
        // A real answer on a login clears its record.
        factory.on_success(0, oct16());
        assert!(factory.accounts()[0].limited.is_none());
    }

    fn reporting(agent: &str) -> Reporting {
        Reporting {
            agent: agent.to_string(),
            short: "ade9d0e2".to_string(),
            ..Reporting::default()
        }
    }

    /// The 16 Oct weekly limit as a STATE record (spec 4.2): one machine
    /// line, the provider's exact words, plain words. Re-posted only when
    /// the state changes or the last record expired while still stopped.
    #[test]
    fn state_record_on_each_change() {
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting = reporting("Reset Worker");
        assert_eq!(f.take_report(oct16()), None, "fine all along: nothing");
        f.plan(0, &scope(), &weekly(), oct16());
        assert_eq!(
            f.take_report(oct16()).as_deref(),
            Some(
                "STATE v1 scope=account:claude/default agent=ade9d0e2 state=BLOCKED \
                 class=weekly_limit since=2026-10-16T16:52:00Z resets=2026-10-19T00:00:00Z \
                 expires=2026-10-19T00:07:00Z incident=inc-20261016-165200\n\
                 Claude said: \"You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)\"\n\
                 Plain words: Reset Worker's work is paused until about Sun 18 Oct 7:02 PM CT. \
                 Its requests are saved; nothing is lost."
            )
        );
        // Another scope parked in the same incident: no new record.
        f.plan(0, &scope(), &weekly(), oct16());
        assert_eq!(f.take_report(oct16()), None);
        // The probe after the reset fails again: the expired record is renewed.
        let probe = at(2026, 10, 19, 0, 8);
        f.plan(0, &scope(), &weekly(), probe);
        assert!(f
            .take_report(probe)
            .is_some_and(|r| r.contains("state=BLOCKED")));

        let answers: Vec<Option<String>> = (0..3)
            .map(|_| {
                f.on_success(0, probe);
                f.take_report(probe)
            })
            .collect();
        let first = answers[0].as_deref().unwrap();
        assert!(
            first.contains("state=RECOVERING class=weekly_limit"),
            "{first}"
        );
        assert_eq!(answers[1], None, "still recovering");
        assert_eq!(
            answers[2].as_deref(),
            Some(
                "STATE v1 scope=account:claude/default agent=ade9d0e2 state=OPERATIONAL \
                 since=2026-10-19T00:08:00Z expires=2026-10-19T00:28:00Z \
                 incident=inc-20261016-165200\n\
                 Plain words: Reset Worker is working normally again."
            )
        );
        assert_eq!(f.take_report(probe), None);
    }

    /// An unknown error is Degraded at its first failure (spec 3.2, class
    /// 18), scrubbed; a hand-off names both logins by folder name only.
    #[test]
    fn state_record_for_retries_and_hand_off_keeps_secrets_out() {
        let mut f = Factory::new(None, "codex-acp");
        f.reporting = reporting("Orchestrator");
        let error = classify(
            &agent_error(r"Internal error: read C:\Users\PC owner\.codex\auth.json token sk-abc"),
            oct16(),
        );
        f.on_error(0, &error, oct16());
        let record = f.take_report(oct16()).unwrap();
        assert!(record.starts_with(
            "STATE v1 scope=account:codex/default agent=ade9d0e2 state=DEGRADED class=error"
        ));
        assert!(record.contains("Codex said: \"read [path] [path] token [token]\""));
        assert!(record.ends_with("Orchestrator hit an error and is retrying it (up to 10 tries)."));

        let mut f = pool();
        f.reporting = reporting("Reset Worker");
        f.launch_env(0, oct16());
        f.plan(0, &scope(), &weekly(), oct16());
        let record = f.take_report(oct16()).unwrap();
        assert!(record.starts_with(
            "STATE v1 scope=account:claude/factory agent=ade9d0e2 state=DEGRADED class=weekly_limit"
        ));
        assert!(record.ends_with(
            "Reset Worker is continuing on account-test while factory is limited. Nothing is lost."
        ));
        for secret in ["Users", "PC owner", "sk-", ".claude"] {
            assert!(!record.contains(secret), "{secret} in {record}");
        }
    }

    fn read_card(bench: &std::path::Path, id: &str) -> Option<serde_json::Value> {
        let path = bench
            .join("inbox")
            .join(CARD_WRITER)
            .join(format!("{id}.json"));
        let text = std::fs::read_to_string(path).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }

    /// A spend cap and a lost sign-in each drop one Critical card per
    /// incident, in the Bench's drop format, with the provider's exact words
    /// and nothing secret; a later incident cards again.
    #[test]
    fn limit_cards_once_per_incident() {
        let bench = tempfile::tempdir().unwrap();
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting = reporting("Support Agent");
        f.reporting.bench = Some(bench.path().to_path_buf());
        let spend = classify(
            &agent_error("Internal error: You've hit your monthly spend limit · raise it at claude.ai/settings/usage?from=cc_cli_limit_message"),
            oct16(),
        );
        f.plan(0, &scope(), &spend, oct16());
        let card = read_card(bench.path(), "ade9d0e2-spend-cap").expect("spend-cap card");
        assert_eq!(
            card,
            serde_json::json!({
                "id": "ade9d0e2-spend-cap",
                "kind": "pending",
                "title": "Claude spend cap reached",
                "body": "Support Agent stopped on login default at Fri 16 Oct 11:52 AM CT. \
                         Claude said: \"You've hit your monthly spend limit · raise it at \
                         claude.ai/settings/usage?from=cc_cli_limit_message\". Its requests \
                         are parked, not dropped.",
                "area": "factory",
                "severity": "critical",
                "options": ["Seen"],
                "links": [{"label": "Claude usage settings", "url": "https://claude.ai/settings/usage"}],
                "plain": {"ifYouWait": "It checks every 30 minutes and resumes by itself once \
                                        the cap is raised. Tapping Seen changes nothing else."},
            })
        );
        // The 30-minute probe fails again: no second card.
        std::fs::remove_file(bench.path().join("inbox/factory/ade9d0e2-spend-cap.json")).unwrap();
        f.plan(0, &scope(), &spend, oct16() + chrono::Duration::minutes(30));
        assert!(read_card(bench.path(), "ade9d0e2-spend-cap").is_none());

        let sign_in = classify(
            &agent_error(
                "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
            ),
            oct16(),
        );
        f.plan(0, &scope(), &sign_in, oct16());
        let card = read_card(bench.path(), "ade9d0e2-sign-in").expect("sign-in card");
        assert_eq!(card["title"], "Support Agent lost its Claude sign-in");
        assert_eq!(card["links"], serde_json::json!([]));

        // Back to Operational, then a new cap: a new card, with a new time.
        for _ in 0..ANSWERS_TO_RECOVER {
            f.on_success(0, oct16());
        }
        let later = at(2026, 10, 17, 13, 9);
        f.plan(0, &scope(), &spend, later);
        let card = read_card(bench.path(), "ade9d0e2-spend-cap").expect("new incident");
        assert!(card["body"]
            .as_str()
            .unwrap()
            .contains("at Sat 17 Oct 8:09 AM CT."));

        // Rate limits and usage limits wait by themselves: no card.
        let none = tempfile::tempdir().unwrap();
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting.bench = Some(none.path().to_path_buf());
        f.plan(0, &scope(), &weekly(), oct16());
        f.plan(
            0,
            &scope(),
            &classify(&agent_error("API Error: 429"), oct16()),
            oct16(),
        );
        assert!(!none.path().join("inbox").exists());
    }

    #[test]
    fn dropped_card_links_the_request_and_never_shows_unsafe_text() {
        let bench = tempfile::tempdir().unwrap();
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting = reporting("Reset Worker");
        f.reporting.bench = Some(bench.path().to_path_buf());
        let channel = uuid::Uuid::parse_str("6e4730e9-0000-4000-8000-000000000001").unwrap();
        let event = "ab".repeat(32);
        f.dropped(
            channel,
            &event,
            "Internal error (the adapter gave no detail)",
        );
        let card = read_card(bench.path(), "ade9d0e2-dropped-abababababab").expect("card");
        assert_eq!(card["title"], "Reset Worker dropped a request");
        assert_eq!(card["severity"], "critical");
        assert_eq!(
            card["links"][0]["url"],
            format!("buzz://message?channel={channel}&id={event}")
        );
        assert!(card["body"].as_str().unwrap().starts_with(
            "Reset Worker gave up on a request: Internal error (the adapter gave no detail)."
        ));

        // Text the Bench would refuse is left off, so the card still lands.
        f.dropped(channel, &"cd".repeat(32), "see www.example.com");
        let card = read_card(bench.path(), "ade9d0e2-dropped-cdcdcdcdcdcd").expect("card");
        assert!(!card["body"].as_str().unwrap().contains("www."));
        assert!(card_safe(
            "You've hit your weekly limit · resets Oct 18, 7pm"
        ));
        for bad in ["a\u{202E}b", "nostr:npub1", "key nsec1x", "tab\u{7}"] {
            assert!(!card_safe(bad), "{bad:?}");
        }

        // No Bench on this PC: nothing is written, nothing fails.
        let mut f = Factory::new(None, "claude-agent-acp");
        f.dropped(channel, &event, "boom");
    }
}
