//! Room audio toward one voice. Each peer is decoded on its own; agents and peers the agent's
//! author gate doesn't admit are dropped before decode; speech comes from decoded PCM energy
//! (never the client-written `level_dbov`); and one human at a time is forwarded, resampled to
//! 24 kHz, with a send log.
//!
//! The caller owns the socket and the clock: it feeds relay frames and control messages in,
//! calls [`Input::tick`] every 10 ms with unix ms, and sends what comes out to Talos.

use crate::frames::{Frame, Role};
use buzz_audio_client::connection::{parse_peers, Peer};
use buzz_audio_client::jitter::PeerJitterBuffer;
use buzz_audio_client::wire::{audio_level_dbov, parse_relay_frame};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};

const TICK_MS: u64 = 10;
const IDLE_MS: u64 = 500;
const CHUNK: usize = 480; // 20 ms at 24 kHz.
const SENT_CAP: usize = 4_096;
const TAPS: usize = 31;

/// Who the voice may hear. The caller computes `admitted` with the agent's author gate and
/// `agents` from the room's `bot` roles, trusted 30177 records, Talos, and this agent itself.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub owner: String,
    pub agents: HashSet<String>,
    pub admitted: HashSet<String>,
}

impl Policy {
    fn allows(&self, pubkey: &str) -> bool {
        !self.agents.contains(pubkey) && (pubkey == self.owner || self.admitted.contains(pubkey))
    }
}

/// Calibration knobs; probe P measures the real values.
#[derive(Clone, Debug)]
pub struct Tuning {
    /// Decoded-PCM RMS level (dBov) at or above which a 10 ms frame is speech.
    pub speech_dbov: i8,
    /// Sustained speech before `onset`.
    pub onset_ms: u64,
    /// Silence before the forwarded human releases the floor.
    pub hangover_ms: u64,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            speech_dbov: -45,
            onset_ms: 100,
            hangover_ms: 300,
        }
    }
}

/// What the caller sends on to Talos.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // Only onset and speaker frames come out of here.
pub enum Out {
    Frame(Frame),
    /// PCM16 LE, 24 kHz mono, 20 ms.
    Pcm(Vec<u8>),
}

/// One send-log entry: this span's audio filled session ms `from_ms..to_ms`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub span: u64,
    pub pubkey: String,
    pub role: Role,
    pub from_ms: u64,
    pub to_ms: u64,
}

struct Floor {
    index: u8,
    span: u64,
}

struct Slot {
    key: (String, u8),
    jitter: PeerJitterBuffer,
    down: Halfband,
    last: Vec<i16>,
    last_packet_ms: u64,
    loud_ms: u64,
    quiet_ms: u64,
    talking: bool,
    since_ms: u64,
}

impl Slot {
    /// Advance speech state by one tick; true on onset.
    fn hear(&mut self, loud: bool, now: u64, t: &Tuning) -> bool {
        if loud {
            self.loud_ms += TICK_MS;
            self.quiet_ms = 0;
        } else {
            self.quiet_ms += TICK_MS;
            if !self.talking {
                self.loud_ms = 0;
            }
        }
        if !self.talking && self.loud_ms >= t.onset_ms {
            self.talking = true;
            self.since_ms = now;
            return true;
        }
        if self.talking && self.quiet_ms >= t.hangover_ms {
            self.talking = false;
            self.loud_ms = 0;
        }
        false
    }
}

#[derive(Default)]
pub struct Input {
    policy: Policy,
    tuning: Tuning,
    roster: HashMap<u8, (String, u8)>,
    slots: HashMap<u8, Slot>,
    floor: Option<Floor>,
    next_span: u64,
    awake: bool,
    session_ms: u64,
    pending: Vec<i16>,
    sent: VecDeque<Sent>,
}

impl Input {
    pub fn new(policy: Policy, tuning: Tuning) -> Self {
        Self {
            policy,
            tuning,
            ..Self::default()
        }
    }

    pub fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
        self.prune();
    }

    /// Replace the peer map from a relay snapshot (the initial `Joined::peers`).
    pub fn set_roster(&mut self, peers: &[Peer]) {
        self.roster = peers
            .iter()
            .map(|p| (p.index, (p.pubkey.clone(), p.epoch)))
            .collect();
        self.prune();
    }

    /// Apply a relay control message: `joined` and `roster` carry the full snapshot.
    pub fn on_control(&mut self, msg: &serde_json::Value) {
        match msg["type"].as_str() {
            Some("joined" | "roster") => match parse_peers(msg) {
                Ok(peers) => self.set_roster(&peers),
                Err(e) => tracing::warn!(error = %e, "huddle bridge: bad roster"),
            },
            Some("left") => {
                if let Some(index) = msg["peer_index"]
                    .as_u64()
                    .and_then(|i| u8::try_from(i).ok())
                {
                    self.roster.remove(&index);
                    self.prune();
                }
            }
            _ => {}
        }
    }

    /// Queue one relay audio frame. Unknown slots, agents and peers the gate doesn't admit
    /// are dropped before decode.
    pub fn on_audio(&mut self, data: &[u8], now_ms: u64) {
        let Some((index, header, opus)) = parse_relay_frame(data) else {
            return;
        };
        let Some(key) = self.roster.get(&index) else {
            return;
        };
        if !self.policy.allows(&key.0) {
            return;
        }
        let slot = match self.slots.entry(index) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let Ok(jitter) = PeerJitterBuffer::new(index) else {
                    return;
                };
                e.insert(Slot {
                    key: key.clone(),
                    jitter,
                    down: Halfband::new(),
                    last: Vec::new(),
                    last_packet_ms: now_ms,
                    loud_ms: 0,
                    quiet_ms: 0,
                    talking: false,
                    since_ms: 0,
                })
            }
        };
        if slot
            .jitter
            .insert_packet(header.seq, header.ts_48k, opus)
            .is_ok()
        {
            slot.last_packet_ms = now_ms;
        }
    }

    /// The voice's session opened (`awake`) or closed (`quiet`). Waking starts a new session
    /// clock and send log; while quiet only `onset` goes out.
    pub fn set_awake(&mut self, awake: bool) {
        if awake && !self.awake {
            self.session_ms = 0;
            self.sent.clear();
        }
        self.awake = awake;
        self.floor = None;
        self.pending.clear();
    }

    /// Send-log entries overlapping session ms `start_ms..end_ms`, for mapping a transcript
    /// delta back to a peer.
    pub fn who_said(&self, start_ms: u64, end_ms: u64) -> impl Iterator<Item = &Sent> {
        self.sent
            .iter()
            .filter(move |s| s.from_ms < end_ms && start_ms < s.to_ms)
    }

    pub fn roster_frame(&self) -> Frame {
        let mut keys: Vec<&String> = self.roster.values().map(|(pk, _)| pk).collect();
        keys.sort();
        keys.dedup();
        let (voices, humans): (Vec<&String>, Vec<&String>) = keys
            .into_iter()
            .partition(|pk| self.policy.agents.contains(*pk));
        Frame::Roster {
            humans: humans.into_iter().cloned().collect(),
            voices: voices.into_iter().cloned().collect(),
        }
    }

    /// Advance 10 ms.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Out> {
        let mut out = Vec::new();
        let t = self.tuning.clone();
        for slot in self.slots.values_mut() {
            slot.last.clear();
            let active =
                now_ms.saturating_sub(slot.last_packet_ms) < IDLE_MS || !slot.jitter.is_empty();
            let pcm = if active {
                slot.jitter.get_audio().ok().map(|(pcm, _)| pcm)
            } else {
                None
            };
            let loud = pcm
                .as_ref()
                .is_some_and(|pcm| audio_level_dbov(pcm) >= t.speech_dbov);
            if let Some(pcm) = pcm {
                slot.last = slot.down.process(&pcm);
            }
            if slot.hear(loud, now_ms, &t) {
                out.push(Out::Frame(Frame::Onset {
                    pubkey: slot.key.0.clone(),
                }));
            }
        }
        if !self.awake {
            return out;
        }
        if self
            .floor
            .as_ref()
            .is_some_and(|f| !self.slots.get(&f.index).is_some_and(|s| s.talking))
        {
            self.floor = None;
        }
        if self.floor.is_none() {
            let next = self
                .slots
                .iter()
                .filter(|(_, s)| s.talking)
                .min_by_key(|(i, s)| (s.since_ms, **i));
            if let Some((&index, slot)) = next {
                self.next_span += 1;
                let floor = Floor {
                    index,
                    span: self.next_span,
                };
                out.push(Out::Frame(Frame::Speaker {
                    span: floor.span,
                    pubkey: slot.key.0.clone(),
                    role: self.role(&slot.key.0),
                    echo: false,
                    t_ms: self.session_ms,
                }));
                self.floor = Some(floor);
            }
        }
        // ponytail: the onset's first `onset_ms` isn't forwarded; the ring slice sends it.
        let held = self.floor.as_ref().map(|f| (f.span, &self.slots[&f.index]));
        let audio = match held {
            Some((_, slot)) if !slot.last.is_empty() => slot.last.clone(),
            _ => vec![0; CHUNK / 2],
        };
        let from = self.session_ms;
        self.session_ms += audio.len() as u64 * 1000 / 24_000;
        if let Some((span, slot)) = held {
            let pubkey = slot.key.0.clone();
            self.log_sent(Sent {
                span,
                role: self.role(&pubkey),
                pubkey,
                from_ms: from,
                to_ms: self.session_ms,
            });
        }
        self.pending.extend(audio);
        while self.pending.len() >= CHUNK {
            out.push(Out::Pcm(
                self.pending
                    .drain(..CHUNK)
                    .flat_map(|s| s.to_le_bytes())
                    .collect(),
            ));
        }
        out
    }

    fn role(&self, pubkey: &str) -> Role {
        if pubkey == self.policy.owner {
            Role::Owner
        } else {
            Role::Admitted
        }
    }

    fn log_sent(&mut self, sent: Sent) {
        if let Some(last) = self.sent.back_mut() {
            if last.span == sent.span && last.to_ms == sent.from_ms {
                last.to_ms = sent.to_ms;
                return;
            }
        }
        if self.sent.len() == SENT_CAP {
            self.sent.pop_front();
        }
        self.sent.push_back(sent);
    }

    /// Drop slots whose occupant changed or is no longer admitted, and a floor without its slot.
    fn prune(&mut self) {
        self.slots
            .retain(|i, s| self.roster.get(i) == Some(&s.key) && self.policy.allows(&s.key.0));
        if self
            .floor
            .as_ref()
            .is_some_and(|f| !self.slots.contains_key(&f.index))
        {
            self.floor = None;
        }
    }
}

/// 48 to 24 kHz: a 31-tap Hamming-windowed half-band low-pass, then every second sample.
struct Halfband {
    taps: [f32; TAPS],
    hist: Vec<f32>,
}

impl Halfband {
    fn new() -> Self {
        let mid = (TAPS - 1) as f32 / 2.0;
        let mut taps = [0.0; TAPS];
        for (i, tap) in taps.iter_mut().enumerate() {
            let n = i as f32 - mid;
            let sinc = if n == 0.0 {
                0.5
            } else {
                (std::f32::consts::FRAC_PI_2 * n).sin() / (std::f32::consts::PI * n)
            };
            *tap =
                sinc * (0.54 - 0.46 * (std::f32::consts::TAU * i as f32 / (TAPS - 1) as f32).cos());
        }
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|tap| *tap /= sum);
        Self {
            taps,
            hist: vec![0.0; TAPS - 1],
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<i16> {
        self.hist.extend_from_slice(input);
        let n = (self.hist.len() - (TAPS - 1)) / 2;
        let out = (0..n)
            .map(|i| {
                let s: f32 = self.hist[2 * i..2 * i + TAPS]
                    .iter()
                    .zip(&self.taps)
                    .map(|(a, b)| a * b)
                    .sum();
                (s.clamp(-1.0, 1.0) * 32_767.0) as i16
            })
            .collect();
        self.hist.drain(..2 * n);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_audio_client::encoder::AudioEncoder;
    use std::f32::consts::TAU;

    const OWNER: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const GUEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const AGENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const STRANGER: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const T0: u64 = 1_793_318_400_000;

    fn input() -> Input {
        // The agent is admitted too, as an author gate open to anyone would: the agent set wins.
        let admitted = [GUEST.into(), AGENT.into()].into();
        let policy = Policy {
            owner: OWNER.into(),
            agents: [AGENT.into()].into(),
            admitted,
        };
        let mut input = Input::new(policy, Tuning::default());
        let peers = [(1, OWNER), (2, GUEST), (3, AGENT), (4, STRANGER)];
        input.set_roster(&peers.map(|(index, pk)| Peer {
            index,
            pubkey: pk.into(),
            epoch: 0,
        }));
        input
    }

    /// A peer sending 20 ms of a 1 kHz tone; `level` overwrites the client-written level byte.
    struct Talker(u8, AudioEncoder, f32, Option<i8>);

    fn talker(index: u8, amp: f32, level: Option<i8>) -> Talker {
        Talker(index, AudioEncoder::new().unwrap(), amp, level)
    }

    fn run(input: &mut Input, now: &mut u64, ms: u64, talkers: &mut [Talker]) -> Vec<Out> {
        let mut out = Vec::new();
        for _ in 0..ms / TICK_MS {
            for Talker(index, enc, amp, level) in
                talkers.iter_mut().filter(|_| now.is_multiple_of(20))
            {
                let pcm: Vec<f32> = (0..960)
                    .map(|i| *amp * (TAU * 1000.0 * i as f32 / 48_000.0).sin())
                    .collect();
                let mut frame = vec![*index];
                frame.extend(enc.encode(&pcm).unwrap());
                if let Some(level) = level {
                    frame[7] = *level as u8;
                }
                input.on_audio(&frame, *now);
            }
            out.extend(input.tick(*now));
            *now += TICK_MS;
        }
        out
    }

    fn frames(out: &[Out]) -> Vec<&Frame> {
        out.iter()
            .filter_map(|o| if let Out::Frame(f) = o { Some(f) } else { None })
            .collect()
    }

    fn voiced(out: &[Out]) -> bool {
        let loud = |b: &[u8]| {
            b.chunks(2)
                .any(|s| i16::from_le_bytes([s[0], s[1]]).abs() > 1000)
        };
        out.iter().any(|o| matches!(o, Out::Pcm(b) if loud(b)))
    }

    #[test]
    fn speech_comes_from_decoded_pcm_never_the_level_byte() {
        let mut input = input();
        input.set_awake(true);
        let mut now = T0;
        let mut owner = [talker(1, 0.0, Some(0))];
        let out = run(&mut input, &mut now, 400, &mut owner);
        assert!(
            frames(&out).is_empty() && !voiced(&out),
            "silence claiming full level"
        );
        owner[0].2 = 0.3;
        owner[0].3 = Some(-127);
        let out = run(&mut input, &mut now, 400, &mut owner);
        assert_eq!(
            frames(&out)[0],
            &Frame::Onset {
                pubkey: OWNER.into()
            }
        );
        assert!(matches!(
            frames(&out)[1],
            Frame::Speaker {
                span: 1,
                role: Role::Owner,
                echo: false,
                ..
            }
        ));
        assert!(voiced(&out), "decoded, resampled and forwarded");
        assert!(out
            .iter()
            .all(|o| !matches!(o, Out::Pcm(b) if b.len() != 2 * CHUNK)));
        assert!(input.who_said(0, u64::MAX).all(|s| s.pubkey == OWNER));
    }

    #[test]
    fn agents_and_peers_the_gate_refuses_are_never_heard() {
        let mut input = input();
        input.set_awake(true);
        let mut now = T0;
        let mut others = [
            talker(3, 0.3, None),
            talker(4, 0.3, None),
            talker(9, 0.3, None),
        ];
        let out = run(&mut input, &mut now, 400, &mut others);
        assert!(frames(&out).is_empty() && !voiced(&out));
        // A new occupant of slot 4 that the gate admits is heard; the roster follows the relay.
        let peers = serde_json::json!([{"pubkey": OWNER, "peer_index": 1}, {"pubkey": AGENT, "peer_index": 3},
            {"pubkey": GUEST, "peer_index": 4, "epoch": 1}]);
        input.on_control(&serde_json::json!({"type": "joined", "peers": peers}));
        let out = run(&mut input, &mut now, 400, &mut others);
        assert_eq!(
            frames(&out)[0],
            &Frame::Onset {
                pubkey: GUEST.into()
            }
        );
        input.on_control(&serde_json::json!({"type": "left", "peer_index": 4}));
        let roster = Frame::Roster {
            humans: vec![OWNER.into()],
            voices: vec![AGENT.into()],
        };
        assert_eq!(input.roster_frame(), roster);
    }

    #[test]
    fn one_human_at_a_time_and_the_send_log_names_them() {
        let mut input = input();
        input.set_awake(true);
        let mut now = T0;
        let mut ts = [talker(1, 0.3, None), talker(2, 0.0, None)];
        run(&mut input, &mut now, 300, &mut ts);
        ts[1].2 = 0.3;
        let both = run(&mut input, &mut now, 500, &mut ts);
        ts[0].2 = 0.0;
        let after = run(&mut input, &mut now, 800, &mut ts);
        assert_eq!(
            frames(&both),
            [&Frame::Onset {
                pubkey: GUEST.into()
            }],
            "the guest waits"
        );
        assert!(matches!(
            frames(&after)[..],
            [Frame::Speaker {
                span: 2,
                role: Role::Admitted,
                ..
            }]
        ));
        let log: Vec<_> = input
            .who_said(0, u64::MAX)
            .map(|s| (s.span, s.pubkey.as_str()))
            .collect();
        assert_eq!(log, [(1, OWNER), (2, GUEST)]);
        let owner_end = input.who_said(0, u64::MAX).next().unwrap().to_ms;
        assert_eq!(
            input
                .who_said(owner_end, owner_end + 10)
                .next()
                .unwrap()
                .pubkey,
            GUEST
        );
    }

    #[test]
    fn halfband_keeps_speech_and_stops_aliases() {
        let gain = |hz: f32| {
            let tone: Vec<f32> = (0..4_800)
                .map(|i| 0.5 * (TAU * hz * i as f32 / 48_000.0).sin())
                .collect();
            let out = Halfband::new().process(&tone);
            assert_eq!(out.len(), 2_400);
            out[100..].iter().map(|s| s.unsigned_abs()).max().unwrap() as f32 / (0.5 * 32_767.0)
        };
        assert!((gain(1_000.0) - 1.0).abs() < 0.05);
        assert!(gain(20_000.0) < 0.01);
    }
}
