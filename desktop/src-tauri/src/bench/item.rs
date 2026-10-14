//! Trust boundary for Bench drop files: the strict drop schema, `validate`,
//! the content hash and the card/board text renderers. Pure functions: no
//! I/O, no AppState. Everything a writer can put in a drop is bounded and
//! rendered inertly here before it can reach the relay.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Writer folder of the one-time claude.ai board import. Its decisions keep
/// the owner's own `decidedBy` values and its cards never ping him.
pub(crate) const MIGRATION_WRITER: &str = "claude-ai";
/// Keycap seeds: fully qualified, and outside the phone's burst-animation list.
pub(crate) const OPTION_EMOJI: [&str; 4] = [
    "1\u{FE0F}\u{20E3}",
    "2\u{FE0F}\u{20E3}",
    "3\u{FE0F}\u{20E3}",
    "4\u{FE0F}\u{20E3}",
];
/// An answered card stays 24 h so late pills still land, then it is retired;
/// also the attention clock on the board.
pub(crate) const ANSWER_KEEP_SECS: u64 = 86_400;
pub(crate) const ALLOWED_HTTPS_HOSTS: &[&str] = &["github.com", "claude.ai"];

const TITLE_MAX: usize = 120;
const BODY_MAX: usize = 4000;
pub(crate) const NAME_MAX: usize = 32;
const ID_MAX: usize = 64;
const OPTION_MAX: usize = 80;
const OPTIONS_MAX: usize = 4;
const LINKS_MAX: usize = 4;
const TO_MAX: usize = 8;
const LABEL_MAX: usize = 40;
const URL_MAX: usize = 512;
const SUMMARY_MAX: usize = 400;
const BOARD_TITLE_MAX: usize = 60;
/// U+FEFF, zero-width no-break space: invisible on both apps, and whitespace
/// to the phone's regexes (Dart `\s`) and the desktop's emphasis rules
/// (micromark's `/\s/`).
const ZWNBSP: char = '\u{FEFF}';

/// A drop file exactly as a writer may author it. `deny_unknown_fields` is
/// the whole "a drop never carries answer/verdict/source/version/card/
/// updatedAt" rule; the typed enums reject bad values before `validate` runs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Drop {
    pub id: Option<String>,
    pub kind: ItemKind,
    pub title: String,
    pub body: String,
    pub area: String,
    #[serde(default)]
    pub severity: Option<Severity>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub state: Option<StatusState>,
    #[serde(default)]
    pub decided_by: Option<String>,
    #[serde(default)]
    pub validity: Validity,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub needs: Needs,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub deadline: Option<String>,
    /// Step 1 scoping. `channel`, `replyTo` and `to` need a `project`, which
    /// roles.json must bind to the writer folder (`roles::bind`).
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    /// The thread root the card is posted under.
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub to: Option<To>,
    /// Training slug: the kind of decision this is.
    #[serde(default)]
    pub decision: Option<String>,
    /// 1-based option the writer recommends.
    #[serde(default)]
    pub recommend: Option<usize>,
}

/// Who a scoped card asks, before the service resolves it to answerers.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct To {
    #[serde(default)]
    pub people: Vec<String>,
    #[serde(default)]
    pub role: Option<ToRole>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ToRole {
    /// The project's Owner and Decider entries.
    Decider,
    Owner,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct Link {
    pub label: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ItemKind {
    Pending,
    Decision,
    Status,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Severity {
    Critical,
    Attention,
    Info,
    Good,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StatusState {
    Done,
    InProgress,
    Active,
    Planned,
    NotStarted,
    Blocked,
    Inactive,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Validity {
    #[default]
    Current,
    SuspectedStale,
    Superseded,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Needs {
    #[default]
    You,
    Delegable,
}

/// The answer on a pending item, derived from a keycap tap (or, on a scoped
/// card, a thread reply naming an option).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Answer {
    /// The answering event's id: the change key.
    pub reaction_id: String,
    /// 0-based index into `options`.
    pub option: usize,
    /// Bench clock when first seen.
    pub answered_at: u64,
    /// The answerer's hex; "" on answers recorded before step 1.
    #[serde(default)]
    pub by: String,
    /// A stamped answerer's roster name; None: the owner's own keys.
    #[serde(default)]
    pub by_name: Option<String>,
    #[serde(default)]
    pub role: Option<AnswerRole>,
    #[serde(default)]
    pub source: AnswerSource,
    /// Local HH:MM when recorded; "" before step 1.
    #[serde(default)]
    pub hhmm: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnswerSource {
    #[default]
    Tap,
    Reply,
}

/// The newest free-text reply on a scoped card from someone who may answer
/// it: their "Other", routed to the writer like an answer.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    pub event_id: String,
    pub by: String,
    /// None: the owner's own keys.
    pub by_name: Option<String>,
    pub role: Option<AnswerRole>,
    pub at: u64,
    pub hhmm: String,
}

/// Who may answer a scoped card, resolved by the service (`roles::stamp`)
/// and folded into its card_hash. The owner's own keys are implicit.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stamp {
    /// "": the card says "the owner".
    pub owner_name: String,
    /// Roster name of the writer's own key, for the Recommended line.
    pub recommender: Option<String>,
    pub answerers: Vec<Answerer>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Answerer {
    pub pk: String,
    pub name: String,
    pub role: AnswerRole,
    pub agent: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnswerRole {
    Owner,
    Decider,
}

/// An answer change waits this long before it is routed, so an untap and a
/// new tap (how the owner changes an answer) reach the agent as one notice.
pub(crate) const ROUTE_SETTLE_SECS: u64 = 120;

/// The last answer notice sent to the writer's route target.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Routed {
    /// 0-based option the notice announced; None: a withdrawal.
    pub option: Option<usize>,
    pub to: String,
    pub event_id: String,
    pub at: u64,
}

/// A validated drop, namespaced by writer, as kept in state.json.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    /// `<writer>/<id>`.
    pub id: String,
    pub writer: String,
    pub kind: ItemKind,
    pub title: String,
    pub body: String,
    pub area: String,
    pub severity: Option<Severity>,
    pub options: Vec<String>,
    pub order: i64,
    pub state: Option<StatusState>,
    pub decided_by: Option<String>,
    pub validity: Validity,
    pub links: Vec<Link>,
    pub needs: Needs,
    pub deadline: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    /// None: #bench. `roles::bind` fills in the project's first channel.
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub to: Option<To>,
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(default)]
    pub recommend: Option<usize>,
    pub hash: String,
    pub updated_at: String,
    #[serde(default)]
    pub card: Option<super::Posted>,
    #[serde(default)]
    pub answer: Option<Answer>,
    /// The option the live card's text shows as Recorded; None = plain.
    #[serde(default)]
    pub recorded: Option<usize>,
    /// The owner was already pinged for an earlier card of this Drop, so the
    /// next one posts without the p tag. A new Drop starts false.
    #[serde(default)]
    pub quiet_repost: bool,
    #[serde(default)]
    pub routed: Option<Routed>,
    /// Bench clock of the last answer change: a new reaction or a clear.
    #[serde(default)]
    pub changed_at: u64,
    /// Some iff scoped; set by the service, never by a drop.
    #[serde(default)]
    pub stamp: Option<Stamp>,
    #[serde(default)]
    pub reply: Option<Reply>,
    /// Scoped: the hex already p-tagged for this Drop.
    #[serde(default)]
    pub pinged: Vec<String>,
}

impl Item {
    /// Uses a step 1 field. Only these get the new card lines, named
    /// answerers and Other; every other drop keeps today's card exactly.
    pub(crate) fn scoped(&self) -> bool {
        self.project.is_some() || self.decision.is_some() || self.recommend.is_some()
    }
}

/// `^[a-z0-9][a-z0-9-]{0,max-1}$`: writer, id and area. Writer and area,
/// like a validated link url, print on the card as they are.
pub(crate) fn valid_name(s: &str, max: usize) -> bool {
    let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !s.is_empty()
        && s.len() <= max
        && s.starts_with(plain)
        && s.chars().all(|c| plain(c) || c == '-')
}

pub(crate) fn valid_text(s: &str, max_chars: usize, field: &str) -> Result<(), String> {
    if s.chars().count() > max_chars {
        return Err(format!("{field}: longer than {max_chars} chars"));
    }
    // Cc except newline and tab, plus the zero-width and bidi format
    // characters both apps honour in card text: an option list reordered
    // on screen steers the one tap the board exists for.
    let bad = |c: char| {
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
    if s.chars().any(bad) {
        return Err(format!("{field}: control or format character"));
    }
    // The egress guard only stops the encrypted-key prefix; a raw secret or a
    // nostr: URI in card text would otherwise publish. The encrypted prefix is
    // assembled at runtime so the source word scan stays confined. "www." is
    // a GFM autolink on the desktop even without a scheme or slash.
    let lower = s.to_lowercase();
    let encrypted = ["ncrypt", "sec1"].concat();
    for needle in ["nsec1", "nostr:", "www.", encrypted.as_str()] {
        if lower.contains(needle) {
            return Err(format!("{field}: contains {needle:?}"));
        }
    }
    Ok(())
}

/// Mirrors the phone's uuid check: RFC variant nibble `[89ab]`, so a nil or
/// non-RFC uuid the phone refuses never reaches the board.
fn valid_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            19 => matches!(c.to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b'),
            _ => c.is_ascii_hexdigit(),
        })
}

/// A channel id as Buzz writes it, so it compares as a plain string.
pub(crate) fn valid_channel(s: &str) -> bool {
    valid_uuid(s) && !s.bytes().any(|b| b.is_ascii_uppercase())
}

/// A pubkey or event id as Buzz writes it.
pub(crate) fn valid_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Exactly `channel=<uuid>&id=<64hex>`: no path, fragment, userinfo, port,
/// extra or duplicate params (the phone's parser rejects all of those).
fn valid_buzz_query(rest: &str) -> bool {
    let mut parts = rest.split('&');
    let (Some(channel), Some(id), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (Some(uuid), Some(hex)) = (channel.strip_prefix("channel="), id.strip_prefix("id=")) else {
        return false;
    };
    valid_uuid(uuid) && hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit())
}

fn valid_link(l: &Link) -> Result<(), String> {
    let label_ok = !l.label.is_empty()
        && l.label.chars().count() <= LABEL_MAX
        && l.label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " _.,:-".contains(c));
    if !label_ok {
        return Err(format!(
            "link label {:?}: not [A-Za-z0-9 _.,:-]{{1,40}}",
            l.label
        ));
    }
    // The card prints the raw url, but the parser below drops tabs and
    // newlines and encodes spaces: bound the raw bytes first.
    if !l.url.chars().all(|c| c.is_ascii_graphic() && c != '@') {
        return Err(format!(
            "link url {:?}: printable ascii without spaces or '@' only",
            l.url
        ));
    }
    valid_text(&l.url, URL_MAX, "link url")?;
    if let Some(rest) = l.url.strip_prefix("buzz://message?") {
        return if valid_buzz_query(rest) {
            Ok(())
        } else {
            Err(format!(
                "link url {:?}: not buzz://message?channel=<uuid>&id=<64hex>",
                l.url
            ))
        };
    }
    let u = url::Url::parse(&l.url).map_err(|e| format!("link url {:?}: {e}", l.url))?;
    if u.scheme() != "https"
        || u.port().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(format!(
            "link url {:?}: https without userinfo or port only",
            l.url
        ));
    }
    if !ALLOWED_HTTPS_HOSTS.contains(&u.host_str().unwrap_or_default()) {
        return Err(format!(
            "link url {:?}: host not in {ALLOWED_HTTPS_HOSTS:?}",
            l.url
        ));
    }
    Ok(())
}

/// The trust boundary: a drop file becomes an `Item` or a reason.
pub(crate) fn validate(
    writer: &str,
    file_stem: &str,
    raw: &str,
    now_iso: &str,
) -> Result<Item, String> {
    let mut d: Drop = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if !valid_name(writer, NAME_MAX) {
        return Err(format!("writer {writer:?}: not [a-z0-9][a-z0-9-]{{0,31}}"));
    }
    let id = d.id.clone().unwrap_or_else(|| file_stem.to_string());
    if !valid_name(&id, ID_MAX) {
        return Err(format!("id {id:?}: not [a-z0-9][a-z0-9-]{{0,63}}"));
    }
    if !valid_name(&d.area, NAME_MAX) {
        return Err(format!("area {:?}: not [a-z0-9][a-z0-9-]{{0,31}}", d.area));
    }
    valid_text(&d.title, TITLE_MAX, "title")?;
    valid_text(&d.body, BODY_MAX, "body")?;
    for o in &d.options {
        valid_text(o, OPTION_MAX, "option")?;
        // Printed through plain_text: an emoji-only option would leave the
        // owner a bare "1." beside its keycap.
        if !plain_text(o, usize::MAX)
            .bytes()
            .any(|b| b.is_ascii_alphanumeric())
        {
            return Err("option: no plain letters or digits".to_string());
        }
    }
    match d.kind {
        ItemKind::Pending => {
            if d.severity.is_none() {
                return Err("pending: severity is required".to_string());
            }
            if !(1..=OPTIONS_MAX).contains(&d.options.len()) {
                return Err("pending: 1 to 4 options".to_string());
            }
        }
        ItemKind::Status => {
            if d.state.is_none() {
                return Err("status: state is required".to_string());
            }
        }
        // The owner's own past decisions ("you"/"orchestrator") arrive only
        // through the migration import; nothing else may launder them.
        ItemKind::Decision => match d.decided_by.as_deref() {
            Some("claude") => {}
            Some("you" | "orchestrator") if writer == MIGRATION_WRITER => {}
            _ => return Err("decision: decidedBy must be \"claude\"".to_string()),
        },
    }
    if d.kind != ItemKind::Decision && d.decided_by.is_some() {
        return Err("decidedBy is for decisions only".to_string());
    }
    if d.private {
        return Err("private items ship in Phase 3".to_string());
    }
    // Kept in canonical form: chrono also takes a space separator and any
    // number of fractional digits, and the card prints it as it is.
    let deadline = d
        .deadline
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .map_err(|e| format!("deadline: {e}"))
        })
        .transpose()?;
    if d.links.len() > LINKS_MAX {
        return Err(format!("more than {LINKS_MAX} links"));
    }
    for l in &d.links {
        valid_text(&l.label, LABEL_MAX, "link label")?;
        valid_link(l)?;
    }
    validate_scope(&mut d)?;
    let hash = version_hash(&d);
    Ok(Item {
        id: format!("{writer}/{id}"),
        writer: writer.to_string(),
        kind: d.kind,
        title: d.title,
        body: d.body,
        area: d.area,
        severity: d.severity,
        options: d.options,
        order: d.order,
        state: d.state,
        decided_by: d.decided_by,
        validity: d.validity,
        links: d.links,
        needs: d.needs,
        deadline,
        project: d.project,
        channel: d.channel,
        reply_to: d.reply_to,
        to: d.to,
        decision: d.decision,
        recommend: d.recommend,
        hash,
        updated_at: now_iso.to_string(),
        card: None,
        answer: None,
        recorded: None,
        quiet_repost: false,
        routed: None,
        changed_at: 0,
        stamp: None,
        reply: None,
        pinged: Vec::new(),
    })
}

/// The step 1 fields' shape only; whether roles.json lets this writer use
/// them is `roles::bind`. `to.people` is sorted so its order never changes
/// the hash.
fn validate_scope(d: &mut Drop) -> Result<(), String> {
    let slug = |field: &str, v: &Option<String>| match v {
        Some(s) if !valid_name(s, NAME_MAX) => {
            Err(format!("{field} {s:?}: not [a-z0-9][a-z0-9-]{{0,31}}"))
        }
        _ => Ok(()),
    };
    slug("project", &d.project)?;
    slug("decision", &d.decision)?;
    if d.project.is_none() {
        for (set, field) in [
            (d.channel.is_some(), "channel"),
            (d.reply_to.is_some(), "replyTo"),
            (d.to.is_some(), "to"),
        ] {
            if set {
                return Err(format!("{field}: requires project"));
            }
        }
    }
    if let Some(c) = d.channel.as_deref().filter(|c| !valid_channel(c)) {
        return Err(format!("channel {c:?}: not a lowercase RFC uuid"));
    }
    if let Some(r) = d.reply_to.as_deref().filter(|r| !valid_hex64(r)) {
        return Err(format!("replyTo {r:?}: not 64 lowercase hex"));
    }
    if let Some(to) = &mut d.to {
        if to.people.is_empty() && to.role.is_none() {
            return Err("to: people or role is required".to_string());
        }
        if to.people.len() > TO_MAX {
            return Err(format!("to: more than {TO_MAX} people"));
        }
        if let Some(pk) = to.people.iter().find(|pk| !valid_hex64(pk)) {
            return Err(format!("to: {pk:?} is not 64 lowercase hex"));
        }
        to.people.sort();
        to.people.dedup();
    }
    if let Some(n) = d.recommend {
        if d.kind != ItemKind::Pending {
            return Err("recommend is for pending items only".to_string());
        }
        if !(1..=d.options.len()).contains(&n) {
            return Err(format!("recommend {n}: not 1 to {}", d.options.len()));
        }
    }
    Ok(())
}

/// Content hash independent of JSON key order, id and updatedAt, so
/// re-dropping identical content is a no-op.
pub(crate) fn version_hash(d: &Drop) -> String {
    fn j<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).unwrap_or_default()
    }
    let mut parts = vec![
        j(&d.kind),
        j(&d.title),
        j(&d.body),
        j(&d.area),
        j(&d.severity),
        j(&d.options),
        j(&d.order),
        j(&d.state),
        j(&d.decided_by),
        j(&d.validity),
        j(&d.links),
        j(&d.needs),
        j(&d.deadline),
    ];
    // Only when set, so a drop without them hashes as it always has: a hash
    // change reposts the card and clears its answer.
    let scope = [
        ("project", d.project.as_ref().map(j)),
        ("channel", d.channel.as_ref().map(j)),
        ("replyTo", d.reply_to.as_ref().map(j)),
        ("to", d.to.as_ref().map(j)),
        ("decision", d.decision.as_ref().map(j)),
        ("recommend", d.recommend.as_ref().map(j)),
    ];
    parts.extend(
        scope
            .into_iter()
            .filter_map(|(k, v)| Some(format!("{k}={}", v?))),
    );
    short_hash(parts.join("\u{1f}").as_bytes())
}

fn short_hash(bytes: &[u8]) -> String {
    let mut h = hex::encode(Sha256::digest(bytes));
    h.truncate(16);
    h
}

pub(crate) fn summary(body: &str) -> String {
    match body.char_indices().nth(SUMMARY_MAX) {
        Some((cut, _)) => format!("{}\u{2026}", &body[..cut]),
        None => body.to_string(),
    }
}

/// Agent text as plain paragraphs both apps show character for character:
/// gpt_markdown 1.1.6 (the phone) honours no backslash escape, so a ZWNBSP
/// breaks each trigger instead. Before `*`, `_` and `~` it makes them unable
/// to close emphasis or strikethrough and splits `**`, `~~` and `~~~`. After
/// `#`, `@`, `:`, `&`, `<`, `[`, `]` and `\` there is no heading, #channel,
/// @mention, :emoji:, autolink, entity, HTML, link, citation, LaTeX or escape.
/// Inside `--` and `||` there is no line-end rule (phone) or spoiler
/// (desktop), and before a line opening with a list, quote, table, setext,
/// radio or rule mark, no block. A ZWNBSP cannot split a code span, so a
/// backtick becomes U+02CB, which looks the same. Spaces and tabs collapse,
/// so no line is indented and none ends in a hard break.
pub(crate) fn prose(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.trim().split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.starts_with(['-', '+', '=', '>', '(', '|', '\u{2E3B}'])
            || list_mark(&line).is_some()
        {
            out.push(ZWNBSP);
        }
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '`' => out.push('\u{02CB}'),
                '*' | '_' | '~' => out.extend([ZWNBSP, c]),
                '#' | '@' | ':' | '&' | '<' | '[' | ']' | '\\' => out.extend([c, ZWNBSP]),
                '-' | '|' if chars.peek() == Some(&c) => out.extend([c, ZWNBSP]),
                c => out.push(c),
            }
        }
    }
    out
}

/// Digits of a leading `1.` or `1)` list mark: an ordered list item on the
/// desktop, and on the phone too when it is `1.`.
fn list_mark(s: &str) -> Option<usize> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let b = s.as_bytes();
    (digits > 0
        && matches!(b.get(digits), Some(b'.' | b')'))
        && matches!(b.get(digits + 1), None | Some(b' ')))
    .then_some(digits)
}

/// Board lines and a card's title, options and link labels: words only.
/// Both apps render markdown there and the phone has no backslash
/// escape, so anything outside a plainly inert set becomes a space: `_`
/// pairs into emphasis on the desktop, `;` closes an entity (`&num;`) the
/// desktop decodes into a live `#channel` or `buzz://` link, and a space
/// rather than nothing so a dropped character never joins `w[w]w.` into an
/// autolink. It also never opens a block after a card's `1. ` (both apps
/// nest blocks in a list item): it starts at a letter, digit or quote, a
/// leading `1.` or `1)` loses its mark, and trailing dashes go (the phone
/// draws any line ending in `---` as a rule). Curly quotes, en and em dashes
/// and the ellipsis fold to ASCII first, so "Don’t" never reads "Don t".
/// Nothing but a space follows a `:`: the phone draws `:name:` as the image
/// any community member published under that shortcode.
pub(crate) fn plain_text(t: &str, max: usize) -> String {
    let inert = |c: char| c.is_ascii_alphanumeric() || " ,.!?'\"()&+-".contains(c);
    let mut spaced = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{02BC}' => spaced.push('\''),
            '\u{201C}' | '\u{201D}' => spaced.push('"'),
            '\u{2013}' | '\u{2014}' => spaced.push('-'),
            '\u{2026}' => spaced.push_str("..."),
            ':' => spaced.push_str(": "),
            c if inert(c) => spaced.push(c),
            _ => spaced.push(' '),
        }
    }
    let words = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = words
        .trim_start_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '"' || c == '\''))
        .to_string();
    if let Some(digits) = list_mark(&s) {
        s.remove(digits);
    }
    let s: String = s.chars().take(max).collect();
    s.trim_end_matches(['-', ' ']).to_string()
}

pub(crate) fn plain_title(t: &str) -> String {
    plain_text(t, BOARD_TITLE_MAX)
}

pub(crate) fn severity_rank(s: Option<Severity>) -> u8 {
    match s {
        Some(Severity::Critical) => 0,
        Some(Severity::Attention) => 1,
        Some(Severity::Info) => 2,
        Some(Severity::Good) => 3,
        None => 4,
    }
}

fn marker(item: &Item) -> &'static str {
    match (item.kind, item.severity) {
        (ItemKind::Decision, _) => "\u{1F9ED}",
        (ItemKind::Pending, Some(Severity::Critical)) => "\u{1F534}",
        (ItemKind::Pending, Some(Severity::Attention)) => "\u{1F7E0}",
        (ItemKind::Pending, Some(Severity::Info)) => "\u{1F535}",
        (ItemKind::Pending, Some(Severity::Good)) => "\u{1F7E2}",
        (ItemKind::Status, _) | (ItemKind::Pending, None) => "\u{25AA}",
    }
}

fn state_name(s: StatusState) -> &'static str {
    match s {
        StatusState::Done => "done",
        StatusState::InProgress => "in-progress",
        StatusState::Active => "active",
        StatusState::Planned => "planned",
        StatusState::NotStarted => "not-started",
        StatusState::Blocked => "blocked",
        StatusState::Inactive => "inactive",
    }
}

/// Bump when render_card's text changes: every live unanswered card is then
/// reposted once, without a ping, with the new text (publish (b) retires it
/// as stale). An answered card gets the new text through its Recorded edit
/// and keeps its answer and its 24 h keep. 3: title and options moved out of
/// the fence, which the phone draws as unwrapped monospace, and the summary
/// left in it is hard-wrapped. 4: the summary leaves the fence, a grey code
/// box on the phone, for its own paragraph of prose().
pub(crate) const CARD_FORMAT: u32 = 4;

/// The version a card is posted and tagged at: `<Drop hash>.<CARD_FORMAT>`,
/// or `<Drop hash>.<stamp>.<CARD_FORMAT>` once it has a stamp, so a roster
/// change reposts an unanswered card while unstamped markers stay as they
/// were. Cards posted before CARD_FORMAT existed carry the bare Drop hash.
pub(crate) fn card_hash(item: &Item) -> String {
    match &item.stamp {
        None => format!("{}.{CARD_FORMAT}", item.hash),
        Some(st) => {
            let json = serde_json::to_string(st).unwrap_or_default();
            let stamp = short_hash(json.as_bytes());
            format!("{}.{}.{CARD_FORMAT}", item.hash, &stamp[..8])
        }
    }
}

/// Card text: marker line, the title in bold, the summary as its own
/// paragraph of prose() (agent text stays inert), the options as a numbered
/// list, then the validated deadline and links so the phone can open them,
/// and on an unanswered pending card the line that says how to answer.
/// Everything but the summary and the url goes through plain_text.
pub(crate) fn render_card(item: &Item, hhmm: &str) -> String {
    let mut out = format!(
        "{} \u{b7} {} \u{b7} {}",
        marker(item),
        item.area,
        item.writer
    );
    let title = plain_text(&item.title, usize::MAX);
    // An empty one would print `****`, a rule on both apps.
    if !title.is_empty() {
        out.push_str(&format!("\n**{title}**"));
    }
    if !item.body.trim().is_empty() {
        // Blank lines make it a paragraph of its own on both apps, so nothing
        // in it pairs with a `*` or `](` on the card's other lines: the phone
        // matches links and italics across lines, and rewrites each link
        // line's url to `[url](url)` first.
        out.push_str("\n\n");
        out.push_str(&prose(&summary(&item.body)));
        out.push('\n');
    }
    if item.kind == ItemKind::Pending {
        for (i, o) in item.options.iter().enumerate() {
            out.push_str(&format!("\n{}. {}", i + 1, plain_text(o, usize::MAX)));
        }
        // A blank line ends the list, or the desktop folds the lines below
        // into the last option.
        out.push('\n');
    }
    if let Some(d) = &item.deadline {
        out.push_str(&format!("\ndue {d}"));
    }
    for l in &item.links {
        let label = plain_text(&l.label, usize::MAX);
        out.push_str(&format!("\n{label}: {}", l.url));
    }
    if item.kind == ItemKind::Pending && item.answer.is_none() {
        out.push_str("\nTap one number below to answer.");
    }
    out.push_str(&format!("\nas of {hhmm}"));
    // Through plain_title like every board line.
    if let Some(a) = &item.answer {
        if let Some(o) = item.options.get(a.option) {
            out.push_str(&format!(
                "\nRecorded: {}. {} \u{b7} {hhmm}",
                a.option + 1,
                plain_title(o)
            ));
        }
    }
    out
}

/// p-tag the owner only for fresh needs-you items; the migration import must
/// not burst the phone.
pub(crate) fn ping_owner(item: &Item) -> bool {
    item.kind == ItemKind::Pending
        && item.needs == Needs::You
        && item.validity == Validity::Current
        && item.writer != MIGRATION_WRITER
}

/// Who to p-tag on a new card. Owner-only cards keep today's rule; a card
/// with answerers pings its humans, and its agents only when it is
/// delegable, each once per Drop.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn card_pings(item: &Item, owner_hex: &str) -> Vec<String> {
    let answerers = item.stamp.as_ref().map_or(&[][..], |s| &s.answerers);
    if answerers.is_empty() {
        return (ping_owner(item) && !item.quiet_repost)
            .then(|| owner_hex.to_string())
            .into_iter()
            .collect();
    }
    answerers
        .iter()
        .filter(|a| (!a.agent || item.needs == Needs::Delegable) && !item.pinged.contains(&a.pk))
        .map(|a| a.pk.clone())
        .collect()
}

/// Whether the route target is owed a notice: the answer has settled and is
/// not the one it was last told. A clear that was never routed owes nothing;
/// a re-tap of the told option is no change.
pub(crate) fn route_due(item: &Item, now: u64) -> bool {
    let want = item.answer.as_ref().map(|a| a.option);
    let told = item.routed.as_ref().map(|r| r.option);
    told.map_or(want.is_some(), |t| t != want)
        && now.saturating_sub(item.changed_at) >= ROUTE_SETTLE_SECS
}

/// The notice for the item's current answer; `prev` is the option the target
/// was last told (None: never, or a withdrawal). Card text goes through
/// plain_text as on the card: it is drop text, so the agent is pointed at the
/// outbox file, which carries the owner's signed reaction.
pub(crate) fn render_route(item: &Item, prev: Option<usize>, hhmm: &str) -> String {
    let id = &item.id;
    let opt = |i: usize| {
        let text = item.options.get(i).map(|o| plain_text(o, usize::MAX));
        format!("{}. {}", i + 1, text.unwrap_or_default())
    };
    let head = format!("{id}\nCard: {}", plain_text(&item.title, usize::MAX));
    let details =
        format!("Answered {hhmm} by the owner. Details: outbox/{id}.json in the Bench folder.");
    let clear = format!(
        "reply in this thread, then clear the card by re-dropping {id} as a decision or status."
    );
    match (item.answer.as_ref().map(|a| a.option), prev) {
        (Some(n), None) => format!(
            "Bench answer \u{b7} {head}\nChosen: {}\n{details}\nAct on it, {clear}",
            opt(n)
        ),
        (Some(n), Some(p)) => format!(
            "Bench answer changed \u{b7} {head}\nChosen: {}\nWas: {}\n{details}\n\
             Undo what you did for the old choice if it conflicts, {clear}",
            opt(n),
            opt(p)
        ),
        (None, Some(p)) => format!(
            "Bench answer withdrawn \u{b7} {head}\nWas: {}\nThe owner cleared the answer at \
             {hhmm}. Stop if you have not acted; if you have, say in this thread what to \
             undo. Do not clear the card.",
            opt(p)
        ),
        (None, None) => String::new(),
    }
}

/// The event a reaction is aimed at: its last `e` tag, how the relay scopes
/// a reaction.
pub(crate) fn reaction_target(e: &nostr::Event) -> Option<&str> {
    e.tags
        .iter()
        .filter_map(|t| match t.as_slice() {
            [k, v, ..] if k == "e" => Some(v.as_str()),
            _ => None,
        })
        .last()
}

/// Whether `pk` may answer this card: the owner's keys (`owner_set`: the
/// owner plus members.txt) always, a stamped answerer if it is a human or
/// the card is delegable. The second half repeats `roles::stamp`'s rule.
pub(crate) fn eligible(item: &Item, pk: &str, owner_set: &[String]) -> bool {
    owner_set.iter().any(|o| o == pk)
        || item.stamp.as_ref().is_some_and(|s| {
            s.answerers
                .iter()
                .any(|a| a.pk == pk && (!a.agent || item.needs == Needs::Delegable))
        })
}

fn refers_to(e: &nostr::Event, card_id: &str) -> bool {
    e.tags
        .iter()
        .any(|t| matches!(t.as_slice(), [k, v, ..] if k == "e" && v == card_id))
}

/// The 0-based option an event answers on the card `card_id`: a kind 7
/// aimed at it by its `reaction_target` and echoing one seeded keycap
/// byte for byte, or, on a scoped card only, a kind 9 in its thread whose
/// whole text is "1" to "n" or exactly one option's label.
pub(crate) fn answer_option(item: &Item, card_id: &str, e: &nostr::Event) -> Option<usize> {
    let n = item.options.len().min(OPTION_EMOJI.len());
    match e.kind.as_u16() {
        7 if reaction_target(e) == Some(card_id) => {
            OPTION_EMOJI[..n].iter().position(|o| *o == e.content)
        }
        9 if item.scoped() && refers_to(e, card_id) => {
            let t = e.content.trim();
            (1..=n).position(|k| t == k.to_string()).or_else(|| {
                let mut hits = item.options[..n]
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| plain_text(o, usize::MAX).eq_ignore_ascii_case(t));
                match (hits.next(), hits.next()) {
                    (Some((i, _)), None) => Some(i),
                    _ => None,
                }
            })
        }
        _ => None,
    }
}

/// The answer on `item`'s live card: the newest valid answer from the
/// owner's keys, else the newest from a stamped answerer, never the
/// writer's own seeds. The owner always wins, so a later tap by anyone
/// else never outranks theirs; newest within each so a second tap changes
/// the answer. A retraction is the event no longer coming back from the
/// relay, which hands the answer to the next.
pub(crate) fn pick_answer<'e>(
    item: &Item,
    events: &'e [nostr::Event],
    owner_set: &[String],
    writer_hex: &str,
) -> Option<(usize, &'e nostr::Event)> {
    let card = item.card.as_ref()?;
    events
        .iter()
        .filter_map(|e| {
            let i = answer_option(item, &card.event_id, e)?;
            let pk = e.pubkey.to_hex();
            let ok = pk != writer_hex && eligible(item, &pk, owner_set) && e.verify().is_ok();
            ok.then(|| (i, e, owner_set.contains(&pk)))
        })
        .max_by_key(|(_, e, owner)| (*owner, e.created_at.as_secs(), e.id.to_hex()))
        .map(|(i, e, _)| (i, e))
}

/// The newest free-text reply in a scoped card's thread from someone who
/// may answer it: a kind 9 that names no option.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn pick_reply<'e>(
    item: &Item,
    events: &'e [nostr::Event],
    owner_set: &[String],
    writer_hex: &str,
) -> Option<&'e nostr::Event> {
    let card = item.card.as_ref().filter(|_| item.scoped())?;
    events
        .iter()
        .filter(|e| {
            let pk = e.pubkey.to_hex();
            e.kind.as_u16() == 9
                && pk != writer_hex
                && refers_to(e, &card.event_id)
                && answer_option(item, &card.event_id, e).is_none()
                && eligible(item, &pk, owner_set)
                && e.verify().is_ok()
        })
        .max_by_key(|e| (e.created_at.as_secs(), e.id.to_hex()))
}

/// Valid answers on the live card from keys that may not answer it, for
/// the refusedTaps log.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn refused_taps<'e>(
    item: &Item,
    events: &'e [nostr::Event],
    owner_set: &[String],
    writer_hex: &str,
) -> Vec<&'e nostr::Event> {
    let Some(card) = &item.card else {
        return Vec::new();
    };
    events
        .iter()
        .filter(|e| {
            let pk = e.pubkey.to_hex();
            pk != writer_hex
                && answer_option(item, &card.event_id, e).is_some()
                && !eligible(item, &pk, owner_set)
                && e.verify().is_ok()
        })
        .collect()
}

/// Board text. Stale and superseded items are folded (not rendered).
pub(crate) fn render_board(items: &[&Item], channel: &str, hhmm: &str, now: u64) -> String {
    let current = |kind: ItemKind| -> Vec<&Item> {
        items
            .iter()
            .copied()
            .filter(|i| i.kind == kind && i.validity == Validity::Current)
            .collect()
    };
    let by_area = |i: &&Item| (i.area.clone(), i.order, i.id.clone());
    let (mut answered, mut pending): (Vec<&Item>, Vec<&Item>) = current(ItemKind::Pending)
        .into_iter()
        .partition(|i| i.answer.is_some());
    pending.sort_by_key(|i| (severity_rank(i.severity), i.order, i.id.clone()));
    answered.sort_by_key(|i| (i.answer.as_ref().map(|a| a.answered_at), i.id.clone()));
    let mut decisions = current(ItemKind::Decision);
    decisions.sort_by_key(by_area);
    let mut status = current(ItemKind::Status);
    status.sort_by_key(by_area);

    let mut out = format!(
        "Bench \u{b7} as of {hhmm} \u{b7} needs you: {}",
        pending.len()
    );
    if !answered.is_empty() {
        out.push_str(&format!(" \u{b7} answered: {}", answered.len()));
    }
    // Line 2, so board_hash sees it: an older board picks it up by edit.
    // Reactions toggle per keycap: untapping the answer before tapping
    // another keeps one pill lit, so an untap never revives an old choice.
    out.push_str(
        "\nTo answer: tap one number under a card. To undo or change it, tap that number again first.",
    );
    let link = |i: &Item| {
        i.card
            .as_ref()
            .map(|c| format!("buzz://message?channel={channel}&id={}", c.event_id))
    };
    // Bullets, not ordinals: a numbered line reads like an option keycap.
    section(
        &mut out,
        "Needs you",
        pending.iter().map(|i| {
            let target = link(i).map_or("(no card yet)".to_string(), |l| format!("\u{2192} {l}"));
            format!(
                "- {} {} ({}) {target}",
                marker(i),
                plain_title(&i.title),
                i.area
            )
        }),
    );
    section(
        &mut out,
        &format!("Answered, waiting for action ({})", answered.len()),
        answered.iter().map(|i| {
            let a = i.answer.as_ref().expect("answered");
            let opt = i
                .options
                .get(a.option)
                .map(|o| plain_title(o))
                .unwrap_or_default();
            let late = if now.saturating_sub(a.answered_at) > ANSWER_KEEP_SECS {
                " \u{1F7E0} 24h"
            } else {
                ""
            };
            // The kept card is where an undo or a change is tapped.
            let card = link(i).map(|l| format!(" \u{b7} {l}")).unwrap_or_default();
            format!(
                "- {} {} ({}) \u{2192} {}. {opt}{late}{card}",
                marker(i),
                plain_title(&i.title),
                i.area,
                a.option + 1
            )
        }),
    );
    section(
        &mut out,
        "Decisions",
        decisions.iter().map(|i| {
            let by = if i.writer == MIGRATION_WRITER {
                "(decided on claude.ai)"
            } else {
                "(claude)"
            };
            format!(
                "- {} {} ({}) {by}",
                marker(i),
                plain_title(&i.title),
                i.area
            )
        }),
    );
    section(
        &mut out,
        "Status",
        status.iter().map(|i| {
            let state = i.state.map_or("?", state_name);
            format!(
                "- {} {}: {} \u{2014} {state}",
                marker(i),
                i.area,
                plain_title(&i.title)
            )
        }),
    );
    out
}

fn section(out: &mut String, name: &str, lines: impl Iterator<Item = String>) {
    let mut lines = lines.peekable();
    if lines.peek().is_none() {
        return;
    }
    // A blank line ends the list above; without it the desktop renders the
    // name as a lazy continuation of the previous bullet.
    out.push_str("\n\n");
    out.push_str(name);
    for l in lines {
        out.push('\n');
        out.push_str(&l);
    }
}

/// Hash of the board minus its `as of` header line, so the clock alone never
/// forces an edit.
pub(crate) fn board_hash(text: &str) -> String {
    short_hash(
        text.split_once('\n')
            .map_or("", |(_, rest)| rest)
            .as_bytes(),
    )
}
