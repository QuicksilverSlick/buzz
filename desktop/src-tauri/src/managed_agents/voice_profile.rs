//! Huddle voice profile for an agent definition (huddle voices, slice 9).
//!
//! Stored on the definition and handed to buzz-acp at spawn only while the
//! switch is on. Nothing speaks from it yet: the voice module (slice 8a) is
//! the first consumer. Off by default, so an untouched definition carries no
//! profile and its stored bytes do not move.

use std::{path::Path, process::Command};

use serde::{Deserialize, Serialize};

use super::{AgentDefinition, ManagedAgentRecord};

pub(crate) const VOICE_PROFILE_ENV_VAR: &str = "BUZZ_ACP_VOICE_PROFILE";
pub(crate) const AUDIO_ROUTE_FILE_ENV_VAR: &str = "BUZZ_ACP_AUDIO_ROUTE_FILE";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceProfile {
    /// The switch. Off by default; no voice joins a huddle while it is off.
    #[serde(default)]
    pub enabled: bool,
    /// GPT-Live built-in voice name (e.g. `marin`) or a custom voice id.
    /// `None` means the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// Short spoken persona. The task backend still gets the full prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    /// Names the voice answers to in a huddle, besides its display name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Join while the owner is absent. Off until Phase 4 (decision 10).
    #[serde(default, alias = "voiceWithoutOwner")]
    pub voice_without_owner: bool,
}

/// Validate and normalize an authored profile. An all-default profile
/// normalizes to `None`, so a definition that never had a voice keeps no key.
pub(crate) fn normalize_voice_profile(
    profile: Option<VoiceProfile>,
) -> Result<Option<VoiceProfile>, String> {
    let Some(profile) = profile else {
        return Ok(None);
    };
    let trimmed = |value: String| Some(value.trim().to_string()).filter(|v| !v.is_empty());
    let plain = |text: &str, max: usize, newlines: bool, what: &str| {
        let bad = |c: char| c.is_control() && !(newlines && c == '\n');
        let ok = text.chars().count() <= max && !text.chars().any(bad);
        ok.then_some(())
            .ok_or_else(|| format!("voice {what} must be plain text of at most {max} characters"))
    };
    let voice = profile.voice.and_then(trimmed);
    if let Some(voice) = &voice {
        plain(voice, 128, false, "name")?;
    }
    let persona = profile.persona.and_then(trimmed);
    if let Some(persona) = &persona {
        plain(persona, 1_000, true, "persona")?;
    }
    let mut aliases: Vec<String> = Vec::new();
    for alias in profile.aliases.into_iter().filter_map(trimmed) {
        plain(&alias, 40, false, "alias")?;
        if !aliases.iter().any(|seen| seen.eq_ignore_ascii_case(&alias)) {
            aliases.push(alias);
        }
    }
    if aliases.len() > 8 {
        return Err("a voice takes at most 8 aliases".to_string());
    }
    let profile = VoiceProfile {
        voice,
        persona,
        aliases,
        ..profile
    };
    Ok((profile != VoiceProfile::default()).then_some(profile))
}

/// Stamp the voice env at spawn: the linked definition's profile, only while
/// switched on. Both keys are reserved, so user env can't switch a voice on.
pub(crate) fn apply_voice_env(
    command: &mut Command,
    persona_id: Option<&str>,
    definitions: &[AgentDefinition],
    route_file: Option<&Path>,
) {
    command.env_remove(VOICE_PROFILE_ENV_VAR);
    command.env_remove(AUDIO_ROUTE_FILE_ENV_VAR);
    let profile = definitions
        .iter()
        .find(|definition| Some(definition.id.as_str()) == persona_id)
        .and_then(|definition| definition.voice_profile.as_ref())
        .filter(|profile| profile.enabled)
        .and_then(|profile| serde_json::to_string(profile).ok());
    if let Some(profile) = profile {
        command.env(VOICE_PROFILE_ENV_VAR, profile);
        if let Some(route_file) = route_file {
            command.env(AUDIO_ROUTE_FILE_ENV_VAR, route_file);
        }
    }
}

/// Where the desktop keeps `{isolated, updated_at}` for its live huddle output
/// route (design 3.3). Passed to voiced agents as `BUZZ_ACP_AUDIO_ROUTE_FILE`;
/// a missing or stale file means not isolated.
pub(crate) fn output_route_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Option<std::path::PathBuf> {
    super::managed_agents_base_dir(app)
        .ok()
        .map(|dir| dir.join("huddle-output-route.json"))
}

fn voice_on(record: &ManagedAgentRecord) -> bool {
    record
        .definition_voice_profile
        .as_ref()
        .is_some_and(|profile| profile.enabled)
}

/// Whether any local definition has its voice switched on.
pub(crate) fn any_voice_enabled<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    super::storage::load_agent_store(app).is_ok_and(|store| store.iter().any(voice_on))
}

/// Whether any of `huddle_agents` is one of this desktop's agents with voice
/// switched on. `store` is the unified agent store (definitions and instances).
fn any_local_agent_voiced(store: &[ManagedAgentRecord], huddle_agents: &[String]) -> bool {
    let voiced = |slug: &str| {
        store.iter().any(|definition| {
            definition.pubkey.is_empty()
                && definition.slug.as_deref() == Some(slug)
                && voice_on(definition)
        })
    };
    store.iter().any(|agent| {
        !agent.pubkey.is_empty()
            && huddle_agents
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&agent.pubkey))
            && agent.persona_id.as_deref().is_some_and(voiced)
    })
}

/// For the huddle speech-to-text poster. A store read failure posts as
/// before: the switch can't be confirmed on.
pub(crate) fn huddle_has_voiced_local_agent<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    huddle_agents: &[String],
) -> bool {
    !huddle_agents.is_empty()
        && super::storage::load_agent_store(app)
            .inspect_err(|error| eprintln!("buzz-desktop: huddle voice check: {error}"))
            .is_ok_and(|store| any_local_agent_voiced(&store, huddle_agents))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(enabled: bool) -> Option<VoiceProfile> {
        serde_json::from_value(json!({ "enabled": enabled, "voice": "marin" })).ok()
    }

    #[test]
    fn profiles_are_trimmed_deduped_and_bounded() {
        let check = |raw| normalize_voice_profile(serde_json::from_value(raw).unwrap());
        assert_eq!(check(json!({})), Ok(None));
        let raw = json!({ "voice": " marin ", "persona": " Calm. ", "voiceWithoutOwner": true,
            "aliases": [" Orch ", "orch", " ", "O"] });
        let normalized = check(raw).unwrap().unwrap();
        assert_eq!(normalized.voice.as_deref(), Some("marin"));
        assert_eq!(normalized.persona.as_deref(), Some("Calm."));
        assert_eq!(normalized.aliases, ["Orch", "O"]);
        assert!(!normalized.enabled && normalized.voice_without_owner);
        assert!(check(json!({ "persona": "line one\nline two" })).is_ok());
        let nine: Vec<String> = (0..9).map(|i| format!("a{i}")).collect();
        for bad in [
            json!({ "voice": "mar\nin" }),
            json!({ "voice": "v".repeat(129) }),
            json!({ "persona": "x\u{7}" }),
            json!({ "persona": "x".repeat(1_001) }),
            json!({ "aliases": ["a\nb"] }),
            json!({ "aliases": nine }),
        ] {
            assert!(check(bad.clone()).is_err(), "{bad}");
        }
    }

    /// Definition + linked instance, through the real view <-> record arms.
    fn store(voice: Option<VoiceProfile>) -> Vec<ManagedAgentRecord> {
        let json = r#"{"id":"orch","display_name":"O","system_prompt":"","created_at":"","updated_at":""}"#;
        let mut definition: AgentDefinition = serde_json::from_str(json).unwrap();
        definition.voice_profile = voice;
        let mut instance = definition.clone().into_agent_record();
        (instance.pubkey, instance.slug) = ("AB".repeat(32), None);
        (instance.persona_id, instance.definition_voice_profile) = (Some("orch".into()), None);
        vec![definition.into_agent_record(), instance]
    }

    #[test]
    fn huddle_check_and_spawn_env_follow_the_definition_switch() {
        let ours = ["ab".repeat(32)];
        let voiced = |voice| any_local_agent_voiced(&store(voice), &ours);
        assert!(voiced(profile(true)) && !voiced(profile(false)) && !voiced(None));
        let theirs = ["cd".repeat(32)];
        assert!(!any_local_agent_voiced(&store(profile(true)), &theirs));

        let env = |voice, key: &str| {
            let definitions = [store(voice)[0].to_definition_view().unwrap()];
            let route = Some(Path::new("r.json"));
            let mut command = Command::new("x");
            apply_voice_env(&mut command, Some("orch"), &definitions, route);
            let value = command.get_envs().find(|(k, _)| *k == key);
            value
                .and_then(|(_, v)| v)
                .map(|v| v.to_string_lossy().into_owned())
        };
        let json = env(profile(true), VOICE_PROFILE_ENV_VAR).unwrap();
        assert_eq!(serde_json::from_str(&json).ok(), profile(true));
        let route = env(profile(true), AUDIO_ROUTE_FILE_ENV_VAR);
        assert_eq!(route.as_deref(), Some("r.json"));
        assert_eq!(env(profile(false), VOICE_PROFILE_ENV_VAR), None);
        assert_eq!(env(profile(false), AUDIO_ROUTE_FILE_ENV_VAR), None);
    }
}
