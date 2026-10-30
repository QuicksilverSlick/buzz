//! The voice's audio toward the room, moved from the desktop's agent publisher
//! (`desktop/src-tauri/src/huddle/relay_api.rs`): 24 kHz PCM16 from Talos is upsampled to
//! 48 kHz and cut into 20 ms frames, at most [`QUEUE_FRAMES`] queued, and one frame is encoded
//! per 20 ms tick. It plays only while this voice holds the floor (the speak token), and a
//! human who talks over it cuts it (barge-in). The flush is synchronous, so the desktop's epoch
//! becomes the cut: audio still on its way from Talos is dropped until the room goes quiet.
//!
//! The caller owns the socket and the clock, as for [`crate::input::Input`]: it calls
//! [`Output::hear`] every 10 ms and [`Output::tick`] every 20 ms.

use crate::input::Level;
use buzz_audio_client::encoder::{AudioEncoder, FRAME_SAMPLES};
use buzz_audio_client::wire::audio_level_dbov;
use std::collections::{HashMap, VecDeque};

/// 16 frames, 320 ms: the most a barge-in ever has to cut. ponytail: Talos must pace its PCM to
/// real time; a burst past this is dropped.
pub const QUEUE_FRAMES: usize = 16;
const FRAME_MS: u64 = 20;
const TICK_MS: u64 = 10;

/// Barge-in knobs; probe P measures the real values.
#[derive(Clone, Debug)]
pub struct Barge {
    /// Decoded level (dBov) at or above which a human's 10 ms counts as energy.
    pub speech_dbov: i8,
    /// Energy that cuts the voice from the owner's isolated desktop.
    pub isolated_ms: u64,
    /// Sustained energy above the echo threshold that cuts it from any other peer.
    pub sustained_ms: u64,
    /// The echo threshold: the voice's own recent output level plus this margin.
    pub echo_margin_db: i8,
    /// How long a frame's level counts as recent output.
    pub echo_ms: u64,
    /// Quiet after a cut before the voice may play again.
    pub release_ms: u64,
}

impl Default for Barge {
    fn default() -> Self {
        Self {
            speech_dbov: -45,
            isolated_ms: 200,
            sustained_ms: 320,
            echo_margin_db: 6,
            echo_ms: 600,
            release_ms: 300,
        }
    }
}

pub struct Output {
    me: String,
    barge: Barge,
    encoder: AudioEncoder,
    queue: VecDeque<Vec<f32>>,
    partial: Vec<f32>,
    prev: f32,
    token: u64,
    floor: bool,
    /// Unix ms and level of each frame sent within `echo_ms`.
    sent: VecDeque<(u64, i8)>,
    /// When the last frame sent ends.
    last_end_ms: u64,
    /// Each peer's run of energy while the voice plays.
    runs: HashMap<u8, u64>,
    /// Quiet ms since a cut; `None` when not cut.
    cut: Option<u64>,
}

impl Output {
    /// `me` is this voice's agent pubkey, as `floor` names its holder.
    pub fn new(me: String, barge: Barge) -> Result<Self, String> {
        Ok(Self {
            me,
            barge,
            encoder: AudioEncoder::new()?,
            queue: VecDeque::new(),
            partial: Vec::new(),
            prev: 0.0,
            token: 0,
            floor: false,
            sent: VecDeque::new(),
            last_end_ms: 0,
            runs: HashMap::new(),
            cut: None,
        })
    }

    /// `floor {holder, token}` from Talos. A token older than the last is ignored; without the
    /// floor nothing queues and what was queued goes.
    pub fn set_floor(&mut self, holder: Option<&str>, token: u64) {
        if token < self.token {
            return;
        }
        self.token = token;
        self.floor = holder == Some(self.me.as_str());
        if !self.floor {
            self.flush();
        }
    }

    /// Queue PCM16 LE, 24 kHz mono, from the voice. Dropped without the floor, while cut, and
    /// past the queue cap.
    pub fn on_pcm(&mut self, pcm: &[u8]) {
        if !self.floor || self.cut.is_some() {
            return;
        }
        for s in pcm.as_chunks::<2>().0 {
            let s = f32::from(i16::from_le_bytes(*s)) / 32_768.0;
            // Linear 24 to 48 kHz that runs on across chunks: the midpoint, then the sample.
            self.partial.extend([(self.prev + s) * 0.5, s]);
            self.prev = s;
            if self.partial.len() == FRAME_SAMPLES {
                let frame = std::mem::take(&mut self.partial);
                if self.queue.len() < QUEUE_FRAMES {
                    self.queue.push_back(frame);
                } else {
                    tracing::debug!("huddle bridge: voice frame dropped, queue full");
                }
            }
        }
    }

    /// One 20 ms tick at unix `now_ms`: the next frame for the relay, or nothing. A short tail
    /// goes out once the queue is empty, padded by the encoder.
    pub fn tick(&mut self, now_ms: u64) -> Option<Vec<u8>> {
        let frame = match self.queue.pop_front() {
            Some(frame) => frame,
            None if !self.partial.is_empty() => std::mem::take(&mut self.partial),
            None => return None,
        };
        let payload = self
            .encoder
            .encode(&frame)
            .inspect_err(|e| tracing::warn!(error = %e, "huddle bridge: voice encode"))
            .ok()?;
        self.sent.push_back((now_ms, audio_level_dbov(&frame)));
        self.last_end_ms = now_ms + FRAME_MS;
        Some(payload)
    }

    /// When the voice's output ends, queued frames included, for the half-duplex window.
    pub fn until(&self) -> u64 {
        self.last_end_ms + FRAME_MS * self.queue.len() as u64
    }

    /// Barge-in, every 10 ms, with each admitted human's level. While the voice plays, a human
    /// on the owner's isolated desktop with `isolated_ms` of energy, or any other with
    /// `sustained_ms` above the echo threshold, cuts it: the queue goes, and it stays silent
    /// until no human has spoken for `release_ms`. True on a cut; the caller then ends the
    /// half-duplex window with [`crate::input::Input::output_stopped`].
    pub fn hear(&mut self, levels: impl IntoIterator<Item = Level>, now_ms: u64) -> bool {
        let b = &self.barge;
        while self
            .sent
            .front()
            .is_some_and(|(at, _)| at + b.echo_ms <= now_ms)
        {
            self.sent.pop_front();
        }
        let recent = self.sent.iter().map(|(_, l)| *l).max().unwrap_or(-127);
        let echo = recent.saturating_add(b.echo_margin_db).max(b.speech_dbov);
        let playing = now_ms < self.until();
        let (mut talk, mut barged) = (false, false);
        let runs: HashMap<u8, u64> = levels
            .into_iter()
            .map(|l| {
                let (need, floor) = if l.isolated {
                    (b.isolated_ms, b.speech_dbov)
                } else {
                    (b.sustained_ms, echo)
                };
                let run = match self.runs.get(&l.index) {
                    Some(run) if playing && l.dbov >= floor => run + TICK_MS,
                    None if playing && l.dbov >= floor => TICK_MS,
                    _ => 0,
                };
                talk |= l.dbov >= b.speech_dbov;
                barged |= run >= need;
                (l.index, run)
            })
            .collect();
        self.runs = runs;
        if barged {
            self.flush();
            self.cut = Some(0);
            return true;
        }
        if let Some(quiet) = self.cut {
            let quiet = if talk { 0 } else { quiet + TICK_MS };
            self.cut = (quiet < self.barge.release_ms).then_some(quiet);
        }
        false
    }

    /// Drop everything queued: a cut, a lost floor, or a dropped socket.
    pub fn flush(&mut self) {
        self.queue.clear();
        self.partial.clear();
        self.prev = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const T0: u64 = 1_793_318_400_000;

    /// `ms` of a 24 kHz tone at `amp`, as Talos sends it.
    fn pcm(ms: usize, amp: f32) -> Vec<u8> {
        (0..ms * 24)
            .map(|i| (amp * 32_767.0 * (i as f32 * 0.26).sin()) as i16)
            .flat_map(i16::to_le_bytes)
            .collect()
    }

    fn output() -> Output {
        let mut output = Output::new(ME.into(), Barge::default()).unwrap();
        output.set_floor(Some(ME), 1);
        output
    }

    fn level(index: u8, isolated: bool, dbov: i8) -> Level {
        Level {
            index,
            isolated,
            dbov,
        }
    }

    /// Play `ms` from `now`, with one human at `dbov` each 10 ms; the step a cut came on.
    fn talk_over(o: &mut Output, now: &mut u64, ms: u64, human: Level) -> Option<u64> {
        let start = *now;
        let mut cut = None;
        while *now < start + ms {
            if now.is_multiple_of(FRAME_MS) {
                o.tick(*now);
                o.on_pcm(&pcm(20, 0.1));
            }
            if o.hear([human], *now) && cut.is_none() {
                cut = Some(*now - start);
            }
            *now += TICK_MS;
        }
        cut
    }

    #[test]
    fn frames_are_20_ms_at_48_khz_capped_at_16_and_one_per_tick() {
        let mut o = output();
        // Chunks that don't line up with frames still make whole ones, nothing padded between.
        o.on_pcm(&pcm(7, 0.3));
        o.on_pcm(&pcm(13, 0.3));
        assert_eq!((o.queue.len(), o.partial.len()), (1, 0));
        assert!(o.queue[0].len() == FRAME_SAMPLES);
        o.on_pcm(&pcm(1_000, 0.3));
        assert_eq!(o.queue.len(), QUEUE_FRAMES, "320 ms, the rest dropped");
        assert_eq!(o.until(), FRAME_MS * 16);
        let mut sent = 0;
        for i in 0..20 {
            sent += usize::from(o.tick(T0 + i * FRAME_MS).is_some());
        }
        assert_eq!(sent, QUEUE_FRAMES);
        assert_eq!(o.until(), T0 + 16 * FRAME_MS);
        // A short tail goes out once the queue is empty.
        o.on_pcm(&pcm(5, 0.3));
        assert!(o.tick(T0 + 400).is_some() && o.tick(T0 + 420).is_none());
    }

    #[test]
    fn only_the_floor_holder_plays_and_a_stale_token_changes_nothing() {
        let mut o = Output::new(ME.into(), Barge::default()).unwrap();
        o.on_pcm(&pcm(100, 0.3));
        assert!(o.queue.is_empty(), "no floor yet");
        o.set_floor(Some(ME), 5);
        o.on_pcm(&pcm(100, 0.3));
        o.set_floor(None, 4);
        assert_eq!(o.queue.len(), 5, "token 4 is older");
        o.set_floor(Some("someone else"), 6);
        assert!(o.queue.is_empty(), "losing the floor flushes");
        o.on_pcm(&pcm(100, 0.3));
        assert!(o.queue.is_empty());
    }

    #[test]
    fn an_isolated_owner_cuts_after_200_ms_and_the_cut_holds_until_quiet() {
        let mut o = output();
        o.on_pcm(&pcm(200, 0.1));
        let mut now = T0;
        // The voice's own level is far above the human's; on headphones that doesn't matter.
        assert_eq!(
            talk_over(&mut o, &mut now, 400, level(1, true, -40)),
            Some(190)
        );
        // The cut takes the queued frames off the half-duplex window: the last frame went at 180.
        assert!(o.queue.is_empty());
        assert_eq!(o.until(), T0 + 200);
        // Audio still coming from Talos is dropped until 300 ms with no human talking.
        talk_over(&mut o, &mut now, 500, level(1, true, -40));
        assert!(o.queue.is_empty(), "still talking");
        talk_over(&mut o, &mut now, 290, level(1, true, -60));
        assert!(o.queue.is_empty(), "not quiet for long enough");
        talk_over(&mut o, &mut now, 30, level(1, true, -60));
        assert!(!o.queue.is_empty(), "released");
    }

    #[test]
    fn on_speakers_a_cut_needs_320_ms_above_the_voices_own_level() {
        let mut o = output();
        o.on_pcm(&pcm(200, 0.1));
        let mut now = T0;
        // The voice plays at about -23 dBov, so its echo threshold is about -17 dBov.
        let echo = talk_over(&mut o, &mut now, 2_000, level(2, false, -20));
        assert_eq!(echo, None, "its own echo never cuts it");
        assert_eq!(
            talk_over(&mut o, &mut now, 400, level(2, false, -10)),
            Some(310)
        );
        // Silent output never cuts: there is nothing to talk over.
        let mut o = output();
        let mut now = T0;
        assert!(!o.hear([level(2, false, -10)], now));
        for _ in 0..50 {
            now += TICK_MS;
            assert!(!o.hear([level(1, true, -10)], now));
        }
    }
}
