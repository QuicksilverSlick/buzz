//! What an agent is permitted to reach beyond answering its own turn.
//!
//! # Why this exists
//!
//! A managed agent already runs with the owner's filesystem access, the
//! owner's shell environment, an unrestricted Bash tool, every skill under
//! `~/.claude/skills`, and `bypass-permissions` — so it can already read every
//! transcript under `~/.claude/projects`, every credential in `~/.claude.json`,
//! and drive any other Claude Code session on the machine.
//!
//! None of that is what this type governs. What it governs is **reachability**:
//! whether a sentence typed in a channel — possibly by someone who is not the
//! owner — is allowed to become the trigger for that power. An agent whose
//! `respond_to` is `allowlist` answers a collaborator, and without a grant here
//! that collaborator's message must not reach past the agent's own turn.
//!
//! # Deny by default
//!
//! The empty set is the default, and it is the only value that keeps existing
//! `managed-agents.json` files parsing and re-serializing byte-identically.
//! That the safe value and the backward-compatible value coincide is the reason
//! this is modelled as a grant list rather than a set of opt-out booleans: a
//! record written before this type existed comes back with nothing granted,
//! which is exactly right.
//!
//! # What a grant does and does not promise
//!
//! A grant is a *policy* statement recorded by the owner. It is not, by itself,
//! an enforcement mechanism — enforcement lives at the spawn boundary and in
//! the approval path. Nothing here should be read as "the agent cannot do this
//! without the grant" until those land; today an ungranted agent is merely one
//! the owner has not authorised, which is a different and weaker claim. Saying
//! so plainly matters more than the checkbox looking reassuring.

use std::collections::BTreeSet;
use std::fmt;

/// One capability an owner can grant to an agent.
///
/// Serialized in wire shape (kebab-case) so the stored value is readable in
/// `managed-agents.json` and stable across renames of the Rust variant.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum AgentCapability {
    /// Append a labelled note to another Claude Code session's transcript.
    ///
    /// Passive: the note is seen the next time that session runs, and nothing
    /// executes on its own. The lowest-blast-radius member of this set.
    CrossSessionNote,

    /// Wake another session and run a prompt in it now (`claude -p --resume`).
    ///
    /// The receiving session cannot distinguish this from the owner typing, so
    /// a grant here effectively lends the agent the owner's voice in every
    /// other project on the machine.
    CrossSessionActivate,

    /// Read another session's transcript.
    ///
    /// Read-only, but it crosses project boundaries — transcripts routinely
    /// contain credentials, customer data, and unrelated production detail.
    CrossSessionRead,

    /// Drive the Claude Desktop application.
    DesktopControl,

    /// Drive the machine directly — screenshots, mouse, keyboard.
    ///
    /// The broadest grant here: it is not scoped to Claude at all, and reaches
    /// anything the owner's desktop session can reach.
    ComputerControl,
}

impl AgentCapability {
    /// Wire string, matching the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CrossSessionNote => "cross-session-note",
            Self::CrossSessionActivate => "cross-session-activate",
            Self::CrossSessionRead => "cross-session-read",
            Self::DesktopControl => "desktop-control",
            Self::ComputerControl => "computer-control",
        }
    }

    /// Parse a wire string, rejecting anything unrecognised.
    ///
    /// Fail-closed on purpose: an unknown capability name is a typo, a rename,
    /// or a record written by a newer build. Silently dropping it would grant
    /// less than the owner asked for while reporting success; silently keeping
    /// it would carry an unenforceable grant forward. Refusing makes the
    /// mismatch visible at the boundary where it can still be fixed.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "cross-session-note" => Ok(Self::CrossSessionNote),
            "cross-session-activate" => Ok(Self::CrossSessionActivate),
            "cross-session-read" => Ok(Self::CrossSessionRead),
            "desktop-control" => Ok(Self::DesktopControl),
            "computer-control" => Ok(Self::ComputerControl),
            other => Err(format!(
                "unknown agent capability `{other}` (expected one of: {})",
                Self::ALL
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Every capability, in wire order. Used for validation messages and to
    /// keep the UI and the parser from drifting apart.
    pub const ALL: [AgentCapability; 5] = [
        Self::CrossSessionNote,
        Self::CrossSessionActivate,
        Self::CrossSessionRead,
        Self::DesktopControl,
        Self::ComputerControl,
    ];
}

impl fmt::Display for AgentCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An owner's grant list. `BTreeSet` so the stored order is deterministic and
/// a re-save never produces a spurious diff.
pub type AgentCapabilities = BTreeSet<AgentCapability>;

/// Validate a wire-shape grant list into a set.
///
/// Empty input yields an empty set — the deny-by-default value.
pub fn parse_capabilities(raw: &[String]) -> Result<AgentCapabilities, String> {
    raw.iter()
        .map(|value| AgentCapability::parse(value))
        .collect()
}

/// Render a grant list for the spawned process environment.
///
/// Comma-separated wire strings, or the empty string when nothing is granted.
/// The empty string and an unset variable mean the same thing — no grants —
/// so a harness that has never heard of this variable behaves identically to
/// one that reads it and finds nothing.
pub fn capabilities_env_value(caps: &AgentCapabilities) -> String {
    caps.iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// Resolve the grants that govern a spawning agent.
///
/// # Why this is a resolve and not a field read
///
/// Grants have no instance-level override: an agent holds what its definition
/// grants, or nothing. A property with no override semantics must never be
/// *copied* onto the instance — a copy is a second source of truth, and the
/// moment the owner edits the definition the two disagree with no signal that
/// they have. That is exactly the bug this function exists to remove: a grant
/// ticked after an agent was minted landed on the definition and never reached
/// the running process, which reported `capabilities=[]` while the UI showed
/// the box ticked.
///
/// Reading through to the definition on every spawn makes the drift
/// unrepresentable rather than merely unlikely.
///
/// # Failing closed
///
/// An agent whose definition cannot be found spawns with nothing granted, and
/// says so at `warn`. Returns the wire strings unvalidated; the caller parses
/// them, so an unrecognized grant fails the spawn rather than being silently
/// dropped.
pub fn resolve_agent_capabilities(
    persona_id: Option<&str>,
    definitions: &[crate::managed_agents::types::AgentDefinition],
) -> Vec<String> {
    let Some(persona_id) = persona_id else {
        // No linked definition: nothing to inherit, and no override exists to
        // inherit it from. Silent because this is a normal shape, not a fault.
        return Vec::new();
    };

    match definitions.iter().find(|d| d.id == persona_id) {
        Some(definition) => definition.capabilities.clone(),
        // Fail closed, but never silently.
        //
        // Two different situations land here and this function cannot tell
        // them apart: the definition was deleted (returning nothing is
        // correct), or `load_personas` failed and handed us an empty slice —
        // it is called as `.unwrap_or_default()` — in which case every agent
        // on the machine silently loses every grant.
        //
        // Both must fail closed; only one is a bug. The warning is what makes
        // the bug reportable, and it carries `definitions_loaded` precisely so
        // the two can be told apart after the fact: a lone unmatched id among
        // many loaded definitions is a deletion, while a zero count across
        // every agent is a failed load.
        None => {
            tracing::warn!(
                persona_id = %persona_id,
                definitions_loaded = definitions.len(),
                "spawning agent with no capability grants: no definition matched its persona_id"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(
        id: &str,
        capabilities: &[&str],
    ) -> crate::managed_agents::types::AgentDefinition {
        let mut d: crate::managed_agents::types::AgentDefinition =
            serde_json::from_value(serde_json::json!({
                "id": id,
                "display_name": id,
                "system_prompt": "",
                "created_at": "",
                "updated_at": "",
            }))
            .expect("minimal definition must deserialize");
        d.capabilities = capabilities.iter().map(|c| c.to_string()).collect();
        d
    }

    #[test]
    fn grants_are_read_from_the_linked_definition() {
        let defs = vec![
            definition("other", &["computer-control"]),
            definition("linked", &["cross-session-note"]),
        ];
        assert_eq!(
            resolve_agent_capabilities(Some("linked"), &defs),
            vec!["cross-session-note".to_string()],
            "an agent must hold what its own definition grants, not another's"
        );
    }

    /// The regression this whole function exists for: a grant added to the
    /// definition after an agent was minted must reach that agent's next
    /// spawn. Nothing on the instance is consulted, so there is no copy left
    /// to go stale.
    #[test]
    fn a_grant_added_after_minting_reaches_the_next_spawn() {
        let mut defs = vec![definition("linked", &[])];
        assert_eq!(
            resolve_agent_capabilities(Some("linked"), &defs),
            Vec::<String>::new()
        );

        defs[0].capabilities = vec!["cross-session-note".to_string()];

        assert_eq!(
            resolve_agent_capabilities(Some("linked"), &defs),
            vec!["cross-session-note".to_string()],
            "editing the definition must change what the next spawn is granted"
        );
    }

    #[test]
    fn an_agent_with_no_definition_holds_nothing() {
        let defs = vec![definition("linked", &["computer-control"])];
        assert_eq!(
            resolve_agent_capabilities(None, &defs),
            Vec::<String>::new(),
            "an unlinked agent must not inherit some other definition's grants"
        );
    }

    /// Deleted definition and failed load are indistinguishable here. Both
    /// fail closed; the `warn` is what makes the second one reportable.
    #[test]
    fn an_unresolvable_definition_fails_closed() {
        assert_eq!(
            resolve_agent_capabilities(
                Some("deleted"),
                &[definition("other", &["computer-control"])]
            ),
            Vec::<String>::new(),
            "an agent whose definition is gone must not keep its grants"
        );
        assert_eq!(
            resolve_agent_capabilities(Some("linked"), &[]),
            Vec::<String>::new(),
            "an empty definition list must not be treated as a grant"
        );
    }

    #[test]
    fn wire_strings_round_trip_through_parse() {
        for cap in AgentCapability::ALL {
            assert_eq!(
                AgentCapability::parse(cap.as_str()),
                Ok(cap),
                "{cap} must parse back from its own wire string"
            );
        }
    }

    #[test]
    fn serde_uses_the_same_wire_strings_as_as_str() {
        // A drift between these two is invisible until a stored record fails to
        // load, so pin them against each other rather than against a literal.
        for cap in AgentCapability::ALL {
            let json = serde_json::to_string(&cap).expect("serialize");
            assert_eq!(
                json,
                format!("\"{}\"", cap.as_str()),
                "serde and as_str disagree for {cap}"
            );
        }
    }

    #[test]
    fn unknown_capability_is_refused_rather_than_dropped() {
        let err = AgentCapability::parse("cross-session-everything")
            .expect_err("an unknown capability must not parse");
        assert!(
            err.contains("cross-session-everything"),
            "the error must name the offending value; got: {err}"
        );
        assert!(
            err.contains("cross-session-note"),
            "the error must list what is accepted; got: {err}"
        );
    }

    #[test]
    fn empty_grant_list_is_the_default_and_survives_a_round_trip() {
        let caps = parse_capabilities(&[]).expect("empty is valid");
        assert!(caps.is_empty(), "no grants is the deny-by-default value");
        assert_eq!(
            capabilities_env_value(&caps),
            "",
            "no grants must render as the empty string, not a placeholder"
        );
    }

    #[test]
    fn env_value_is_deterministic_regardless_of_input_order() {
        // Two owners ticking the same boxes in a different order must produce
        // byte-identical records, or every save churns the store.
        let a = parse_capabilities(&["computer-control".into(), "cross-session-note".into()])
            .expect("valid");
        let b = parse_capabilities(&["cross-session-note".into(), "computer-control".into()])
            .expect("valid");
        assert_eq!(capabilities_env_value(&a), capabilities_env_value(&b));
    }

    #[test]
    fn a_duplicate_grant_collapses() {
        let caps = parse_capabilities(&["desktop-control".into(), "desktop-control".into()])
            .expect("valid");
        assert_eq!(caps.len(), 1);
    }
}
