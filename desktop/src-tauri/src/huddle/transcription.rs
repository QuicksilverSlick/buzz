use tauri::State;

use crate::app_state::AppState;

use super::{models, pipeline::maybe_start_stt_pipeline, stt};

/// A five-minute voice note is ~14.4 MB at the encoder's 24 kHz 16-bit mono.
const MAX_VOICE_NOTE_WAV_BYTES: usize = 20 * 1024 * 1024;

/// Transcribe a voice note the composer just recorded (raw WAV body).
///
/// A voice note is audio only, and agents read text: without this the agent
/// gets a player it cannot hear. The composer puts the words in the message,
/// where the sender can correct them before sending.
///
/// Errors say why in words the composer can show; the detail goes to the log.
/// A missing model starts its download, so the next note can be transcribed.
#[tauri::command]
pub async fn transcribe_voice_note(
    request: tauri::ipc::Request<'_>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw binary body".to_string());
    };
    if bytes.len() > MAX_VOICE_NOTE_WAV_BYTES {
        return Err("the voice note is too long to transcribe".to_string());
    }
    let model_dir = models::stt_model_dir().filter(|_| models::is_stt_ready());
    let Some(model_dir) = model_dir else {
        let Some(manager) = models::global_model_manager() else {
            return Err("the speech model isn't available on this computer".to_string());
        };
        manager.start_stt_download(state.http_client.clone());
        return Err("the speech model isn't downloaded yet; it's downloading now".to_string());
    };
    let wav = bytes.to_vec();
    tauri::async_runtime::spawn_blocking(move || stt::transcribe_voice_note_wav(&model_dir, &wav))
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result)
        .map_err(|detail| {
            eprintln!("buzz-desktop: voice note transcription failed: {detail}");
            "the speech model couldn't read it".to_string()
        })
}

/// Start the STT pipeline for the active huddle.
///
/// Delegates to `maybe_start_stt_pipeline` — returns `Err` if models are not
/// ready or no huddle is active. Safe to call multiple times: replaces the
/// existing pipeline if already running.
#[tauri::command]
pub async fn start_stt_pipeline(state: State<'_, AppState>) -> Result<(), String> {
    let ephemeral_channel_id = {
        let mut hs = state.huddle()?;
        let ephemeral_channel_id = hs
            .ephemeral_channel_id
            .clone()
            .ok_or("no active huddle — start or join a huddle first")?;
        hs.set_transcription_enabled_by_user(true);
        ephemeral_channel_id
    };

    match maybe_start_stt_pipeline(&state, &ephemeral_channel_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err("STT model not ready".to_string()),
        Err(e) => Err(e),
    }
}

/// Enable or disable huddle transcript posting.
///
/// Disabling tears down STT immediately and invalidates any in-flight transcript
/// task before it can post another segment. Enabling starts STT if models are
/// ready; otherwise the hot-start loop will begin transcribing once the model
/// download finishes.
#[tauri::command]
pub async fn set_huddle_transcription_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (ephemeral_channel_id, old_stt) = {
        let mut hs = state.huddle()?;
        let ephemeral_channel_id = hs
            .ephemeral_channel_id
            .clone()
            .ok_or("no active huddle — start or join a huddle first")?;
        hs.set_transcription_enabled_by_user(enabled);

        if enabled {
            (ephemeral_channel_id, None)
        } else {
            hs.invalidate_transcription_pipeline();
            (ephemeral_channel_id, hs.take_stt_pipeline())
        }
    };

    if let Some(ref pipeline) = old_stt {
        pipeline.shutdown();
    }
    drop(old_stt);

    if enabled {
        if let Some(manager) = models::global_model_manager() {
            manager.start_stt_download(state.http_client.clone());
        }
        if let Err(e) = maybe_start_stt_pipeline(&state, &ephemeral_channel_id).await {
            eprintln!("buzz-desktop: STT transcript start failed: {e}");
        }
    }

    state.emit_huddle_state_changed();
    Ok(())
}
