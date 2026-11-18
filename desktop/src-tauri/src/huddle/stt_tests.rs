use std::sync::{atomic::AtomicBool, mpsc, Arc, Barrier};

use super::{
    has_enough_voiced_audio, run_stt_receive_loop, vad_flush_allowed, HumanFloor, SttLoopInput,
    VadEndpoint, VadFrameAction, MIN_VOICED_FRAMES, SILENCE_FLUSH_FRAMES, VAD_FRAME_SAMPLES,
    VAD_ONSET_FRAMES, VAD_PRE_ROLL_FRAMES,
};

#[derive(Clone, Copy)]
enum WorkerExit {
    Shutdown,
    SenderDisconnect,
}

fn assert_worker_exit_releases_floor(exit: WorkerExit) {
    let human_floor = HumanFloor::new();
    let shutdown = Arc::new(AtomicBool::new(false));
    let (audio_tx, audio_rx) = mpsc::channel();
    let acquired = Arc::new(Barrier::new(2));
    let worker_floor = human_floor.clone();
    let worker_shutdown = Arc::clone(&shutdown);
    let worker_acquired = Arc::clone(&acquired);
    let worker = std::thread::spawn(move || {
        run_stt_receive_loop(
            audio_rx,
            &worker_shutdown,
            worker_floor.clone(),
            |input, local_barge_in_state| {
                if matches!(input, SttLoopInput::Batch(_)) && !worker_floor.is_blocked() {
                    local_barge_in_state.acquire(&worker_floor, true, false);
                    worker_acquired.wait();
                }
            },
        );
    });

    audio_tx.send(Vec::new()).expect("worker receiver is open");
    acquired.wait();
    assert!(human_floor.is_blocked());
    match exit {
        WorkerExit::Shutdown => {
            shutdown.store(true, std::sync::atomic::Ordering::Release);
        }
        WorkerExit::SenderDisconnect => drop(audio_tx),
    }
    worker.join().expect("worker exits cleanly");

    let replacement_epoch = human_floor.epoch();
    assert!(
        human_floor.permits(replacement_epoch),
        "fresh TTS authorization must proceed after worker exit"
    );
    assert!(human_floor.enter_local(true, false));
}

#[test]
fn worker_shutdown_releases_local_floor_for_replacement() {
    assert_worker_exit_releases_floor(WorkerExit::Shutdown);
}

#[test]
fn worker_channel_disconnect_releases_local_floor_for_replacement() {
    assert_worker_exit_releases_floor(WorkerExit::SenderDisconnect);
}

fn frame(value: f32) -> Vec<f32> {
    vec![value; VAD_FRAME_SAMPLES]
}

#[test]
fn short_vad_blips_do_not_reach_the_recognizer() {
    assert!(!has_enough_voiced_audio(1));
    assert!(!has_enough_voiced_audio(MIN_VOICED_FRAMES - 1));
    assert!(has_enough_voiced_audio(MIN_VOICED_FRAMES));
}

#[test]
fn confirmed_onset_prepends_pre_roll_once() {
    let mut endpoint = VadEndpoint::new();
    for value in 0..VAD_PRE_ROLL_FRAMES - VAD_ONSET_FRAMES {
        assert_eq!(
            endpoint.process_frame(frame(value as f32), 0.0, true, true, SILENCE_FLUSH_FRAMES),
            VadFrameAction::None
        );
    }
    for value in 0..VAD_ONSET_FRAMES {
        let action = endpoint.process_frame(
            frame(100.0 + value as f32),
            0.9,
            true,
            true,
            SILENCE_FLUSH_FRAMES,
        );
        if value + 1 == VAD_ONSET_FRAMES {
            assert_eq!(action, VadFrameAction::ConfirmedOnset);
        } else {
            assert_eq!(action, VadFrameAction::None);
        }
    }

    assert_eq!(
        endpoint.speech_buf.len(),
        VAD_PRE_ROLL_FRAMES * VAD_FRAME_SAMPLES
    );
    assert_eq!(endpoint.speech_buf[0], 0.0);
    assert_eq!(endpoint.speech_buf[VAD_FRAME_SAMPLES], 1.0);
    assert_eq!(endpoint.pre_roll.len(), 0);
    endpoint.process_frame(frame(200.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    assert_eq!(
        endpoint.speech_buf.len(),
        (VAD_PRE_ROLL_FRAMES + 1) * VAD_FRAME_SAMPLES
    );
}

#[test]
fn onset_requires_consecutive_high_frames() {
    let mut endpoint = VadEndpoint::new();
    for probability in [0.9, 0.9, 0.2, 0.9, 0.9] {
        assert_eq!(
            endpoint.process_frame(frame(1.0), probability, true, true, SILENCE_FLUSH_FRAMES),
            VadFrameAction::None
        );
    }
    assert_eq!(
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES),
        VadFrameAction::ConfirmedOnset
    );
}

#[test]
fn offset_hysteresis_preserves_borderline_speech() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    assert_eq!(
        endpoint.process_frame(frame(2.0), 0.4, true, true, SILENCE_FLUSH_FRAMES),
        VadFrameAction::Speech
    );
    assert_eq!(endpoint.silence_frames, 0);
}

#[test]
fn below_offset_threshold_starts_silence() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    assert_eq!(
        endpoint.process_frame(frame(0.0), 0.3, true, true, SILENCE_FLUSH_FRAMES),
        VadFrameAction::FirstSilence
    );
    assert_eq!(endpoint.silence_frames, 1);
}

#[test]
fn short_segment_reaches_the_visible_drop_path() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let mut action = VadFrameAction::None;
    for _ in 0..SILENCE_FLUSH_FRAMES {
        action = endpoint.process_frame(frame(0.0), 0.0, true, true, SILENCE_FLUSH_FRAMES);
    }
    assert_eq!(action, VadFrameAction::Flush);
    assert!(!has_enough_voiced_audio(endpoint.voiced_frames));
    assert!(!endpoint.speech_buf.is_empty());
}

#[test]
fn silence_flush_retains_only_hangover_audio() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let speech_len = endpoint.speech_buf.len();
    for index in 1..=SILENCE_FLUSH_FRAMES {
        let action = endpoint.process_frame(frame(0.0), 0.0, true, true, SILENCE_FLUSH_FRAMES);
        if index == SILENCE_FLUSH_FRAMES {
            assert_eq!(action, VadFrameAction::Flush);
        }
    }
    assert_eq!(
        endpoint.speech_buf.len(),
        speech_len + 6 * VAD_FRAME_SAMPLES
    );
}

#[test]
fn flush_boundary_never_double_includes_audio() {
    const SEGMENT_N_MARKER: f32 = 777.0;
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(
            frame(SEGMENT_N_MARKER),
            0.9,
            true,
            true,
            SILENCE_FLUSH_FRAMES,
        );
    }
    for _ in 0..SILENCE_FLUSH_FRAMES {
        endpoint.process_frame(
            frame(SEGMENT_N_MARKER),
            0.0,
            true,
            true,
            SILENCE_FLUSH_FRAMES,
        );
    }
    endpoint.reset_segment();

    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(2.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let leaked = endpoint
        .speech_buf
        .iter()
        .filter(|sample| **sample == SEGMENT_N_MARKER)
        .count();
    assert_eq!(leaked, 0, "segment N audio leaked into segment N+1");
}

#[test]
fn reset_prevents_pre_roll_from_leaking_between_segments() {
    const SEGMENT_N_MARKER: f32 = 777.0;
    let mut endpoint = VadEndpoint::new();
    endpoint.pre_roll.push_back(frame(SEGMENT_N_MARKER));
    endpoint.reset_segment();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(2.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let leaked = endpoint
        .speech_buf
        .iter()
        .filter(|sample| **sample == SEGMENT_N_MARKER)
        .count();
    assert_eq!(leaked, 0, "segment N pre-roll leaked into segment N+1");
}

#[test]
fn held_push_to_talk_never_silence_flushes() {
    // Pure VAD mode: silence always ends the utterance.
    assert!(vad_flush_allowed(false, false, false));
    // Shortcut configured, nothing transmitting: nothing to flush anyway,
    // but the pause path stays closed.
    assert!(!vad_flush_allowed(true, false, false));
    // Shortcut held: "I am not done talking" — never flush on silence,
    // regardless of the manual mic state.
    assert!(!vad_flush_allowed(true, false, true));
    assert!(!vad_flush_allowed(true, true, true));
    // Manually open mic with the shortcut up: normal VAD behavior.
    assert!(vad_flush_allowed(true, true, false));
}

fn voice_note_wav(rate: u32, samples: &[i16]) -> Vec<u8> {
    let mut wav = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[test]
fn a_voice_note_wav_parses_to_its_rate_and_samples() {
    let (rate, samples) =
        super::parse_voice_note_wav(&voice_note_wav(24_000, &[0, 16_384, -32_768])).unwrap();
    assert_eq!(rate, 24_000);
    assert_eq!(samples, vec![0.0, 0.5, -1.0]);
}

#[test]
fn anything_but_a_mono_16_bit_voice_note_is_refused() {
    assert!(super::parse_voice_note_wav(b"not audio").is_err());
    let mut stereo = voice_note_wav(24_000, &[0, 0]);
    stereo[22] = 2;
    assert!(super::parse_voice_note_wav(&stereo).is_err());
    let mut truncated = voice_note_wav(24_000, &[1, 2, 3]);
    truncated.truncate(40);
    assert!(super::parse_voice_note_wav(&truncated).is_err());
}

/// Needs the Parakeet model on disk: `BUZZ_STT_MODEL_DIR=<dir> cargo test --
/// --ignored transcribes_real_speech`.
#[test]
#[ignore = "needs the downloaded Parakeet model"]
fn transcribes_real_speech() {
    let dir = std::env::var("BUZZ_STT_MODEL_DIR").expect("BUZZ_STT_MODEL_DIR");
    let wav = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/resources/pocket-voices/paul.wav"
    ))
    .unwrap();
    let text = super::transcribe_voice_note_wav(std::path::Path::new(&dir), &wav).unwrap();
    println!("transcript: {text}");
    assert!(text.split_whitespace().count() > 5, "{text}");
}
