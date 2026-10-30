//! Huddle voice profile handed over by the desktop (huddle voices, slice 9).
//!
//! The desktop sets `BUZZ_ACP_VOICE_PROFILE` only while the definition's
//! switch is on. Read-only until the voice module (slice 8a): today only the
//! startup summary mentions it.
//!
//! Not bounded here. Before persona or aliases reach GPT-Live instructions or
//! name matching, slice 8a must re-apply the desktop's length, count and
//! visible-text checks, since any launcher can set this env value.

use serde::Deserialize;

/// Mirrors the desktop's `VoiceProfile`. Unknown keys are ignored, so a newer
/// desktop never stops an older harness from starting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct VoiceProfile {
    #[serde(default)]
    pub enabled: bool,
    /// GPT-Live built-in voice name or custom voice id; `None` = default.
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub persona: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub voice_without_owner: bool,
}

pub(crate) fn parse_voice_profile(raw: &str) -> Result<VoiceProfile, String> {
    serde_json::from_str(raw).map_err(|error| format!("invalid voice profile JSON: {error}"))
}
