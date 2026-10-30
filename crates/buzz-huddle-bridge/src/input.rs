//! Room audio toward one voice. Each peer is decoded on its own; agents and peers the agent's
//! author gate doesn't admit are dropped before decode; speech comes from decoded PCM energy
//! (never the client-written `level_dbov`); one human at a time is forwarded, resampled to
//! 24 kHz, with a send log; each peer keeps an 8 s ring, bounded by age, that never holds a
//! secret window or echo; and on a non-isolated route the voice doesn't hear a peer while its
//! own output plays (half-duplex).
//!
//! The caller owns the socket and the clock: it feeds relay frames and control messages in,
//! calls [`Input::tick`] every 10 ms, and sends what comes out to Talos. Every `now_ms` is unix
//! ms on a monotonic clock (a start time plus `Instant` elapsed), so no window or age jumps.

use crate::frames::{Role, ToTalos};
use buzz_audio_client::connection::{parse_peers, Peer};
use buzz_audio_client::jitter::PeerJitterBuffer;
use buzz_audio_client::wire::{audio_level_dbov, parse_relay_frame};
use serde::Deserialize;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::Path;

const TICK_MS: u64 = 10;
const RING_MS: u64 = 8_000;
const IDLE_MS: u64 = 500;
const CHUNK: usize = 480; // 20 ms at 24 kHz.
const SENT_CAP: usize = 4_096;
const TAPS: usize = 31;
/// A tick gap past this is a stall: audio queued through it is dropped, never played late.
pub(crate) const STALL_MS: u64 = 5 * TICK_MS;
/// How long an index that changed hands is held back. v2 frames carry no occupancy epoch, so a
/// delayed frame of the old occupant would otherwise be heard as the new one.
const QUARANTINE_MS: u64 = 500;

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
    /// Sustained speech before `onset`; this much audio is also sent ahead of the span.
    pub onset_ms: u64,
    /// Silence before the forwarded human releases the floor.
    pub hangover_ms: u64,
    /// Echo tail after the voice's output, for the owner's desktop. ponytail: both tails are
    /// guesses until probe P measures them from `output_until`, the desktop's and the phone's.
    pub desktop_tail_ms: u64,
    /// Echo tail for every other peer.
    pub remote_tail_ms: u64,
    /// Widening on both sides of each `secret` window, for clock skew with Talos.
    pub secret_skew_ms: u64,
    /// A talker this long without a pause yields the floor to any fresh talker, so steady
    /// noise can't hold it. ponytail: a per-peer noise floor is the upgrade, after probe P.
    pub floor_cap_ms: u64,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            speech_dbov: -45,
            onset_ms: 100,
            hangover_ms: 300,
            desktop_tail_ms: 300,
            remote_tail_ms: 600,
            secret_skew_ms: 1_000,
            floor_cap_ms: 30_000,
        }
    }
}

/// The desktop's output route as it applies to one huddle: `here` when the desktop's audio is
/// in it, `isolated` when that output is headphones.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Route {
    pub here: bool,
    pub isolated: bool,
}

/// Read the desktop's route file, `{isolated, huddle, updated_at}` in unix s, for `huddle` at
/// `now_ms`. The desktop rewrites it every 5 s while in a huddle and deletes it on leaving. A
/// missing or unreadable file, one for another huddle, or one 3 heartbeats stale means not here
/// and not isolated. Re-read it every few seconds: going stale changes nothing on disk.
pub fn read_route(path: &Path, huddle: &str, now_ms: u64) -> Route {
    #[derive(Deserialize)]
    struct File {
        isolated: bool,
        huddle: String,
        updated_at: u64,
    }
    let file: Option<File> = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let fresh = |f: &File| f.updated_at.saturating_mul(1000).abs_diff(now_ms) <= 15_000;
    file.filter(|f| f.huddle == huddle && fresh(f))
        .map_or_else(Route::default, |f| Route {
            here: true,
            isolated: f.isolated,
        })
}

/// What the caller sends on to Talos.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // Only onset and speaker frames come out of here.
pub enum Out {
    Frame(ToTalos),
    /// PCM16 LE, 24 kHz mono, 20 ms.
    Pcm(Vec<u8>),
}

/// One admitted human's decoded level this tick, for barge-in. `isolated` marks the owner's
/// desktop on headphones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub index: u8,
    pub isolated: bool,
    pub dbov: i8,
}

/// One send-log entry: this span's audio filled session ms `from_ms..to_ms`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub span: u64,
    pub pubkey: String,
    pub role: Role,
    pub echo: bool,
    pub from_ms: u64,
    pub to_ms: u64,
}

struct Floor {
    index: u8,
    span: u64,
    echo: bool,
}

struct Slot {
    key: (String, u8),
    jitter: PeerJitterBuffer,
    down: Halfband,
    ring: VecDeque<(u64, Vec<i16>)>,
    last: Vec<i16>,
    last_packet_ms: u64,
    loud_ms: u64,
    quiet_ms: u64,
    talking: bool,
    since_ms: u64,
    echo: bool,
    /// An onset not yet sent because it fell in the echo.
    onset: bool,
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
    route: Route,
    roster: HashMap<u8, (String, u8)>,
    slots: HashMap<u8, Slot>,
    floor: Option<Floor>,
    next_span: u64,
    awake: bool,
    session_ms: u64,
    pending: Vec<i16>,
    sent: VecDeque<Sent>,
    output_until_ms: u64,
    secret: Vec<Range<u64>>,
    last_tick_ms: Option<u64>,
    levels: Vec<Level>,
    /// Index to the unix ms its frames are heard again.
    quarantine: HashMap<u8, u64>,
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

    pub fn set_route(&mut self, route: Route) {
        self.route = route;
    }

    /// Replace the peer map from a relay snapshot (the initial `Joined::peers`).
    pub fn set_roster(&mut self, peers: &[Peer]) {
        self.roster = peers.iter().map(|p| (p.index, key(p))).collect();
        self.prune();
    }

    /// Rebuild the peer map from a full snapshot after a gap: a new socket's after a reconnect,
    /// or the relay's `roster` resync after missed deltas. An index whose occupant changed, or
    /// is new, is not heard for 500 ms. One the snapshot no longer lists is a `left` missed in
    /// the gap, so a `joined` reusing it within 500 ms is held back as after a `left`.
    pub fn rejoin(&mut self, peers: &[Peer], now_ms: u64) {
        for &i in self.roster.keys() {
            if !peers.iter().any(|p| p.index == i) {
                self.quarantine.insert(i, now_ms + QUARANTINE_MS);
            }
        }
        for p in peers {
            if self.roster.get(&p.index) != Some(&key(p)) {
                self.quarantine.insert(p.index, now_ms + QUARANTINE_MS);
            }
        }
        self.set_roster(peers);
    }

    /// Add or replace peers; a slot whose occupant changed loses its state. An index that
    /// changed hands, or that `left` or a snapshot freed under 500 ms ago, is not heard for
    /// 500 ms from `now_ms`; one never used, or whose occupant stayed, is heard at once.
    fn add_peers(&mut self, peers: &[Peer], now_ms: u64) {
        for p in peers {
            let changed = match self.roster.insert(p.index, key(p)) {
                Some(old) => old != key(p),
                None => self.held(p.index, now_ms),
            };
            if changed {
                self.quarantine.insert(p.index, now_ms + QUARANTINE_MS);
            }
        }
        self.prune();
    }

    fn held(&self, index: u8, now_ms: u64) -> bool {
        self.quarantine
            .get(&index)
            .is_some_and(|&until| now_ms < until)
    }

    /// Apply a relay control message. `roster` is the full snapshot; `joined` adds its peers
    /// (all of them locally, only the newcomer from another pod); `left` drops one, and a
    /// `joined` reusing its index within 500 ms is held back. `now_ms` is the clock `on_audio`
    /// checks the quarantine against.
    pub fn on_control(&mut self, msg: &serde_json::Value, now_ms: u64) {
        match msg["type"].as_str() {
            Some(kind @ ("joined" | "roster")) => match parse_peers(msg) {
                Ok(peers) if kind == "roster" => self.rejoin(&peers, now_ms),
                Ok(peers) => self.add_peers(&peers, now_ms),
                Err(e) => tracing::warn!(error = %e, "huddle bridge: bad roster"),
            },
            Some("left") => {
                if let Some(index) = msg["peer_index"]
                    .as_u64()
                    .and_then(|i| u8::try_from(i).ok())
                {
                    self.roster.remove(&index);
                    self.quarantine.insert(index, now_ms + QUARANTINE_MS);
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
        if self.held(index, now_ms) {
            return;
        }
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
                    ring: VecDeque::new(),
                    last: Vec::new(),
                    last_packet_ms: now_ms,
                    loud_ms: 0,
                    quiet_ms: 0,
                    talking: false,
                    since_ms: 0,
                    echo: false,
                    onset: false,
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
        // A repeated `awake` keeps the clock, the floor and the audio already counted.
        if awake == self.awake {
            return;
        }
        if awake {
            self.session_ms = 0;
            self.sent.clear();
        }
        self.awake = awake;
        self.floor = None;
        self.pending.clear();
    }

    /// The voice's own output, sent and queued, plays until `unix_ms`. The output path sets it
    /// every step, so a flush (a cut, a lost floor) ends the half-duplex window there, plus its
    /// tail.
    pub fn set_output_until(&mut self, unix_ms: u64) {
        self.output_until_ms = unix_ms;
    }

    /// Each admitted human's level at the last tick.
    pub fn levels(&self) -> impl Iterator<Item = Level> + '_ {
        self.levels.iter().copied()
    }

    /// Talos marked unix ms `from_ms..to_ms`, on its clock, secret; it arrived at `now_ms`. The
    /// window is widened by the skew margin on both sides and starts by `now_ms` at the latest.
    /// Its audio leaves every ring, nothing arriving inside it is recorded or forwarded, and what
    /// is still queued at its end is dropped. ponytail: judged by arrival, so a packet the network
    /// holds past the window still plays; the RTP timestamp is the upgrade.
    pub fn add_secret(&mut self, from_ms: u64, to_ms: u64, now_ms: u64) {
        let skew = self.tuning.secret_skew_ms;
        let window =
            from_ms.saturating_sub(skew).min(now_ms)..to_ms.max(from_ms).saturating_add(skew);
        for slot in self.slots.values_mut() {
            slot.ring.retain(|(t, _)| !window.contains(t));
        }
        // Up to 10 ms not yet sent: silence it, keeping the session clock.
        self.pending.fill(0);
        if window.end > now_ms {
            self.secret.push(window);
        }
    }

    /// Answer `replay_req` with one peer's ring audio in unix ms `from_ms..to_ms`: `replay`, the
    /// PCM, `replay_end`, all sent before the next tick's output. It counts on the session clock
    /// and in the send log like live audio. Nothing goes out while the voice is quiet.
    pub fn replay(&mut self, index: u8, from_ms: u64, to_ms: u64, now_ms: u64) -> Vec<Out> {
        let mut out = Vec::new();
        if !self.awake {
            return out;
        }
        // Live audio already counted goes first, padded to a whole chunk.
        let pad = (CHUNK - self.pending.len() % CHUNK) % CHUNK;
        self.pending.resize(self.pending.len() + pad, 0);
        self.session_ms += pad as u64 * 1000 / 24_000;
        self.send(&mut out);
        self.next_span += 1;
        let span = self.next_span;
        out.push(Out::Frame(ToTalos::Replay {
            span,
            from_ms,
            to_ms,
        }));
        let mut pcm = self.ring_pcm(index, from_ms, to_ms, now_ms);
        pcm.resize(pcm.len().next_multiple_of(CHUNK), 0);
        let from = self.session_ms;
        self.session_ms += pcm.len() as u64 * 1000 / 24_000;
        if let Some(slot) = self.slots.get(&index).filter(|_| !pcm.is_empty()) {
            let pubkey = slot.key.0.clone();
            self.log_sent(Sent {
                span,
                role: self.role(&pubkey),
                pubkey,
                echo: false,
                from_ms: from,
                to_ms: self.session_ms,
            });
        }
        self.pending = pcm;
        self.send(&mut out);
        out.push(Out::Frame(ToTalos::ReplayEnd { span }));
        out
    }

    /// One peer's ring audio in unix ms `from_ms..to_ms`. Audio older than the ring at `now_ms`
    /// is never returned, even if no tick has evicted it yet.
    fn ring_pcm(&self, index: u8, from_ms: u64, to_ms: u64, now_ms: u64) -> Vec<i16> {
        let ring = self.slots.get(&index).into_iter().flat_map(|s| &s.ring);
        ring.filter(|(t, _)| (from_ms..to_ms).contains(t) && fresh(*t, now_ms))
            .flat_map(|(_, pcm)| pcm.iter().copied())
            .collect()
    }

    /// Send-log entries overlapping session ms `start_ms..end_ms`, for mapping a transcript
    /// delta back to a peer.
    pub fn who_said(&self, start_ms: u64, end_ms: u64) -> impl Iterator<Item = &Sent> {
        self.sent
            .iter()
            .filter(move |s| s.from_ms < end_ms && start_ms < s.to_ms)
    }

    pub fn roster_frame(&self) -> ToTalos {
        let mut keys: Vec<&String> = self.roster.values().map(|(pk, _)| pk).collect();
        keys.sort();
        keys.dedup();
        let (voices, humans): (Vec<&String>, Vec<&String>) = keys
            .into_iter()
            .partition(|pk| self.policy.agents.contains(*pk));
        ToTalos::Roster {
            humans: humans.into_iter().cloned().collect(),
            voices: voices.into_iter().cloned().collect(),
        }
    }

    /// Advance 10 ms.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Out> {
        let mut out = Vec::new();
        let t = self.tuning.clone();
        let windows = self.secret.len();
        self.secret.retain(|w| w.end > now_ms);
        // Windows start by their receipt, so one not yet over holds through a clock step back.
        let secret = !self.secret.is_empty();
        // What a stall, a clock step or a window's end left queued must not play late.
        let stalled = self
            .last_tick_ms
            .is_some_and(|last| now_ms < last || now_ms - last > STALL_MS);
        self.last_tick_ms = Some(now_ms);
        if stalled || self.secret.len() < windows {
            self.flush();
        }
        let owner_peers = self
            .roster
            .values()
            .filter(|(pk, _)| *pk == self.policy.owner)
            .count();
        let desktop_here = self.route.here && owner_peers == 1;
        self.levels.clear();
        for (&index, slot) in self.slots.iter_mut() {
            slot.last.clear();
            // Age bounds the ring, so sparse ticks can't keep audio past RING_MS.
            slot.ring.retain(|(t, _)| fresh(*t, now_ms));
            let active =
                now_ms.saturating_sub(slot.last_packet_ms) < IDLE_MS || !slot.jitter.is_empty();
            let pcm = if active {
                slot.jitter.get_audio().ok().map(|(pcm, _)| pcm)
            } else {
                None
            };
            let dbov = pcm.as_deref().map_or(-127, audio_level_dbov);
            let loud = dbov >= t.speech_dbov;
            // ponytail: the route names the desktop only when the owner has one peer here.
            // Upgrade: have the desktop write its peer index into the route file.
            let desktop = desktop_here && slot.key.0 == self.policy.owner;
            let tail = if desktop {
                t.desktop_tail_ms
            } else {
                t.remote_tail_ms
            };
            let isolated = desktop && self.route.isolated;
            self.levels.push(Level {
                index,
                isolated,
                dbov,
            });
            slot.echo = !isolated && now_ms < self.output_until_ms.saturating_add(tail);
            if let Some(pcm) = pcm {
                slot.last = slot.down.process(&pcm);
                // A replay or preroll never carries secret audio or the voice's own echo.
                if !secret && !slot.echo {
                    slot.ring.push_back((now_ms, slot.last.clone()));
                    if slot.ring.len() as u64 > RING_MS / TICK_MS {
                        slot.ring.pop_front();
                    }
                }
            }
            // An onset in the echo goes out once the echo clears, if the talk goes on.
            let onset = slot.hear(loud, now_ms, &t);
            slot.onset = (slot.onset || onset) && slot.talking;
            if slot.onset && !slot.echo {
                slot.onset = false;
                out.push(Out::Frame(ToTalos::Onset {
                    pubkey: slot.key.0.clone(),
                }));
            }
        }
        if !self.awake {
            return out;
        }
        // A talker in its echo, or one that hasn't paused for `floor_cap_ms`, yields the floor
        // to any other talker.
        let demoted = |s: &Slot| s.echo || now_ms.saturating_sub(s.since_ms) >= t.floor_cap_ms;
        let rival = |held: u8| {
            let mut others = self.slots.iter().filter(|(i, _)| **i != held);
            others.any(|(_, s)| s.talking && !demoted(s))
        };
        if self.floor.as_ref().is_some_and(|f| {
            let slot = self.slots.get(&f.index);
            !slot.is_some_and(|s| s.talking) || (slot.is_some_and(demoted) && rival(f.index))
        }) {
            self.floor = None;
        }
        let mut preroll = false;
        if self.floor.is_none() {
            let next = self
                .slots
                .iter()
                .filter(|(_, s)| s.talking)
                .min_by_key(|(i, s)| (demoted(s), s.since_ms, **i));
            if let Some((&index, slot)) = next {
                self.next_span += 1;
                let floor = Floor {
                    index,
                    span: self.next_span,
                    echo: slot.echo,
                };
                out.push(Out::Frame(ToTalos::Speaker {
                    span: floor.span,
                    pubkey: slot.key.0.clone(),
                    role: self.role(&slot.key.0),
                    echo: floor.echo,
                    t_ms: self.session_ms,
                }));
                self.floor = Some(floor);
                preroll = true;
            }
        }
        let held = self.floor.as_ref().map(|f| (f, &self.slots[&f.index]));
        let audio: Vec<i16> = match held {
            Some((_, slot)) if !slot.echo && !secret && preroll => slot
                .ring
                .iter()
                .filter(|(at, _)| now_ms - at < t.onset_ms)
                .flat_map(|(_, pcm)| pcm.iter().copied())
                .collect(),
            Some((_, slot)) if !slot.echo && !secret => slot.last.clone(),
            _ => Vec::new(),
        };
        let audio = if audio.is_empty() {
            vec![0; CHUNK / 2]
        } else {
            audio
        };
        let from = self.session_ms;
        self.session_ms += audio.len() as u64 * 1000 / 24_000;
        if let Some((floor, slot)) = held.filter(|(_, s)| !s.echo && !secret) {
            let (span, echo, pubkey) = (floor.span, floor.echo, slot.key.0.clone());
            self.log_sent(Sent {
                span,
                role: self.role(&pubkey),
                pubkey,
                echo,
                from_ms: from,
                to_ms: self.session_ms,
            });
        }
        self.pending.extend(audio);
        self.send(&mut out);
        out
    }

    /// Send pending audio in whole chunks.
    fn send(&mut self, out: &mut Vec<Out>) {
        while self.pending.len() >= CHUNK {
            let chunk = self.pending.drain(..CHUNK).flat_map(|s| s.to_le_bytes());
            out.push(Out::Pcm(chunk.collect()));
        }
    }

    /// Drop every slot's queued packets and filter state; one that can't restart goes.
    fn flush(&mut self) {
        self.slots.retain(|&index, slot| {
            slot.down = Halfband::new();
            PeerJitterBuffer::new(index)
                .map(|j| slot.jitter = j)
                .is_ok()
        });
        self.prune();
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

fn key(p: &Peer) -> (String, u8) {
    (p.pubkey.clone(), p.epoch)
}

/// A ring entry stamped `t` is still inside the ring at `now_ms`. One stamped ahead of `now_ms`
/// (the clock stepped back) is not, so it can't outlive the window.
fn fresh(t: u64, now_ms: u64) -> bool {
    t <= now_ms && now_ms - t < RING_MS
}

/// 48 to 24 kHz: a 31-tap Hamming-windowed half-band low-pass, then every second sample. It
/// passes speech flat and takes about 30 dB off 14 kHz and 50 dB off 16 kHz and up.
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

    const H: &str = "5b6f8c2e-1d3a-4c8e-9f21-7a0b3c4d5e6f";
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

    impl Talker {
        fn frame(&mut self) -> Vec<u8> {
            let Talker(index, enc, amp, level) = self;
            let pcm: Vec<f32> = (0..960)
                .map(|i| *amp * (TAU * 1000.0 * i as f32 / 48_000.0).sin())
                .collect();
            let mut frame = vec![*index];
            frame.extend(enc.encode(&pcm).unwrap());
            if let Some(level) = level {
                frame[7] = *level as u8;
            }
            frame
        }
    }

    fn run(input: &mut Input, now: &mut u64, ms: u64, talkers: &mut [Talker]) -> Vec<Out> {
        let mut out = Vec::new();
        for _ in 0..ms / TICK_MS {
            for talker in talkers.iter_mut().filter(|_| now.is_multiple_of(20)) {
                input.on_audio(&talker.frame(), *now);
            }
            out.extend(input.tick(*now));
            *now += TICK_MS;
        }
        out
    }

    fn ring(input: &Input, from: u64, to: u64, now: u64) -> usize {
        input.ring_pcm(1, from, to, now).len() / (CHUNK / 2)
    }

    fn frames(out: &[Out]) -> Vec<&ToTalos> {
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
            &ToTalos::Onset {
                pubkey: OWNER.into()
            }
        );
        assert!(matches!(
            frames(&out)[1],
            ToTalos::Speaker {
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
        // A repeated `awake` keeps the span and the audio already counted.
        let mut chunks = 0;
        for _ in 0..10 {
            input.set_awake(true);
            let out = run(&mut input, &mut now, 10, &mut owner);
            chunks += out.iter().filter(|o| matches!(o, Out::Pcm(_))).count();
        }
        assert_eq!(chunks, 5);
        assert!(input.who_said(0, u64::MAX).all(|s| s.span == 1));
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
        // A new occupant of slot 4 that the gate admits is heard, after its 500 ms hold. From
        // another pod `joined` lists only the newcomer, and everyone else stays.
        let peers = serde_json::json!([{"pubkey": GUEST, "peer_index": 4, "epoch": 1}]);
        input.on_control(&serde_json::json!({"type": "joined", "peers": peers}), now);
        let out = run(&mut input, &mut now, 800, &mut others);
        let onset = |pk: &str| ToTalos::Onset { pubkey: pk.into() };
        assert_eq!(frames(&out)[0], &onset(GUEST));
        let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
        assert!(
            frames(&out).contains(&&onset(OWNER)),
            "the owner is still heard"
        );
        let roster = |humans: &[&str], voices: &[&str]| ToTalos::Roster {
            humans: humans.iter().map(|pk| pk.to_string()).collect(),
            voices: voices.iter().map(|pk| pk.to_string()).collect(),
        };
        input.on_control(&serde_json::json!({"type": "left", "peer_index": 2}), now);
        assert_eq!(input.roster_frame(), roster(&[OWNER, GUEST], &[AGENT]));
        // `roster` is the full snapshot.
        let peers = serde_json::json!([{"pubkey": OWNER, "peer_index": 1}]);
        input.on_control(&serde_json::json!({"type": "roster", "peers": peers}), now);
        assert_eq!(input.roster_frame(), roster(&[OWNER], &[]));
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
            [&ToTalos::Onset {
                pubkey: GUEST.into()
            }],
            "the guest waits"
        );
        assert!(matches!(
            frames(&after)[..],
            [ToTalos::Speaker {
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
    fn half_duplex_unless_the_desktop_here_is_isolated() {
        let mut now = T0;
        for (here, echo) in [(false, true), (true, false)] {
            let mut input = input();
            input.set_awake(true);
            input.set_route(Route {
                here,
                isolated: true,
            });
            input.set_output_until(now + 1_000);
            let mut owner = [talker(1, 0.3, None)];
            let start = now;
            let during = run(&mut input, &mut now, 600, &mut owner);
            let onset = frames(&during)
                .iter()
                .any(|f| matches!(f, ToTalos::Onset { .. }));
            assert_eq!(onset, !echo, "here: {here}");
            assert!(frames(&during)
                .iter()
                .any(|f| matches!(f, ToTalos::Speaker { echo: e, .. } if *e == echo)));
            assert_eq!(voiced(&during), !echo, "here: {here}");
            let after = run(&mut input, &mut now, 1_200, &mut owner);
            assert!(voiced(&after) && input.who_said(0, u64::MAX).all(|s| s.echo == echo));
            // Echo lasts to start + 1.6 s (1 s of output, 600 ms tail) and never enters the ring.
            let kept = if echo { 20 } else { 180 };
            assert_eq!(ring(&input, start, now, now), kept);
        }
        // Quiet, an onset in the echo goes out once the echo clears.
        let mut input = input();
        input.set_output_until(now + 100);
        let out = run(&mut input, &mut now, 1_000, &mut [talker(2, 0.3, None)]);
        let onset = ToTalos::Onset {
            pubkey: GUEST.into(),
        };
        assert_eq!(frames(&out), [&onset]);
    }

    #[test]
    fn echo_and_steady_noise_yield_the_floor() {
        for echo in [true, false] {
            let mut input = input();
            input.set_awake(true);
            let mut now = T0;
            if echo {
                // The owner on headphones, the guest on speakers while the voice talks.
                input.set_route(Route {
                    here: true,
                    isolated: true,
                });
                input.set_output_until(now + 3_000);
            } else {
                input.tuning.floor_cap_ms = 1_000;
            }
            // The guest takes the floor, then the owner talks.
            let mut ts = [talker(2, 0.3, None), talker(1, 0.0, None)];
            run(&mut input, &mut now, 1_500, &mut ts);
            ts[1].2 = 0.3;
            run(&mut input, &mut now, 500, &mut ts);
            let owner = input.who_said(0, u64::MAX).any(|s| s.pubkey == OWNER);
            assert!(owner, "echo: {echo}");
        }
    }

    #[test]
    fn audio_queued_through_a_stall_never_plays_late() {
        // The secret window, or the output and its tail, end while no tick runs.
        for secret in [true, false] {
            let mut input = input();
            input.set_awake(true);
            let mut now = T0;
            if secret {
                input.add_secret(now, now + 100, now);
            } else {
                input.set_output_until(now + 500);
            }
            input.tick(now);
            let mut owner = talker(1, 0.3, None);
            for _ in 0..55 {
                input.on_audio(&owner.frame(), now);
                now += 20;
            }
            owner.2 = 0.0;
            let out = run(&mut input, &mut now, 300, &mut [owner]);
            let pcm = input.ring_pcm(1, T0, now, now);
            assert!(
                !voiced(&out) && pcm.iter().all(|s| s.abs() < 1000),
                "{secret}"
            );
        }
    }

    #[test]
    fn a_replay_counts_on_the_session_clock_and_names_its_peer() {
        let mut input = input();
        input.set_awake(true);
        let mut now = T0;
        let mut ts = [talker(1, 0.3, None), talker(2, 0.3, None)];
        let mut out = run(&mut input, &mut now, 500, &mut ts);
        let replay = input.replay(2, now - 300, now, now);
        let at = input.session_ms;
        assert!(matches!(
            frames(&replay)[..],
            [
                ToTalos::Replay { span: 2, .. },
                ToTalos::ReplayEnd { span: 2 }
            ]
        ));
        out.extend(replay);
        out.extend(run(&mut input, &mut now, 200, &mut ts));
        // Every sample sent counts once, so Talos's ms map back to the right peer.
        let chunks = out.iter().filter(|o| matches!(o, Out::Pcm(_))).count();
        let sent = chunks * CHUNK + input.pending.len();
        assert_eq!(sent, input.session_ms as usize * 24);
        let who = |from, to| {
            let log = input.who_said(from, to);
            log.map(|s| s.pubkey.as_str()).collect::<Vec<_>>()
        };
        assert_eq!(who(at - 300, at), [GUEST]);
        assert_eq!(who(at, at + 200), [OWNER]);
        input.set_awake(false);
        assert!(input.replay(2, now - 300, now, now).is_empty(), "quiet");
    }

    #[test]
    fn secret_windows_widen_by_the_skew_and_never_reach_the_ring() {
        let mut input = input();
        let mut now = T0;
        let mut owner = [talker(1, 0.3, None)];
        let out = run(&mut input, &mut now, 3_000, &mut owner);
        assert!(
            out.iter()
                .all(|o| matches!(o, Out::Frame(ToTalos::Onset { .. }))),
            "quiet: onset only"
        );
        assert_eq!(ring(&input, T0, now, now), 300);
        // 1 s of skew on each side: T0 + 0.5 s to T0 + 2.6 s goes.
        input.add_secret(T0 + 1_500, T0 + 1_600, now);
        assert_eq!(ring(&input, T0, now, now), 90);
        assert_eq!(ring(&input, T0 + 500, T0 + 2_600, now), 0);
        // A window still ahead on this clock starts dropping on receipt: start to start + 3.1 s.
        // Forwarding when it lands, with 10 ms of voiced audio still pending.
        input.set_awake(true);
        assert!(voiced(&run(&mut input, &mut now, 200, &mut owner)));
        let start = now;
        input.add_secret(start + 2_000, start + 2_100, start);
        let out = run(&mut input, &mut now, 3_100, &mut owner);
        assert!(!voiced(&out) && ring(&input, start, now, now) == 0);
        // Its end drops what is still queued.
        run(&mut input, &mut now, 10, &mut owner);
        assert!(input.slots[&1].jitter.is_empty());
        assert!(
            voiced(&run(&mut input, &mut now, 200, &mut owner)),
            "after the window"
        );
        // A window not yet over holds when the clock steps back.
        input.add_secret(now, now + 100, now);
        now -= 5_000;
        let start = now;
        let out = run(&mut input, &mut now, 500, &mut owner);
        assert!(!voiced(&out) && ring(&input, start, now, now) == 0);
    }

    #[test]
    fn sparse_ticks_never_replay_audio_older_than_the_ring() {
        let mut input = input();
        let mut now = T0;
        run(&mut input, &mut now, 500, &mut [talker(1, 0.3, None)]);
        // No tick since, and a replay still ages entries out: T0 + 0.26 s to T0 + 0.49 s is left.
        assert_eq!(ring(&input, 0, u64::MAX, T0 + RING_MS + 250), 24);
        assert_eq!(ring(&input, 0, u64::MAX, T0 + RING_MS + 490), 0);
        // One late tick evicts by age, where a count bound would have kept all 50 entries; one
        // after the clock steps back drops entries stamped ahead of it.
        for late in [T0 + 20_000, T0 - 5_000] {
            let mut input = self::input();
            let mut now = T0;
            run(&mut input, &mut now, 500, &mut [talker(1, 0.3, None)]);
            input.tick(late);
            assert!(input.slots[&1].ring.iter().all(|(t, _)| *t == late));
        }
    }

    #[test]
    fn a_rejoin_holds_back_indices_that_changed_hands_for_500_ms() {
        let mut input = input();
        let mut now = T0;
        // The new socket's snapshot swaps the owner and the guest.
        let peers = [(1, GUEST), (2, OWNER)].map(|(index, pk)| Peer {
            index,
            pubkey: pk.into(),
            epoch: 0,
        });
        input.rejoin(&peers, now);
        let onset = |pk: &str| ToTalos::Onset { pubkey: pk.into() };
        let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
        assert!(frames(&out).is_empty(), "quarantined");
        let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
        assert_eq!(frames(&out), [&onset(GUEST)]);
        // An index whose occupant stayed is heard at once.
        input.rejoin(&peers, now);
        let out = run(&mut input, &mut now, 400, &mut [talker(2, 0.3, None)]);
        assert_eq!(frames(&out), [&onset(OWNER)]);
        // The relay's resync after missed deltas is the same stale view.
        let swap = [(1, OWNER), (2, GUEST)]
            .map(|(i, pk)| serde_json::json!({"pubkey": pk, "peer_index": i}));
        input.on_control(&serde_json::json!({"type": "roster", "peers": swap}), now);
        let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
        assert!(frames(&out).is_empty(), "quarantined");
        let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
        assert_eq!(frames(&out), [&onset(OWNER)]);
    }

    #[test]
    fn a_roster_resync_before_any_tick_or_after_a_stall_holds_back_from_now() {
        let swap = [(1, GUEST), (2, OWNER)]
            .map(|(i, pk)| serde_json::json!({"pubkey": pk, "peer_index": i}));
        let msg = serde_json::json!({"type": "roster", "peers": swap});
        let onset = ToTalos::Onset {
            pubkey: GUEST.into(),
        };
        // The last tick was never, or a minute ago: neither is the clock the hold runs on.
        for stall in [None, Some(60_000)] {
            let mut input = input();
            let mut now = T0;
            if let Some(gap) = stall {
                run(&mut input, &mut now, 100, &mut []);
                now += gap;
            }
            input.on_control(&msg, now);
            let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
            assert!(frames(&out).is_empty(), "quarantined ({stall:?})");
            let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
            assert_eq!(frames(&out), [&onset]);
        }
    }

    fn joined(peers: &[(u8, &str)]) -> serde_json::Value {
        let peers: Vec<_> = peers
            .iter()
            .map(|(i, pk)| serde_json::json!({"pubkey": pk, "peer_index": i}))
            .collect();
        serde_json::json!({"type": "joined", "peers": peers})
    }

    #[test]
    fn a_joined_reusing_an_index_holds_it_back_for_500_ms() {
        let onset = ToTalos::Onset {
            pubkey: GUEST.into(),
        };
        // `joined` hands the owner's index to the guest, or `left` frees it and `joined` reuses
        // it 400 ms later: the hold runs 500 ms from the `joined` either way.
        for gap in [None, Some(400)] {
            let mut input = input();
            let mut now = T0;
            if let Some(gap) = gap {
                let left = serde_json::json!({"type": "left", "peer_index": 1});
                input.on_control(&left, now);
                run(&mut input, &mut now, gap, &mut []);
            }
            input.on_control(&joined(&[(1, GUEST)]), now);
            let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
            assert!(frames(&out).is_empty(), "quarantined ({gap:?})");
            let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
            assert_eq!(frames(&out), [&onset]);
        }
    }

    /// A frame on `index` at `at` is taken in as `pubkey`'s: the hold, probed directly.
    fn takes(input: &mut Input, index: u8, pubkey: &str, at: u64) -> bool {
        input.on_audio(&talker(index, 0.3, None).frame(), at);
        input.slots.get(&index).is_some_and(|s| s.key.0 == pubkey)
    }

    /// `index` is `pubkey`'s exactly 500 ms after `at`: refused at +499, taken at +500.
    fn held_500_ms(input: &mut Input, index: u8, pubkey: &str, at: u64) -> bool {
        !takes(input, index, pubkey, at + 499) && takes(input, index, pubkey, at + 500)
    }

    #[test]
    fn every_hold_is_500_ms_to_the_ms() {
        let peers = |list: &[(u8, &str)]| -> Vec<Peer> {
            let peer = |&(index, pk): &(u8, &str)| Peer {
                index,
                pubkey: pk.into(),
                epoch: 0,
            };
            list.iter().map(peer).collect()
        };
        // The owner's index goes to the guest, in a reconnect's snapshot or live in `joined`.
        let mut swapped = input();
        swapped.rejoin(&peers(&[(1, GUEST), (2, OWNER)]), T0);
        assert!(held_500_ms(&mut swapped, 1, GUEST, T0), "rejoin");
        let mut swapped = input();
        swapped.on_control(&joined(&[(1, GUEST)]), T0);
        assert!(held_500_ms(&mut swapped, 1, GUEST, T0), "joined");
        // `left` frees it, or a resync or a reconnect's snapshot no longer lists it (a `left`
        // missed in the gap). A `joined` reusing it under 500 ms later is held 500 ms from the
        // `joined`; one 500 ms later is heard at once.
        let resync =
            serde_json::json!({"type": "roster", "peers": [{"pubkey": GUEST, "peer_index": 2}]});
        let left = serde_json::json!({"type": "left", "peer_index": 1});
        for what in ["left", "resync", "reconnect"] {
            for gap in [0, 250, 499, 500] {
                let mut input = input();
                match what {
                    "left" => input.on_control(&left, T0),
                    "resync" => input.on_control(&resync, T0),
                    _ => input.rejoin(&peers(&[(2, GUEST)]), T0),
                }
                let at = T0 + gap;
                input.on_control(&joined(&[(1, GUEST)]), at);
                let ok = if gap < 500 {
                    held_500_ms(&mut input, 1, GUEST, at)
                } else {
                    takes(&mut input, 1, GUEST, at)
                };
                assert!(ok, "{what}, then joined at +{gap} ms");
            }
        }
    }

    #[test]
    fn a_joined_on_a_fresh_or_unchanged_index_is_heard_at_once() {
        let onset = |pk: &str| ToTalos::Onset { pubkey: pk.into() };
        let mut input = input();
        let mut now = T0;
        // The first snapshot holds nothing back.
        let out = run(&mut input, &mut now, 400, &mut [talker(1, 0.3, None)]);
        assert_eq!(frames(&out), [&onset(OWNER)]);
        // A local `joined` lists everyone: the newcomer on an index never used, the rest unchanged.
        input.on_control(&joined(&[(1, OWNER), (2, GUEST), (5, GUEST)]), now);
        let mut ts = [talker(2, 0.3, None), talker(5, 0.3, None)];
        let out = run(&mut input, &mut now, 400, &mut ts);
        assert_eq!(frames(&out), [&onset(GUEST), &onset(GUEST)]);
        // An index freed 500 ms ago or more is fresh again.
        input.on_control(&serde_json::json!({"type": "left", "peer_index": 2}), now);
        run(&mut input, &mut now, 500, &mut []);
        input.on_control(&joined(&[(2, GUEST)]), now);
        let out = run(&mut input, &mut now, 400, &mut [talker(2, 0.3, None)]);
        assert_eq!(frames(&out), [&onset(GUEST)]);
    }

    #[test]
    fn levels_mark_the_isolated_desktop_and_a_cut_ends_the_echo_window() {
        let mut input = input();
        input.set_awake(true);
        let mut now = T0;
        input.set_route(Route {
            here: true,
            isolated: true,
        });
        input.set_output_until(now + 5_000);
        let mut ts = [talker(1, 0.3, None), talker(2, 0.3, None)];
        let out = run(&mut input, &mut now, 1_000, &mut ts);
        let mut levels: Vec<_> = input.levels().collect();
        levels.sort_by_key(|l| l.index);
        let marks: Vec<_> = levels.iter().map(|l| (l.index, l.isolated)).collect();
        assert_eq!(marks, [(1, true), (2, false)]);
        assert!(levels.iter().all(|l| l.dbov > -20));
        // The owner holds the floor; the guest on speakers is in the echo for 5 s, unless a cut
        // ends the window: then only its 600 ms tail is left, and its audio is forwarded.
        assert!(input.who_said(0, u64::MAX).all(|s| s.pubkey == OWNER));
        assert!(voiced(&out));
        ts[0].2 = 0.0;
        input.set_output_until(now);
        run(&mut input, &mut now, 1_000, &mut ts);
        assert!(input.who_said(0, u64::MAX).any(|s| s.pubkey == GUEST));
    }

    #[test]
    fn the_floor_starts_with_the_onset_audio() {
        let mut input = input();
        input.set_awake(true);
        let mut now = T0;
        run(&mut input, &mut now, 300, &mut [talker(1, 0.3, None)]);
        // 30 ticks of 10 ms, plus the onset's 100 ms sent at once in place of one tick.
        assert_eq!(input.who_said(0, u64::MAX).last().unwrap().to_ms, 390);
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
        assert!(gain(14_000.0) < 0.04 && gain(16_000.0) < 0.004 && gain(20_000.0) < 0.01);
    }

    #[test]
    fn route_file_counts_only_for_this_huddle_while_fresh() {
        let path =
            std::env::temp_dir().join(format!("huddle-bridge-route-{}.json", std::process::id()));
        let read = |text: String| {
            std::fs::write(&path, text).unwrap();
            read_route(&path, H, T0)
        };
        let file = |huddle: &str, age_s: u64| {
            let at = T0 / 1000 - age_s;
            format!(r#"{{"isolated":true,"huddle":"{huddle}","updated_at":{at}}}"#)
        };
        let on = Route {
            here: true,
            isolated: true,
        };
        assert_eq!(read(file(H, 5)), on);
        assert_eq!(read(file("other", 5)), Route::default());
        // A desktop that crashed, or left without deleting it: three heartbeats missed.
        assert_eq!(read(file(H, 20)), Route::default());
        let unnamed = format!(r#"{{"isolated":true,"updated_at":{}}}"#, T0 / 1000);
        assert_eq!(read(unnamed), Route::default());
        assert_eq!(read("not json".into()), Route::default());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read_route(&path, H, T0), Route::default());
    }
}
