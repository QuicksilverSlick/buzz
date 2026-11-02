//! Audio output device enumeration, selection, and sink creation.

use tauri::State;

use crate::app_state::AppState;

/// List available audio output devices. Returns (name, is_default) pairs.
#[tauri::command]
pub async fn list_audio_output_devices() -> Result<Vec<AudioOutputDevice>, String> {
    tokio::task::spawn_blocking(list_audio_output_devices_blocking)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

fn list_audio_output_devices_blocking() -> Result<Vec<AudioOutputDevice>, String> {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;

    let host = rodio::cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()));
    let devices = host
        .output_devices()
        .map_err(|e| format!("enumerate output devices: {e}"))?;

    let mut result = Vec::new();
    for device in devices {
        if let Ok(name) = device.description().map(|d| d.name().to_string()) {
            let is_default = default_name.as_deref() == Some(&name);
            result.push(AudioOutputDevice { name, is_default });
        }
    }
    Ok(result)
}

/// Set the preferred audio output device by name. Empty string = system default.
/// Takes effect on the next huddle start/join (does not change a live stream).
#[tauri::command]
pub fn set_audio_output_device(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state
        .huddle_audio
        .output_device
        .lock()
        .map_err(|e| e.to_string())?;
    *guard = if name.is_empty() { None } else { Some(name) };
    Ok(())
}

/// Get the currently selected audio output device name (empty = system default).
#[tauri::command]
pub fn get_audio_output_device(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state
        .huddle_audio
        .output_device
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(guard.clone().unwrap_or_default())
}

#[derive(Debug, serde::Serialize)]
pub struct AudioOutputDevice {
    pub name: String,
    pub is_default: bool,
}

/// Open a rodio sink for a named output device, falling back to default.
pub(crate) fn open_output_sink_by_name(
    preferred: Option<&str>,
) -> Result<rodio::MixerDeviceSink, String> {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;

    if let Some(name) = preferred {
        let host = rodio::cpal::default_host();
        if let Ok(devices) = host.output_devices() {
            for device in devices {
                if device
                    .description()
                    .ok()
                    .map(|d| d.name().to_string())
                    .as_deref()
                    == Some(name)
                {
                    if let Ok(sink) = rodio::DeviceSinkBuilder::from_device(device) {
                        return sink
                            .open_stream()
                            .map_err(|e| format!("audio output ({name}): {e}"));
                    }
                }
            }
        }
        eprintln!(
            "buzz-desktop: preferred output device {name:?} not found, falling back to default"
        );
    }

    rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| format!("audio output: {e}"))
}

fn device_type_is_isolated(device_type: rodio::cpal::DeviceType) -> bool {
    use rodio::cpal::DeviceType;
    matches!(
        device_type,
        DeviceType::Headphones
            | DeviceType::Headset
            | DeviceType::Earpiece
            | DeviceType::HearingAid
    )
}

/// Conservative route-isolation query using cpal's safe structured device
/// description. This is intentionally re-evaluated at confirmed local onset,
/// so a route change cannot leave a stale isolated capability behind.
pub(crate) fn output_route_is_isolated(preferred: Option<&str>) -> bool {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;

    let host = rodio::cpal::default_host();
    let device = match preferred.filter(|name| !name.is_empty()) {
        Some(name) => {
            let Ok(devices) = host.output_devices() else {
                return false;
            };
            let mut matches = devices.filter(|device| {
                device
                    .description()
                    .ok()
                    .map(|description| description.name().to_owned())
                    == Some(name.to_owned())
            });
            let Some(device) = matches.next() else {
                return false;
            };
            if matches.next().is_some() {
                return false;
            }
            device
        }
        None => match host.default_output_device() {
            Some(device) => device,
            None => return false,
        },
    };

    device
        .description()
        .is_ok_and(|description| device_type_is_isolated(description.device_type()))
}

/// Keep the route file current while one huddle audio connection runs and a
/// local agent has voice on; remove it otherwise and when the connection is
/// cancelled (leaving the huddle). Rewritten every 5 s so `updated_at` doubles
/// as a heartbeat: a reader treats a stale file as not here and not isolated,
/// which also covers a crash.
pub(crate) fn spawn_output_route_writer(
    app: Option<tauri::AppHandle>,
    huddle: String,
    output_device: Option<String>,
    cancel: tokio_util::sync::CancellationToken,
) {
    use crate::managed_agents::voice_profile;
    let Some((app, path)) =
        app.and_then(|app| voice_profile::output_route_file(&app).map(|path| (app, path)))
    else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        // A fixed beat, so a slow device query never stretches the period past 5 s.
        let mut beat = tokio::time::interval(std::time::Duration::from_secs(5));
        beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                _ = beat.tick() => {}
            }
            // The store read and the device query both block, so off the runtime.
            let (app, path, huddle, device) = (
                app.clone(),
                path.clone(),
                huddle.clone(),
                output_device.clone(),
            );
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let isolated = voice_profile::any_voice_enabled(&app)
                    .then(|| output_route_is_isolated(device.as_deref()));
                sync_output_route_file(&path, &huddle, isolated);
            })
            .await;
        }
        let _ = std::fs::remove_file(&path);
    });
}

/// Write `{isolated, huddle, updated_at}` (`huddle` is the huddle channel id,
/// `updated_at` unix seconds), or remove the file when no voice needs it
/// (`None`). This is the shape `buzz_huddle_bridge::input::read_route` reads.
fn sync_output_route_file(path: &std::path::Path, huddle: &str, isolated: Option<bool>) {
    let Some(isolated) = isolated else {
        let _ = std::fs::remove_file(path);
        return;
    };
    let updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let body = serde_json::json!({
        "isolated": isolated,
        "huddle": huddle,
        "updated_at": updated_at,
    })
    .to_string();
    if let Err(error) =
        crate::managed_agents::storage::atomic_write_json_restricted(path, body.as_bytes())
    {
        eprintln!("buzz-desktop: huddle output route file: {error}");
    }
}

#[cfg(test)]
mod route_isolation_tests {
    use super::{device_type_is_isolated, sync_output_route_file};
    use rodio::cpal::DeviceType;

    #[test]
    fn only_positive_isolated_terminal_types_are_accepted() {
        assert!(device_type_is_isolated(DeviceType::Headphones));
        assert!(device_type_is_isolated(DeviceType::Headset));
        assert!(device_type_is_isolated(DeviceType::Earpiece));
        assert!(device_type_is_isolated(DeviceType::HearingAid));
        assert!(!device_type_is_isolated(DeviceType::Speaker));
        assert!(!device_type_is_isolated(DeviceType::Virtual));
        assert!(!device_type_is_isolated(DeviceType::Unknown));
    }

    /// Pins the exact shape the bridge reads; its reader side is
    /// `route_file_counts_only_for_this_huddle_while_fresh` in
    /// `crates/buzz-huddle-bridge/src/input.rs`. Change both together.
    #[test]
    fn output_route_file_has_the_bridge_shape_then_is_removed() {
        const HUDDLE: &str = "5b6f8c2e-1d3a-4c8e-9f21-7a0b3c4d5e6f";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huddle-output-route.json");
        sync_output_route_file(&path, HUDDLE, Some(true));
        let body: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let updated_at = body["updated_at"].as_u64().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Unix seconds, not milliseconds: the reader multiplies by 1000.
        assert!(
            now.abs_diff(updated_at) <= 5,
            "updated_at={updated_at} now={now}"
        );
        assert_eq!(
            body,
            serde_json::json!({ "isolated": true, "huddle": HUDDLE, "updated_at": updated_at })
        );
        sync_output_route_file(&path, HUDDLE, None);
        assert!(!path.exists());
    }
}
