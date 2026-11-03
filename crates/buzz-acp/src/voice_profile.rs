//! Huddle voice profile handed over by the desktop (huddle voices, slice 9).
//!
//! The desktop sets `BUZZ_ACP_VOICE_PROFILE` only while the definition's
//! switch is on. The voice module takes it through [`VoiceProfile::wire`],
//! which re-applies the desktop's bounds, since any launcher can set this env
//! value.

use buzz_huddle_bridge::frames::Profile;
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

impl VoiceProfile {
    /// What reaches Talos, under the desktop's `normalize_voice_profile` bounds. With no voice
    /// named it's GPT-Live's own default (design §3.1).
    pub(crate) fn wire(&self) -> Result<Profile, String> {
        let voice = bounded(self.voice.as_deref(), 128, false, "name")?;
        let persona = bounded(self.persona.as_deref(), 1_000, true, "persona")?;
        let mut aliases: Vec<String> = Vec::new();
        for alias in &self.aliases {
            let alias = bounded(Some(alias), 40, false, "alias")?;
            if !alias.is_empty() && !aliases.iter().any(|a| a.eq_ignore_ascii_case(&alias)) {
                aliases.push(alias);
            }
        }
        if aliases.len() > 8 {
            return Err("a voice takes at most 8 aliases".to_owned());
        }
        let voice = if voice.is_empty() {
            "marin".to_owned()
        } else {
            voice
        };
        Ok(Profile {
            voice,
            persona,
            aliases,
        })
    }
}

/// Trimmed, within `max`, and free of control and invisible characters. The desktop allows a
/// joiner or variation selector inside an emoji, so those two are stripped here, not refused. The
/// error never quotes the text.
fn bounded(text: Option<&str>, max: usize, multiline: bool, what: &str) -> Result<String, String> {
    let text = text
        .unwrap_or("")
        .trim()
        .replace(['\u{200D}', '\u{FE0F}'], "");
    if text.chars().count() > max {
        return Err(format!("voice {what} is longer than {max} characters"));
    }
    let layout = |c: char| multiline && matches!(c, '\n' | '\t');
    if text
        .chars()
        .any(|c| (c.is_control() && !layout(c)) || invisible(c))
    {
        return Err(format!("voice {what} has a control or invisible character"));
    }
    Ok(text)
}

/// The desktop's `is_default_ignorable` table.
#[rustfmt::skip]
fn invisible(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x034F | 0x061C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF
        | 0xFFA0 | 0xFFF0..=0xFFF8 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE0FFF)
}

pub(crate) fn parse_voice_profile(raw: &str) -> Result<VoiceProfile, String> {
    serde_json::from_str(raw).map_err(|error| format!("invalid voice profile JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wire(raw: serde_json::Value) -> Result<Profile, String> {
        serde_json::from_value::<VoiceProfile>(raw).unwrap().wire()
    }

    #[test]
    fn the_desktops_bounds_hold_for_any_launcher() {
        let empty = wire(json!({})).unwrap();
        assert_eq!(
            (empty.voice.as_str(), empty.persona.as_str()),
            ("marin", "")
        );
        let raw = json!({ "voice": " cedar ", "persona": "Calm.\nKind. \u{2764}\u{FE0F}",
            "aliases": [" Orch ", "orch", " ", "O"] });
        let good = wire(raw).unwrap();
        assert_eq!(good.voice, "cedar");
        assert_eq!(good.persona, "Calm.\nKind. \u{2764}");
        assert_eq!(good.aliases, ["Orch", "O"]);
        let nine: Vec<String> = (0..9).map(|i| format!("a{i}")).collect();
        for bad in [
            json!({ "voice": "mar\nin" }),
            json!({ "voice": "v".repeat(129) }),
            json!({ "persona": "x\u{7}" }),
            json!({ "persona": "x".repeat(1_001) }),
            json!({ "aliases": ["a\nb"] }),
            json!({ "aliases": ["a".repeat(41)] }),
            json!({ "aliases": ["O\u{200B}"] }),
            json!({ "persona": "x\u{202E}y" }),
            json!({ "aliases": nine }),
        ] {
            assert!(wire(bad.clone()).is_err(), "{bad}");
        }
        let error = wire(json!({ "persona": "secret\u{7}" })).unwrap_err();
        assert!(!error.contains("secret"), "{error}");
    }
}
