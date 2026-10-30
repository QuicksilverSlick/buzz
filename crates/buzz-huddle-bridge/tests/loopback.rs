//! Loopback: the bridge's public API against a fake relay audio endpoint and a fake LiveSession
//! that says back what it hears. The join runs over a real local socket; the rest runs in memory
//! on a paused clock, so pacing and windows are exact and the restart spread is a bounded random
//! draw. No network, no OpenAI. NetEQ
//! times arrivals on the wall clock, so there its decode delay varies (up to 200 ms) run to run;
//! a timing that includes it is checked against what the bridge itself heard, or as a bound.
//!
//! ponytail: the fakes live here until slice 8a needs them from buzz-acp, then move behind a
//! `loopback` feature; 3d runs the relay as a binary.

use buzz_audio_client::connection::{parse_peers, Peer};
use buzz_audio_client::encoder::AudioEncoder;
use buzz_audio_client::wire::{FrameHeader, PROTOCOL_VERSION};
use buzz_huddle_bridge::frames::{self, Role, ToPc, ToTalos};
use buzz_huddle_bridge::input::{Input, Out, Policy, Route, Tuning};
use buzz_huddle_bridge::join::{audio_request, join};
use buzz_huddle_bridge::link::{run, Cmd, End};
use buzz_huddle_bridge::output::{Barge, Output};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::f32::consts::TAU;
use std::future::Future;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, timeout, Duration, Instant};
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role as Side};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use uuid::Uuid;

const OWNER: &str = "0000000000000000000000000000000000000000000000000000000000000001";
/// Another agent's voice in the room.
const AGENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GUEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const GUEST2: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
/// A human the agent's author gate doesn't admit.
const STRANGER: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
/// This voice, in the in-memory rooms.
const ME: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Wait until `done`, for at most 30 s, so a broken bridge fails rather than spins.
async fn until(done: impl Fn() -> bool) {
    let wait = async {
        while !done() {
            sleep(ms(5)).await;
        }
    };
    timeout(ms(30_000), wait).await.expect("timed out");
}

fn policy(me: &str) -> Policy {
    Policy {
        owner: OWNER.into(),
        agents: [AGENT.into(), me.into()].into(),
        admitted: [GUEST.into(), GUEST2.into(), AGENT.into()].into(),
    }
}

/// One peer's mic: 20 ms Opus frames of a 1 kHz tone at `amp`, plus what it picks up.
struct Mic {
    enc: AudioEncoder,
    amp: f32,
    n: usize,
}

impl Mic {
    fn frame(&mut self, index: u8, pickup: &[f32]) -> Vec<u8> {
        let pcm: Vec<f32> = (0..960)
            .map(|i| {
                let tone = self.amp * (TAU * ((self.n + i) % 48) as f32 / 48.0).sin();
                tone + pickup.get(i).copied().unwrap_or(0.0)
            })
            .collect();
        self.n += 960;
        [vec![index], self.enc.encode(&pcm).unwrap()].concat()
    }
}

/// The fake relay audio endpoint for one huddle. Each 20 ms it sends queued control messages,
/// then every mic's frame with its peer index. What the voice plays is timed and decoded, and
/// with `bleed` (speakers, no headset) it comes back on the owner's mic, delayed.
#[derive(Default)]
struct Relay {
    peers: RefCell<Vec<(u8, String)>>,
    mics: RefCell<BTreeMap<u8, Mic>>,
    /// The room's audio protocol version, pinned by the first peer to join.
    pinned: Cell<Option<u64>>,
    auth: RefCell<Value>,
    control: RefCell<VecDeque<Value>>,
    /// Close the socket with this code at the next tick.
    kick: Cell<Option<u16>>,
    /// On speakers, how long after the relay sends the voice's audio it is back in the owner's mic.
    bleed: Cell<Option<Duration>>,
    dials: RefCell<Vec<Instant>>,
    played: RefCell<Vec<Instant>>,
    closes: RefCell<Vec<Option<u16>>>,
}

impl Relay {
    /// A room of these peers, each with a mic at its amplitude.
    fn with(peers: &[(u8, &str, f32)]) -> Self {
        let relay = Self::default();
        for &(index, pubkey, amp) in peers {
            relay.seat(index, pubkey, Some(amp));
        }
        relay
    }

    /// `pubkey` takes `index`, with a fresh mic at `amp` if it has one: the relay's `joined`,
    /// whose `peers` is the new snapshot.
    fn seat(&self, index: u8, pubkey: &str, amp: Option<f32>) -> Value {
        if let Some(amp) = amp {
            let enc = AudioEncoder::new().unwrap();
            self.mics.borrow_mut().insert(index, Mic { enc, amp, n: 0 });
        }
        let mut peers = self.peers.borrow_mut();
        peers.retain(|(i, pk)| *i != index && pk != pubkey);
        peers.push((index, pubkey.into()));
        let all: Vec<Value> = peers
            .iter()
            .map(|(i, pk)| json!({"pubkey": pk, "peer_index": i, "epoch": 0}))
            .collect();
        json!({"type": "joined", "pubkey": pubkey, "peer_index": index, "epoch": 0, "peers": all})
    }

    /// `pubkey` joins at the lowest index free (the relay's are 0-254), or its own.
    fn admit(&self, pubkey: &str) -> Value {
        let peers = self.peers.borrow();
        let free = |i: &u8| peers.iter().all(|(j, pk)| j != i || pk == pubkey);
        let index = (0..u8::MAX).find(free).unwrap();
        drop(peers);
        self.seat(index, pubkey, None)
    }

    fn talk(&self, index: u8, amp: f32) {
        self.mics.borrow_mut().get_mut(&index).unwrap().amp = amp;
    }

    /// The relay's admission: a challenge, the 22242 checked as the relay checks it, the room's
    /// version pin, then `joined`; or an `error` and a Close.
    async fn handshake(&self, ws: &mut WebSocketStream<TcpStream>, origin: &str) -> bool {
        let text = |v: Value| Message::Text(v.to_string().into());
        let challenge = json!({"type": "challenge", "challenge": "c-1"});
        let _ = ws.send(text(challenge)).await;
        let Some(Ok(Message::Text(auth))) = ws.next().await else {
            return false;
        };
        let auth: Value = serde_json::from_str(&auth).unwrap();
        let event: Event = serde_json::from_value(auth["event"].clone()).unwrap();
        let tags = auth["event"]["tags"].as_array().unwrap();
        let now = Timestamp::now().as_secs();
        let signed = event.verify().is_ok()
            && event.created_at.as_secs().abs_diff(now) <= 60
            && event.kind == Kind::Authentication
            && tags.contains(&json!(["challenge", "c-1"]))
            && tags.contains(&json!(["relay", origin]));
        let requested = auth["protocol_version"].as_u64().unwrap_or(1);
        let pinned = self.pinned.get().unwrap_or(requested);
        let error = if !signed {
            json!({"type": "error", "message": "auth failed"})
        } else if pinned != requested {
            let words = "this huddle is using audio protocol";
            let message = format!("{words} v{pinned}; your client requested v{requested}");
            json!({"type": "error", "code": "upgrade_required", "message": message})
        } else {
            self.pinned.set(Some(pinned));
            *self.auth.borrow_mut() = auth;
            let joined = self.admit(&event.pubkey.to_hex());
            return ws.send(text(joined)).await.is_ok();
        };
        let _ = ws.send(text(error)).await;
        let _ = ws.close(None).await;
        false
    }

    /// Serve one joined socket until it closes.
    async fn serve<S: AsyncRead + AsyncWrite + Unpin>(&self, mut ws: WebSocketStream<S>) {
        let mut decoder = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
        let mut echo = VecDeque::new();
        let mut tick = interval(ms(20));
        loop {
            tokio::select! {
                msg = ws.next() => match msg {
                    Some(Ok(Message::Binary(frame))) => {
                        // The relay drops a frame with no Opus payload, or over 4 KiB.
                        assert!((9..=4096).contains(&frame.len()), "{} bytes", frame.len());
                        self.played.borrow_mut().push(Instant::now());
                        let (_, opus) = FrameHeader::parse(&frame).expect("a v2 frame");
                        let mut pcm = vec![0.0; 960];
                        let n = decoder.decode_float(opus, &mut pcm, false).unwrap();
                        pcm.truncate(n);
                        if let Some(delay) = self.bleed.get() {
                            echo.push_back((Instant::now() + delay, pcm));
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        self.closes.borrow_mut().push(frame.map(|f| f.code.into()));
                    }
                    Some(Ok(_)) => {}
                    _ => return,
                },
                _ = tick.tick() => {
                    if let Some(code) = self.kick.take() {
                        let close = CloseFrame { code: code.into(), reason: "".into() };
                        // Nothing more goes out on this socket.
                        let _ = ws.close(Some(close)).await;
                    }
                    let due = echo.front().is_some_and(|(at, _)| *at <= Instant::now());
                    let pickup = if due { echo.pop_front().unwrap().1 } else { Vec::new() };
                    let owner = self.peers.borrow().iter().find(|(_, pk)| pk == OWNER).map(|p| p.0);
                    let control = self.control.borrow_mut().drain(..).collect::<Vec<_>>();
                    let mut out: Vec<Message> =
                        control.into_iter().map(|c| Message::Text(c.to_string().into())).collect();
                    for (&index, mic) in self.mics.borrow_mut().iter_mut() {
                        let pickup = if Some(index) == owner { &pickup[..] } else { &[] };
                        out.push(Message::Binary(mic.frame(index, pickup).into()));
                    }
                    for msg in out {
                        let _ = ws.send(msg).await;
                    }
                }
            }
        }
    }
}

/// The fake LiveSession: it reads each frame as Talos parses it, wakes the voice, and 300 ms after
/// a talker's turn ends takes the floor with a newer token and says it back `repeat` times, paced.
#[derive(Default)]
struct Talos {
    log: RefCell<Vec<(Instant, Out)>>,
    repeat: Cell<usize>,
}

impl Talos {
    async fn run(&self, mut outs: mpsc::Receiver<Out>, cmd: mpsc::Sender<Cmd>) {
        // A frame through its v1 text both ways, then what 8a's frame client does with it.
        let to_bridge = |frame: ToPc| match frames::parse(&frames::to_text(&frame)).unwrap() {
            ToPc::Floor { holder, token } => Cmd::Floor { holder, token },
            ToPc::Awake { .. } => Cmd::Awake(true),
            other => unreachable!("{other:?}"),
        };
        let awake = ToPc::Awake { expires_at: 1 };
        let _ = cmd.send(to_bridge(awake)).await;
        let (mut turn, mut quiet, mut token) = (Vec::new(), 0, 0);
        let mut reply = VecDeque::new();
        let mut pace = interval(ms(20));
        loop {
            tokio::select! {
                out = outs.recv() => {
                    let out = match out {
                        None => return,
                        Some(Out::Frame(f)) => Out::Frame(frames::parse(&frames::to_text(&f)).unwrap()),
                        Some(Out::Pcm(pcm)) => {
                            if voiced(&pcm) {
                                quiet = 0;
                                turn.push(pcm.clone());
                            } else if !turn.is_empty() {
                                quiet += 1;
                            }
                            if quiet == 15 {
                                token += 1;
                                quiet = 0;
                                let floor = ToPc::Floor { holder: Some(ME.into()), token };
                                let _ = cmd.send(to_bridge(floor)).await;
                                let n = turn.len() * self.repeat.get();
                                reply = std::mem::take(&mut turn).into_iter().cycle().take(n).collect();
                                // One chunk of lead, so the bridge's queue never runs dry.
                                if let Some(pcm) = reply.pop_front() {
                                    let _ = cmd.send(Cmd::Pcm(pcm)).await;
                                }
                            }
                            Out::Pcm(pcm)
                        }
                    };
                    self.log.borrow_mut().push((Instant::now(), out));
                }
                _ = pace.tick() => {
                    if let Some(pcm) = reply.pop_front() {
                        let _ = cmd.send(Cmd::Pcm(pcm)).await;
                    }
                }
            }
        }
    }
}

/// The bridge for voice `me`, until it ends.
async fn link<S, D, F>(
    me: &str,
    dial: D,
    mut cmds: mpsc::Receiver<Cmd>,
    outs: mpsc::Sender<Out>,
) -> End
where
    S: AsyncRead + AsyncWrite + Unpin,
    D: FnMut() -> F,
    F: Future<Output = Result<(WebSocketStream<S>, Vec<Peer>), String>>,
{
    let mut input = Input::new(policy(me), Tuning::default());
    let mut output = Output::new(me.into(), Barge::default()).unwrap();
    run(dial, &mut input, &mut output, &mut cmds, &outs).await
}

/// The bridge for ME in `relay`'s room over memory sockets, each dial returning the joined
/// snapshot, with `talos` on the far side while `script` runs; then it leaves.
async fn loopback<F>(relay: &Relay, talos: &Talos, script: impl FnOnce(mpsc::Sender<Cmd>) -> F)
where
    F: Future<Output = ()>,
{
    let (cmd, cmds) = mpsc::channel(64);
    let (outs, rx) = mpsc::channel(4096);
    let (accept, mut sockets) = mpsc::channel(1);
    let dial = move || {
        let accept = accept.clone();
        async move {
            relay.dials.borrow_mut().push(Instant::now());
            let (client, server) = tokio::io::duplex(1 << 16);
            let server = WebSocketStream::from_raw_socket(server, Side::Server, None).await;
            accept.send(server).await.map_err(|e| e.to_string())?;
            let client = WebSocketStream::from_raw_socket(client, Side::Client, None).await;
            Ok::<_, String>((client, parse_peers(&relay.admit(ME))?))
        }
    };
    let host = async {
        while let Some(ws) = sockets.recv().await {
            relay.serve(ws).await;
        }
    };
    let script = async {
        script(cmd.clone()).await;
        cmd.send(Cmd::Leave).await.unwrap();
    };
    let voice = link(ME, dial, cmds, outs);
    let (end, ..) = tokio::join!(voice, host, talos.run(rx, cmd.clone()), script);
    assert_eq!(end, End::Left);
}

fn samples(pcm: &[u8]) -> Vec<f32> {
    let s = pcm.chunks(2).map(|s| i16::from_le_bytes([s[0], s[1]]));
    s.map(|s| f32::from(s) / 32_768.0).collect()
}

fn voiced(pcm: &[u8]) -> bool {
    samples(pcm).iter().any(|s| s.abs() > 0.03)
}

/// Sign changes: 40 in 20 ms of a 1 kHz tone, at any sample rate.
fn crossings(s: &[f32]) -> usize {
    s.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

/// The 20 ms chunks Talos heard with speech in them, and when.
fn speech(log: &[(Instant, Out)]) -> Vec<(Instant, &[u8])> {
    let voiced = log.iter().filter_map(|(t, o)| match o {
        Out::Pcm(p) if voiced(p) => Some((*t, &p[..])),
        _ => None,
    });
    voiced.collect()
}

/// The frames Talos got at or after `after`, in order.
fn frames_since(talos: &Talos, after: Instant) -> Vec<ToTalos> {
    let log = talos.log.borrow();
    let frames = log.iter().filter_map(|(t, o)| match o {
        Out::Frame(f) if *t >= after => Some(f.clone()),
        _ => None,
    });
    frames.collect()
}

/// When Talos first got a `speaker` (or, with `onset`, an `onset`) for `pubkey` after `after`.
fn first(talos: &Talos, onset: bool, pubkey: &str, after: Instant) -> Option<Instant> {
    let log = talos.log.borrow();
    let mut frames = log.iter().filter(|(t, _)| *t >= after);
    let found = frames.find(|(_, o)| match o {
        Out::Frame(ToTalos::Onset { pubkey: pk }) => onset && pk == pubkey,
        Out::Frame(ToTalos::Speaker { pubkey: pk, .. }) => !onset && pk == pubkey,
        _ => false,
    });
    found.map(|(t, _)| *t)
}

fn roster(humans: &[&str], voices: &[&str]) -> ToTalos {
    let keys = |k: &[&str]| k.iter().map(|k| k.to_string()).collect();
    let (humans, voices) = (keys(humans), keys(voices));
    ToTalos::Roster { humans, voices }
}

/// The rosters Talos got at or after `after`.
fn rosters(talos: &Talos, after: Instant) -> Vec<ToTalos> {
    let frames = frames_since(talos, after).into_iter();
    frames
        .filter(|f| matches!(f, ToTalos::Roster { .. }))
        .collect()
}

/// The owner's desktop in the huddle, on speakers.
const DESKTOP: Route = Route {
    here: true,
    isolated: false,
};

/// How long the voice's audio takes from the relay back into the owner's mic on `route`: never on
/// headphones, 40 ms from the desktop, 200 ms from the owner's phone. A tail covers this plus the
/// bridge's own NetEQ delay, here at its 200 ms cap, so echo leaks from 65 ms on the desktop (its
/// 300 ms tail) and 380 ms on the phone (600 ms). Probe P measures the real path.
fn bleed(route: Route) -> Option<Duration> {
    (!route.isolated).then(|| ms(if route.here { 40 } else { 200 }))
}

/// Over a real local socket the agent answers the challenge as itself, for the dialed origin, with
/// its owner tag, the parent channel and buzz-audio-client's protocol version, and leaves with a
/// 1000. A room whose first peer pinned another version refuses it, and the bridge doesn't retry.
#[tokio::test]
async fn the_agent_signs_its_way_in_and_a_room_pinned_to_another_version_refuses_it_for_good() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("ws://{}", listener.local_addr().unwrap());
    let relay = Relay::with(&[(1, OWNER, 0.0)]);
    let (keys, huddle, parent) = (Keys::generate(), Uuid::new_v4(), Uuid::new_v4());
    let me = keys.public_key().to_hex();
    let owner_tag = Tag::parse(["auth", OWNER, "", "sig"]).unwrap();
    let dials = Cell::new(0);
    let dial = {
        let (origin, keys, owner_tag, dials) = (&origin, &keys, &owner_tag, &dials);
        move || {
            dials.set(dials.get() + 1);
            let request = audio_request(origin, huddle);
            async move {
                let joined = join(request?, parent, keys, Some(owner_tag)).await?;
                Ok::<_, String>((joined.socket, joined.peers))
            }
        }
    };
    let host = async {
        for _ in 0..2 {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            if relay.handshake(&mut ws, &origin).await {
                relay.serve(ws).await;
            }
        }
    };
    let voice = async {
        let (cmd, cmds) = mpsc::channel(8);
        let (outs, mut talos) = mpsc::channel(64);
        let leave = async {
            let joined = Out::Frame(roster(&[OWNER], &[me.as_str()]));
            assert_eq!(talos.recv().await, Some(joined));
            cmd.send(Cmd::Leave).await.unwrap();
            while talos.recv().await.is_some() {}
        };
        let session = async { tokio::join!(link(&me, dial, cmds, outs), leave).0 };
        assert_eq!(timeout(ms(10_000), session).await, Ok(End::Left));
        let auth = relay.auth.borrow().clone();
        let tags = auth["event"]["tags"].as_array().unwrap();
        assert!(tags.contains(&json!(owner_tag)), "{tags:?}");
        assert_eq!(auth["parent_channel_id"], parent.to_string());
        assert_eq!(auth["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(relay.closes.borrow().last(), Some(&Some(1000)));

        relay.pinned.set(Some(1));
        let (_cmd, cmds) = mpsc::channel(8);
        let refusal = timeout(ms(5_000), link(&me, dial, cmds, mpsc::channel(8).0));
        let words = "this huddle is using audio protocol v1; your client requested v";
        let refused = End::Refused(format!("{words}{PROTOCOL_VERSION}"));
        assert_eq!(refusal.await, Ok(refused));
        assert_eq!(dials.get(), 2, "no retry after a version refusal");
    };
    tokio::join!(host, voice);
}

/// The voice hears only the owner, though another voice and a stranger talk throughout, at
/// 24 kHz; says it back at 48 kHz paced 20 ms apart; on the desktop's or the phone's speakers
/// neither cuts itself on its echo nor hears it; replays the owner's ring without that echo, and
/// nothing of another voice's or a stranger's; and leaves with a 1000.
#[tokio::test(start_paused = true)]
async fn the_voice_hears_the_owner_and_says_it_back_paced_without_hearing_its_own_echo() {
    for route in [DESKTOP, Route::default()] {
        let room = [(1, OWNER, 0.0), (3, AGENT, 0.5), (4, STRANGER, 0.5)];
        let (relay, talos) = (&Relay::with(&room), &Talos::default());
        relay.bleed.set(bleed(route));
        talos.repeat.set(1);
        loopback(relay, talos, |cmd| async move {
            cmd.send(Cmd::Route(route)).await.unwrap();
            let start = Instant::now();
            sleep(ms(200)).await;
            relay.talk(1, 0.1);
            sleep(ms(600)).await;
            relay.talk(1, 0.0);
            sleep(ms(2_500)).await;
            let said = frames_since(talos, start);
            assert_eq!(said[0], roster(&[OWNER, STRANGER], &[AGENT, ME]));
            let owner = |pk: &String| pk == OWNER;
            assert!(
                matches!(&said[1..], [
                    ToTalos::Onset { pubkey: a },
                    ToTalos::Speaker { pubkey: b, role: Role::Owner, echo: false, .. },
                    ToTalos::Speaker { pubkey: c, echo: true, .. },
                ] if owner(a) && owner(b) && owner(c)),
                "{route:?}: only the owner, and the echo span flagged: {said:?}"
            );
            let turn = {
                let (log, played) = (talos.log.borrow(), relay.played.borrow());
                let (turn, late): (Vec<_>, Vec<_>) =
                    speech(&log).into_iter().partition(|(t, _)| *t < played[0]);
                assert!(late.is_empty(), "{route:?}: the echo is never heard");
                assert!(turn.len() >= 30, "{}", turn.len());
                let mid = crossings(&samples(turn[turn.len() / 2].1));
                assert!((38..=42).contains(&mid), "{mid} crossings at 24 kHz");
                assert_eq!(played.len(), turn.len(), "{route:?}: cut by its echo");
                assert!(played.windows(2).all(|w| w[1] - w[0] == ms(20)), "paced");
                turn.len()
            };
            for (index, want) in [(1, turn), (3, 0), (4, 0)] {
                let asked = Instant::now();
                let all = Cmd::Replay {
                    index,
                    from_ms: 0,
                    to_ms: u64::MAX,
                };
                cmd.send(all).await.unwrap();
                sleep(ms(50)).await;
                let frames = frames_since(talos, asked);
                let replay = matches!(
                    &frames[..],
                    [ToTalos::Replay { .. }, ToTalos::ReplayEnd { .. }]
                );
                assert!(replay, "{frames:?}");
                let log = talos.log.borrow();
                let replayed = speech(&log).iter().filter(|(t, _)| *t >= asked).count();
                // The owner's turn to a chunk or two; the agent's and the stranger's rings are empty.
                let close = replayed.abs_diff(want) <= want.min(2);
                assert!(close, "{route:?}: {replayed} of {want} from {index}");
            }
        })
        .await;
        assert_eq!(relay.closes.borrow().last(), Some(&Some(1000)));
    }
}

/// The owner talking over the voice cuts it: after 200 ms of energy on the isolated desktop, or
/// 320 ms over the echo on speakers. The rest of that response never plays, and the owner is heard:
/// at once on headphones, else once the echo tail after the cut (desktop 300 ms, phone 600) passes.
#[tokio::test(start_paused = true)]
async fn talking_over_the_voice_cuts_it_after_200_ms_on_headphones_or_320_ms_on_speakers() {
    let headphones = Route {
        here: true,
        isolated: true,
    };
    for (route, need) in [(headphones, 200), (DESKTOP, 320), (Route::default(), 320)] {
        let (relay, talos) = (&Relay::with(&[(1, OWNER, 0.0)]), &Talos::default());
        relay.bleed.set(bleed(route));
        talos.repeat.set(4);
        loopback(relay, talos, |cmd| async move {
            cmd.send(Cmd::Route(route)).await.unwrap();
            sleep(ms(200)).await;
            relay.talk(1, 0.1);
            sleep(ms(600)).await;
            relay.talk(1, 0.0);
            until(|| relay.played.borrow().len() >= 20).await;
            let barged = Instant::now();
            relay.talk(1, 0.8);
            // Past the end of the response, had it played on.
            sleep(ms(3_000)).await;
            let played = relay.played.borrow();
            let cut = *played.last().unwrap() + ms(20);
            let lag = cut - barged;
            assert!(played.len() < 60, "{route:?}: the rest never plays");
            assert!(lag >= ms(need), "{route:?}: cut {lag:?} after the barge");
            let onset = first(talos, true, OWNER, barged).expect("the owner is heard");
            if route.isolated {
                // Heard at once: 100 ms to the onset, 200 ms to the cut.
                let after = cut - onset;
                assert!((ms(80)..ms(130)).contains(&after), "cut {after:?} after");
            } else {
                let (after, tail) = (onset - cut, ms(if route.here { 300 } else { 600 }));
                let heard = (tail..tail + ms(40)).contains(&after);
                assert!(heard, "{route:?}: heard {after:?} after the cut");
                assert!(lag < ms(need + 300), "cut {lag:?} after the barge");
            }
        })
        .await;
    }
}

/// A relay restart (1012) rejoins within the 10 s spread on the new snapshot, and an index that
/// changed hands is held back 500 ms. One reused live (`left`, `joined`) is held back as long,
/// then is its new occupant's.
#[tokio::test(start_paused = true)]
async fn a_1012_rejoins_on_the_new_roster_and_holds_back_an_index_that_changed_hands() {
    let room = [(1, OWNER, 0.0), (2, GUEST, 0.0)];
    let (relay, talos) = (&Relay::with(&room), &Talos::default());
    loopback(relay, talos, |_| async move {
        sleep(ms(500)).await;
        // While the relay restarts, the guest leaves and GUEST2 takes index 2, talking.
        relay.seat(2, GUEST2, Some(0.3));
        relay.kick.set(Some(1012));
        let kicked = Instant::now();
        until(|| relay.dials.borrow().len() >= 2).await;
        sleep(ms(1_500)).await;
        let rejoined = relay.dials.borrow()[1];
        assert!(rejoined - kicked <= ms(10_020), "{:?}", rejoined - kicked);
        assert_eq!(rosters(talos, rejoined), [roster(&[OWNER, GUEST2], &[ME])]);
        let held = first(talos, false, GUEST2, rejoined).expect("GUEST2 heard") - rejoined;
        assert!((ms(500)..ms(900)).contains(&held), "held back {held:?}");
        // Live, GUEST2 leaves and GUEST takes index 2 back.
        let left = json!({"type": "left", "pubkey": GUEST2, "peer_index": 2, "epoch": 0});
        let joined = relay.seat(2, GUEST, Some(0.3));
        relay.control.borrow_mut().extend([left, joined]);
        let swapped = Instant::now();
        sleep(ms(1_000)).await;
        let held = first(talos, false, GUEST, swapped).expect("index 2 is GUEST's now") - swapped;
        assert!((ms(500)..ms(900)).contains(&held), "held back {held:?}");
        let left_then_joined = [roster(&[OWNER], &[ME]), roster(&[OWNER, GUEST], &[ME])];
        assert_eq!(rosters(talos, swapped), left_then_joined);
        // A second restart in a row (a 2 s backoff) sends a roster too, though nothing changed.
        relay.kick.set(Some(1012));
        until(|| relay.dials.borrow().len() >= 3).await;
        sleep(ms(100)).await;
        let again = rosters(talos, relay.dials.borrow()[2]);
        assert_eq!(again, [roster(&[OWNER, GUEST], &[ME])]);
    })
    .await;
    assert_eq!(relay.dials.borrow().len(), 3);
}
