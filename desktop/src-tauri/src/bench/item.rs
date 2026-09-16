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
pub(crate) const ALLOWED_HTTPS_HOSTS: &[&str] = &["github.com", "claude.ai"];

const TITLE_MAX: usize = 120;
const BODY_MAX: usize = 4000;
pub(crate) const NAME_MAX: usize = 32;
const ID_MAX: usize = 64;
const OPTION_MAX: usize = 80;
const OPTIONS_MAX: usize = 4;
const LINKS_MAX: usize = 4;
const LABEL_MAX: usize = 40;
const URL_MAX: usize = 512;
const SUMMARY_MAX: usize = 400;
const BOARD_TITLE_MAX: usize = 60;

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
    pub hash: String,
    pub updated_at: String,
    #[serde(default)]
    pub card: Option<super::Posted>,
}

/// `^[a-z0-9][a-z0-9-]{0,max-1}$`: writer, id and area. Writer and area are
/// the only untrusted strings printed outside the fence on a card.
pub(crate) fn valid_name(s: &str, max: usize) -> bool {
    let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !s.is_empty()
        && s.len() <= max
        && s.starts_with(plain)
        && s.chars().all(|c| plain(c) || c == '-')
}

fn valid_text(s: &str, max_chars: usize, field: &str) -> Result<(), String> {
    if s.chars().count() > max_chars {
        return Err(format!("{field}: longer than {max_chars} chars"));
    }
    // Cc except newline and tab, plus the zero-width and bidi format
    // characters both apps honour inside a fence: an option list reordered
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
    // The card prints the raw url outside the fence, but the parser below
    // drops tabs and newlines and encodes spaces: bound the raw bytes first.
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
    let d: Drop = serde_json::from_str(raw).map_err(|e| e.to_string())?;
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
    // number of fractional digits, and the card prints it outside the fence.
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
        hash,
        updated_at: now_iso.to_string(),
        card: None,
    })
}

/// Content hash independent of JSON key order, id and updatedAt, so
/// re-dropping identical content is a no-op.
pub(crate) fn version_hash(d: &Drop) -> String {
    fn j<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).unwrap_or_default()
    }
    let parts = [
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
    short_hash(parts.join("\u{1f}").as_bytes())
}

fn short_hash(bytes: &[u8]) -> String {
    let mut h = hex::encode(Sha256::digest(bytes));
    h.truncate(16);
    h
}

/// Wrap text in a code fence longer than any backtick run inside it. Both
/// apps render fenced code inertly: no links, mentions or markdown.
pub(crate) fn fence(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let f = "`".repeat((longest + 1).max(3));
    format!("{f}text\n{text}\n{f}")
}

pub(crate) fn summary(body: &str) -> String {
    match body.char_indices().nth(SUMMARY_MAX) {
        Some((cut, _)) => format!("{}\u{2026}", &body[..cut]),
        None => body.to_string(),
    }
}

/// Board lines sit outside any fence, where both apps linkify bare URLs and
/// `[x](url)`: keep only characters that can form neither.
pub(crate) fn plain_title(t: &str) -> String {
    let kept: String = t
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " ,.;:!?'\"()&+_-".contains(*c))
        .collect();
    kept.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(BOARD_TITLE_MAX)
        .collect()
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

/// Card text: marker line, one fence holding title/summary/options, then the
/// validated deadline and links outside the fence so the phone can open them.
pub(crate) fn render_card(item: &Item, hhmm: &str) -> String {
    let mut inner = format!("{}\n\n{}", item.title, summary(&item.body));
    if item.kind == ItemKind::Pending {
        inner.push('\n');
        for (i, o) in item.options.iter().enumerate() {
            inner.push_str(&format!("\n{}. {o}", i + 1));
        }
    }
    let mut out = format!(
        "{} \u{b7} {} \u{b7} {}\n{}",
        marker(item),
        item.area,
        item.writer,
        fence(&inner)
    );
    if let Some(d) = &item.deadline {
        out.push_str(&format!("\ndue {d}"));
    }
    for l in &item.links {
        out.push_str(&format!("\n{}: {}", l.label, l.url));
    }
    out.push_str(&format!("\nas of {hhmm}"));
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

/// Board text. Stale and superseded items are folded (not rendered).
pub(crate) fn render_board(items: &[&Item], channel: &str, hhmm: &str) -> String {
    let current = |kind: ItemKind| -> Vec<&Item> {
        items
            .iter()
            .copied()
            .filter(|i| i.kind == kind && i.validity == Validity::Current)
            .collect()
    };
    let by_area = |i: &&Item| (i.area.clone(), i.order, i.id.clone());
    let mut pending = current(ItemKind::Pending);
    pending.sort_by_key(|i| (severity_rank(i.severity), i.order, i.id.clone()));
    let mut decisions = current(ItemKind::Decision);
    decisions.sort_by_key(by_area);
    let mut status = current(ItemKind::Status);
    status.sort_by_key(by_area);

    let mut out = format!(
        "Bench \u{b7} as of {hhmm} \u{b7} {} need you",
        pending.len()
    );
    section(
        &mut out,
        "Needs you",
        pending.iter().enumerate().map(|(n, i)| {
            let target = match &i.card {
                Some(c) => format!(
                    "\u{2192} buzz://message?channel={channel}&id={}",
                    c.event_id
                ),
                None => "(no card yet)".to_string(),
            };
            format!(
                "{}. {} {} ({}) {target}",
                n + 1,
                marker(i),
                plain_title(&i.title),
                i.area
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
    out.push('\n');
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
