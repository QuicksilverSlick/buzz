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
/// cancelled. Rewritten every 5 s so `updated_at` doubles as a heartbeat: a
/// reader treats a stale file as not isolated, which also covers a crash.
pub(crate) fn spawn_output_route_writer(
    app: Option<tauri::AppHandle>,
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
        loop {
            if voice_profile::any_voice_enabled(&app) {
                write_output_route(&path, output_device.clone()).await;
            } else {
                let _ = std::fs::remove_file(&path);
            }
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
            }
        }
        let _ = std::fs::remove_file(&path);
    });
}

async fn write_output_route(path: &std::path::Path, output_device: Option<String>) {
    let isolated =
        tokio::task::spawn_blocking(move || output_route_is_isolated(output_device.as_deref()))
            .await
            .unwrap_or(false);
    let updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let body = serde_json::json!({ "isolated": isolated, "updated_at": updated_at }).to_string();
    if let Err(error) =
        crate::managed_agents::storage::atomic_write_json_restricted(path, body.as_bytes())
    {
        eprintln!("buzz-desktop: huddle output route file: {error}");
    }
}

#[cfg(test)]
mod route_isolation_tests {
    use super::device_type_is_isolated;
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
}
