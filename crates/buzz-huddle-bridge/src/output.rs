//! The voice's audio toward the room, moved from the desktop's agent publisher
//! (`desktop/src-tauri/src/huddle/relay_api.rs`): 24 kHz PCM16 from Talos is upsampled to
//! 48 kHz and cut into 20 ms frames, at most [`QUEUE_FRAMES`] queued, and one frame is encoded
//! per 20 ms tick. It plays only while this voice holds the floor (the speak token), and a
//! human who talks over it cuts it (barge-in). The cut takes the place of the desktop's epoch:
//! the queue goes, and the rest of that response is dropped as it arrives, until Talos has sent
//! nothing for `release_ms` or gives the floor again with a newer token.
//!
//! The caller owns the socket and the clock, as for [`crate::input::Input`]: it calls
//! [`Output::hear`] every 10 ms and [`Output::tick`] every 20 ms.

use crate::input::Level;
use buzz_audio_client::encoder::{AudioEncoder, FRAME_SAMPLES};
use buzz_audio_client::wire::audio_level_dbov;
use std::collections::{HashMap, VecDeque};

/// 16 frames, 320 ms: the most a barge-in ever has to cut. Talos paces its PCM to real time
/// (frames v1); a burst past this is dropped, with one warning per response.
pub const QUEUE_FRAMES: usize = 16;
const FRAME_MS: u64 = 20;
const TICK_MS: u64 = 10;

/// Barge-in knobs; probe P measures the real values.
#[derive(Clone, Debug)]
pub struct Barge {
    /// Decoded level (dBov) at or above which a human's 10 ms counts as energy. Keep it equal to
    /// [`crate::input::Tuning::speech_dbov`], so a cut and Input's onset agree.
    pub speech_dbov: i8,
    /// Energy that cuts the voice from the owner's isolated desktop.
    pub isolated_ms: u64,
    /// Energy above the echo threshold that cuts it from any other peer.
    pub sustained_ms: u64,
    /// A dip this short, between syllables or while the queue runs dry for a moment, doesn't
    /// end a run of energy.
    pub gap_ms: u64,
    /// The echo threshold is the loudest frame sent within `echo_ms` plus this, since an echo
    /// of any of them may still be coming back. Negative means the expected loss from speaker
    /// to mic. ponytail: +6 dB lets only a loud talker through; probe P measures the coupling.
    pub echo_margin_db: i8,
    /// How long a frame's echo may still come back, so how long its level counts.
    pub echo_ms: u64,
    /// After a cut, how long Talos must send nothing (the cut response is over) before the
    /// voice may play again.
    pub release_ms: u64,
}

impl Default for Barge {
    fn default() -> Self {
        Self {
            speech_dbov: -45,
            isolated_ms: 200,
            sustained_ms: 320,
            gap_ms: 80,
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
    /// The partial frame waited one tick for more PCM and goes out at the next.
    tail_due: bool,
    /// A frame was dropped past the cap since the queue last ran empty.
    overflow: bool,
    token: u64,
    floor: bool,
    /// Unix ms and level of each frame sent within `echo_ms`.
    sent: VecDeque<(u64, i8)>,
    /// When the last frame sent ends.
    last_end_ms: u64,
    /// Each peer's ms of energy while the voice plays, and the ms since its last energy.
    runs: HashMap<u8, (u64, u64)>,
    /// Ms since Talos last sent PCM, while cut; `None` when not cut.
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
            tail_due: false,
            overflow: false,
            token: 0,
            floor: false,
            sent: VecDeque::new(),
            last_end_ms: 0,
            runs: HashMap::new(),
            cut: None,
        })
    }

    /// `floor {holder, token}` from Talos. A token older than the last is ignored, and a newer
    /// one ends a cut, since it is a new turn. Without the floor nothing queues and what was
    /// queued goes.
    pub fn set_floor(&mut self, holder: Option<&str>, token: u64) {
        if token < self.token {
            return;
        }
        if token > self.token {
            self.cut = None;
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
        if !self.floor {
            return;
        }
        if let Some(quiet) = &mut self.cut {
            *quiet = 0;
            return;
        }
        self.tail_due = false;
        for s in pcm.as_chunks::<2>().0 {
            let s = f32::from(i16::from_le_bytes(*s)) / 32_768.0;
            // Linear 24 to 48 kHz that runs on across chunks: the midpoint, then the sample.
            self.partial.extend([(self.prev + s) * 0.5, s]);
            self.prev = s;
            if self.partial.len() == FRAME_SAMPLES {
                let frame = std::mem::take(&mut self.partial);
                if self.queue.len() < QUEUE_FRAMES {
                    self.queue.push_back(frame);
                } else if !std::mem::replace(&mut self.overflow, true) {
                    tracing::warn!(
                        "huddle bridge: voice PCM ahead of real time, dropped past 320 ms"
                    );
                }
            }
        }
    }

    /// One 20 ms tick at unix `now_ms`: the next frame for the relay, or nothing. A short tail
    /// waits one tick for the PCM that may finish it, so a chunk just in time pads no silence
    /// into speech, then goes out padded by the encoder.
    pub fn tick(&mut self, now_ms: u64) -> Option<Vec<u8>> {
        let frame = if let Some(frame) = self.queue.pop_front() {
            frame
        } else if self.partial.is_empty() || !std::mem::replace(&mut self.tail_due, true) {
            self.overflow = false;
            return None;
        } else {
            std::mem::take(&mut self.partial)
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

    /// When the voice's output ends, queued audio included, for the half-duplex window.
    pub fn until(&self) -> u64 {
        let frames = self.queue.len() + usize::from(!self.partial.is_empty());
        self.last_end_ms + FRAME_MS * frames as u64
    }

    /// Barge-in, every 10 ms, with each admitted human's level. While the voice plays, a human
    /// on the owner's isolated desktop with `isolated_ms` of energy, or any other with
    /// `sustained_ms` above the echo threshold, cuts it: the queue goes, and the rest of the
    /// response is dropped (see [`Output::set_floor`] and [`Barge::release_ms`]). True on a cut.
    ///
    /// ponytail: on speakers the run that cuts falls in Input's echo window, so GPT-Live never
    /// hears it; a short interjection is lost. Upgrade (probe P / slice 15): keep echo-flagged
    /// audio in the ring and replay that run with its flag, or a frames `barge` that truncates.
    pub fn hear(&mut self, levels: impl IntoIterator<Item = Level>, now_ms: u64) -> bool {
        let b = &self.barge;
        while self
            .sent
            .front()
            .is_some_and(|(at, _)| at + b.echo_ms <= now_ms)
        {
            self.sent.pop_front();
        }
        if let Some(quiet) = self.cut {
            let quiet = quiet + TICK_MS;
            self.cut = (quiet < b.release_ms).then_some(quiet);
            if self.cut.is_none() {
                tracing::info!("huddle bridge: the cut response is over");
            }
            return false;
        }
        let recent = self.sent.iter().map(|(_, l)| *l).max().unwrap_or(-127);
        let echo = recent.saturating_add(b.echo_margin_db).max(b.speech_dbov);
        let playing = now_ms < self.until();
        let mut barged = None;
        let runs = levels
            .into_iter()
            .map(|l| {
                let (need, floor) = if l.isolated {
                    (b.isolated_ms, b.speech_dbov)
                } else {
                    (b.sustained_ms, echo)
                };
                let (mut energy, mut gap) = self.runs.get(&l.index).copied().unwrap_or_default();
                if playing && l.dbov >= floor {
                    (energy, gap) = (energy + TICK_MS, 0);
                } else {
                    gap += TICK_MS;
                    if gap > b.gap_ms {
                        energy = 0;
                    }
                }
                if energy >= need {
                    barged.get_or_insert(l.index);
                }
                (l.index, (energy, gap))
            })
            .collect();
        self.runs = runs;
        let Some(index) = barged else {
            return false;
        };
        tracing::info!(index, "huddle bridge: barge-in cut the voice");
        self.flush();
        self.runs.clear();
        self.cut = Some(0);
        true
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

    /// Talos's response at about -23 dBov, in `chunk` ms chunks each due as the last runs out,
    /// until `end` ms in.
    fn talos(chunk: u64, end: u64) -> impl Fn(u64) -> Option<Vec<u8>> {
        move |t| (t < end && t % chunk == 0).then(|| pcm(chunk as usize, 0.1))
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

    /// Step `ms` from `now` in link.rs's order: Talos's PCM due `t` ms in, barge-in with one
    /// human, then a frame every 20 ms. The frames sent, and how far in a cut came.
    fn play(
        o: &mut Output,
        now: &mut u64,
        ms: u64,
        talos: impl Fn(u64) -> Option<Vec<u8>>,
        human: impl Fn(u64) -> Level,
    ) -> (usize, Option<u64>) {
        let start = *now;
        let (mut sent, mut cut) = (0, None);
        while *now < start + ms {
            let t = *now - start;
            if let Some(pcm) = talos(t) {
                o.on_pcm(&pcm);
            }
            if o.hear([human(t)], *now) {
                cut.get_or_insert(t);
            }
            if now.is_multiple_of(FRAME_MS) {
                sent += usize::from(o.tick(*now).is_some());
            }
            *now += TICK_MS;
        }
        (sent, cut)
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
        let mut sent = 0;
        for i in 0..20 {
            sent += usize::from(o.tick(T0 + i * FRAME_MS).is_some());
        }
        assert_eq!(sent, QUEUE_FRAMES);
        assert_eq!(o.until(), T0 + 16 * FRAME_MS);
        // A short tail counts toward the end, waits a tick for more, then goes out.
        o.on_pcm(&pcm(5, 0.3));
        assert_eq!(o.until(), T0 + 17 * FRAME_MS);
        assert!(o.tick(T0 + 400).is_none() && o.tick(T0 + 420).is_some());
        assert!(o.tick(T0 + 440).is_none());
    }

    #[test]
    fn just_in_time_chunks_pad_no_silence_and_a_dry_queue_keeps_the_run() {
        let mut o = output();
        let mut now = T0;
        // 1 s in 50 ms chunks: 50 frames, no padded tail between chunks.
        let quiet = |_| level(2, false, -127);
        assert_eq!(play(&mut o, &mut now, 1_200, talos(50, 1_000), quiet).0, 50);
        // The queue holds under a frame now and then; a talker on speakers still cuts at 320 ms.
        let loud = |_| level(2, false, -10);
        let (_, cut) = play(&mut o, &mut now, 1_000, talos(50, 1_000), loud);
        assert_eq!(cut, Some(320));
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
    fn barge_in_takes_200_ms_isolated_or_320_ms_over_the_echo_and_outlasts_dips() {
        // Speech with a 20 ms dip every 200 ms; the voice plays from the first tick.
        let dipping = |index, isolated, dbov| {
            move |t: u64| level(index, isolated, if t % 200 >= 180 { -60 } else { dbov })
        };
        // On headphones the voice's level doesn't matter; on speakers it plays at about
        // -23 dBov, so its echo threshold is about -17 dBov.
        for (human, cut) in [(dipping(1, true, -40), 220), (dipping(2, false, -10), 340)] {
            let mut o = output();
            let mut now = T0;
            let (_, at) = play(&mut o, &mut now, 1_000, talos(20, 1_000), human);
            assert_eq!(at, Some(cut));
            assert!(o.queue.is_empty() && o.until() <= now, "flushed");
        }
        let mut o = output();
        let mut now = T0;
        let echo = |_| level(2, false, -20);
        let (sent, cut) = play(&mut o, &mut now, 2_000, talos(20, 2_000), echo);
        assert_eq!((sent, cut), (100, None), "its own echo never cuts it");
        // Silent output never cuts: there is nothing to talk over.
        let loud = |_| level(1, true, -10);
        assert_eq!(play(&mut o, &mut now, 1_000, |_| None, loud).1, None);
    }

    #[test]
    fn a_cut_drops_the_rest_of_the_response_until_talos_stops_or_turns_the_floor() {
        let mut o = output();
        let mut now = T0;
        let owner = |_| level(1, true, -30);
        let voice = || talos(20, u64::MAX);
        assert_eq!(play(&mut o, &mut now, 400, voice(), owner).1, Some(200));
        // The response goes on, the room is quiet or noisy: none of it plays.
        let noise = |_| level(2, false, -44);
        assert_eq!(play(&mut o, &mut now, 3_000, voice(), noise).0, 0);
        // Talos stops: 300 ms later the cut is over, whoever is still talking.
        play(&mut o, &mut now, 260, |_| None, owner);
        assert!(o.cut.is_some());
        play(&mut o, &mut now, 40, |_| None, owner);
        assert!(o.cut.is_none());
        // The next response plays, and the owner's run before the cut doesn't count.
        let done = |_| level(1, true, -127);
        assert!(play(&mut o, &mut now, 100, voice(), done).0 > 0);
        // A newer floor token is a new turn, and ends a cut at once.
        play(&mut o, &mut now, 400, voice(), owner);
        o.set_floor(Some(ME), 1);
        assert!(o.cut.is_some(), "the same token");
        o.set_floor(Some(ME), 2);
        assert!(o.cut.is_none());
    }

    /// Input and Output together in link.rs's step order, with a decoded talker: `loud(t)` says
    /// whether the talker's mic carries speech `t` ms in, and `voice(t)` whether Talos sends.
    /// How far in the cut came, and the first frame sent after it.
    fn room(
        isolated: bool,
        loud: impl Fn(u64) -> bool,
        voice: impl Fn(u64) -> bool,
    ) -> (Option<u64>, Option<u64>) {
        use crate::input::{Input, Policy, Route, Tuning};
        use buzz_audio_client::connection::Peer;
        let (owner, guest) = ("0".repeat(63) + "1", "b".repeat(64));
        let policy = Policy {
            owner: owner.clone(),
            agents: [ME.into()].into(),
            admitted: [guest.clone()].into(),
        };
        let mut input = Input::new(policy, Tuning::default());
        let peers = [(1, owner), (2, guest)].map(|(index, pubkey)| Peer {
            index,
            pubkey,
            epoch: 0,
        });
        input.set_roster(&peers);
        input.set_route(Route {
            here: true,
            isolated: true,
        });
        let index = if isolated { 1 } else { 2 };
        let mut mic = AudioEncoder::new().unwrap();
        let mut o = output();
        let (mut cut, mut resumed) = (None, None);
        for t in (0..4_000).step_by(10) {
            let now = T0 + t;
            if t % 20 == 0 {
                let amp = if loud(t) { 0.3 } else { 0.0 };
                let tone = (0..FRAME_SAMPLES)
                    .map(|i| amp * (std::f32::consts::TAU * 1_000.0 * i as f32 / 48_000.0).sin());
                let mut frame = vec![index];
                frame.extend(mic.encode(&tone.collect::<Vec<_>>()).unwrap());
                input.on_audio(&frame, now);
                if voice(t) {
                    o.on_pcm(&pcm(20, 0.1));
                }
            }
            input.tick(now);
            if o.hear(input.levels(), now) {
                cut.get_or_insert(t);
            }
            let sent = t % 20 == 0 && o.tick(now).is_some();
            if sent && cut.is_some() {
                resumed.get_or_insert(t);
            }
            input.set_output_until(o.until());
        }
        (cut, resumed)
    }

    #[test]
    fn through_input_a_dipping_talker_cuts_and_the_cut_response_never_resumes() {
        // 1.5 s of speech from 1 s in, off for 20 ms every 200 ms, over a response that runs on.
        let speech = |t: u64| (1_000..2_500).contains(&t) && t % 200 < 180;
        for isolated in [true, false] {
            let (cut, resumed) = room(isolated, speech, |_| true);
            assert!(cut.is_some_and(|c| (1_000..2_000).contains(&c)), "{cut:?}");
            assert_eq!(resumed, None, "isolated: {isolated}");
        }
        // A 500 ms interjection on speakers: the rest of that response is dropped, and the
        // next one, from 3.5 s, plays.
        let (cut, resumed) = room(
            false,
            |t| (1_000..1_500).contains(&t),
            |t| !(3_000..3_500).contains(&t),
        );
        assert!(cut.is_some());
        assert!(resumed.is_some_and(|r| r >= 3_500), "{resumed:?}");
    }
}
