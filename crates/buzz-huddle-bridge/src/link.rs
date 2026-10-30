//! The relay socket for one voice: room audio into [`Input`], [`Output`] paced onto the relay one
//! frame per 20 ms, commands from the voice module applied in between, a clean Close on leave,
//! and rejoins after a drop. A rolling relay deploy closes with 1012 or 1001, so the first
//! rejoin waits a random 0-10 s, as upstream PR 8653 does for buzz-acp, and a room's voices
//! don't all return at once. Otherwise it backs off 1, 2, 4 ... 30 s. A version refusal is final.

use crate::frames::ToTalos;
use crate::input::{Input, Out, Policy, Route};
use crate::output::Output;
use buzz_audio_client::connection::Peer;
use futures_util::{SinkExt, StreamExt};
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::WebSocketStream;

const STEP_MS: u64 = 10;
/// More steps behind than this and the loop skips ahead; Input drops what queued in the gap.
const CATCH_UP: u64 = 4;
const SPREAD: Duration = Duration::from_secs(10);
const MAX_WAIT: Duration = Duration::from_secs(30);
/// A socket joined this long resets the backoff.
const STABLE: Duration = Duration::from_secs(30);
const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// From the voice module, applied between steps.
#[derive(Debug)]
pub enum Cmd {
    /// PCM16 LE, 24 kHz mono, from the voice. Dropped while no socket is joined.
    Pcm(Vec<u8>),
    Floor {
        holder: Option<String>,
        token: u64,
    },
    Awake(bool),
    Secret {
        from_ms: u64,
        to_ms: u64,
    },
    Replay {
        index: u8,
        from_ms: u64,
        to_ms: u64,
    },
    Policy(Policy),
    Route(Route),
    /// Close the socket with 1000 and stop. Closing the channel does the same.
    Leave,
}

/// Why a socket ended.
#[derive(Debug, PartialEq)]
pub enum End {
    Left,
    /// The relay's Close code, or `None` for a dropped socket.
    Closed(Option<u16>),
    /// The dial failed or the relay refused the join, with its message. `buzz-audio-client`
    /// keeps only the relay's message, and both version refusals (`upgrade_required`,
    /// `unsupported_version`) say "audio protocol v".
    Refused(String),
}

impl End {
    fn is_final(&self) -> bool {
        match self {
            End::Left => true,
            End::Refused(message) => message.contains("audio protocol v"),
            End::Closed(_) => false,
        }
    }

    /// A relay pod going away: its 1012 or 1001, or a join it refused while draining.
    fn restarting(&self) -> bool {
        match self {
            End::Closed(code) => matches!(code, Some(1001 | 1012)),
            End::Refused(message) => message.contains("relay is draining"),
            End::Left => false,
        }
    }
}

#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    /// The wait before rejoining after `end`, on a socket that stayed joined for `up` (zero for
    /// a failed dial), or `None` when the end is final. `unit` is a uniform draw in [0, 1].
    pub fn next(&mut self, end: &End, up: Duration, unit: f64) -> Option<Duration> {
        if end.is_final() {
            return None;
        }
        if up >= STABLE {
            self.failures = 0;
        }
        if self.failures == 0 && end.restarting() {
            return Some(SPREAD.mul_f64(unit.clamp(0.0, 1.0)));
        }
        let wait = Duration::from_secs(1 << self.failures.min(5)).min(MAX_WAIT);
        self.failures += 1;
        Some(wait)
    }
}

/// A uniform draw in [0, 1) from the OS RNG, through a v4 UUID as upstream PR 8653 does.
fn unit() -> f64 {
    let (_, low) = uuid::Uuid::new_v4().as_u64_pair();
    (low >> 11) as f64 / (1u64 << 53) as f64
}

/// Join, run and rejoin until the voice leaves or the relay refuses for good; that end is
/// returned. `dial` joins the room (see [`crate::join::join`]) and yields the socket and its peer
/// snapshot. A rejoin rebuilds the peer map with [`Input::rejoin`]. Talos gets a `roster` on
/// every join and whenever the relay's changes it; 8c voids any pending read-back on the join's.
pub async fn run<S, D, F>(
    mut dial: D,
    input: &mut Input,
    output: &mut Output,
    cmds: &mut mpsc::Receiver<Cmd>,
    outs: &mpsc::Sender<Out>,
) -> End
where
    S: AsyncRead + AsyncWrite + Unpin,
    D: FnMut() -> F,
    F: Future<Output = Result<(WebSocketStream<S>, Vec<Peer>), String>>,
{
    let mut room = Room {
        input,
        output,
        outs,
        clock: Clock::new(),
        step: 0,
        roster: None,
    };
    let (mut backoff, mut joined) = (Backoff::default(), false);
    loop {
        let Some(dialed) = room.between(dial(), cmds).await else {
            return End::Left;
        };
        let (end, up) = match dialed {
            Ok((socket, peers)) => {
                let now = room.clock.now();
                if joined {
                    room.input.rejoin(&peers, now);
                } else {
                    room.input.set_roster(&peers);
                }
                joined = true;
                room.roster(true).await;
                let at = Instant::now();
                let end = room.session(socket, cmds).await;
                room.output.flush();
                (end, at.elapsed())
            }
            Err(e) => (End::Refused(e), Duration::ZERO),
        };
        let Some(wait) = backoff.next(&end, up, unit()) else {
            return end;
        };
        tracing::info!(?end, ?wait, "huddle bridge: rejoining the audio room");
        if room.between(tokio::time::sleep(wait), cmds).await.is_none() {
            return End::Left;
        }
    }
}

/// Unix ms on a monotonic clock, counted in 10 ms steps.
struct Clock {
    unix0: u64,
    start: Instant,
}

impl Clock {
    fn new() -> Self {
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            unix0: unix.as_millis() as u64,
            start: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.unix0 + self.start.elapsed().as_millis() as u64
    }

    fn steps(&self) -> u64 {
        self.start.elapsed().as_millis() as u64 / STEP_MS
    }
}

struct Room<'a> {
    input: &'a mut Input,
    output: &'a mut Output,
    outs: &'a mpsc::Sender<Out>,
    clock: Clock,
    step: u64,
    roster: Option<ToTalos>,
}

impl Room<'_> {
    async fn session<S>(
        &mut self,
        mut socket: WebSocketStream<S>,
        cmds: &mut mpsc::Receiver<Cmd>,
    ) -> End
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut wake = tokio::time::interval(Duration::from_millis(STEP_MS));
        wake.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    None | Some(Cmd::Leave) => {
                        let frame = CloseFrame { code: CloseCode::Normal, reason: "".into() };
                        let _ = socket.close(Some(frame)).await;
                        drain(socket).await;
                        return End::Left;
                    }
                    Some(cmd) => self.apply(cmd).await,
                },
                msg = socket.next() => match msg {
                    Some(Ok(Message::Binary(data))) => self.input.on_audio(&data, self.clock.now()),
                    Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                        Ok(msg) => {
                            self.input.on_control(&msg);
                            self.roster(false).await;
                        }
                        Err(e) => tracing::warn!(error = %e, "huddle bridge: bad relay text"),
                    },
                    Some(Ok(Message::Close(frame))) => {
                        // Polling on sends tungstenite's answer to the relay's Close.
                        drain(socket).await;
                        return End::Closed(frame.map(|f| f.code.into()));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "huddle bridge: audio socket");
                        return End::Closed(None);
                    }
                    None => return End::Closed(None),
                },
                _ = wake.tick() => {
                    if let Err(e) = self.steps(&mut socket).await {
                        tracing::warn!(error = %e, "huddle bridge: audio send");
                        return End::Closed(None);
                    }
                }
            }
        }
    }

    /// Every 10 ms step now due, at its own time however late the wake: Input's tick and
    /// barge-in each step, an Output frame every second one.
    async fn steps<S>(&mut self, socket: &mut WebSocketStream<S>) -> Result<(), WsError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let due = self.clock.steps();
        if due > self.step + CATCH_UP {
            self.step = due - 1;
        }
        while self.step < due {
            self.step += 1;
            let now = self.clock.unix0 + self.step * STEP_MS;
            let outs = self.input.tick(now);
            self.emit(outs).await;
            if self.output.hear(self.input.levels(), now) {
                self.input.output_stopped(now);
            }
            if self.step.is_multiple_of(2) {
                if let Some(frame) = self.output.tick(now) {
                    socket.send(Message::Binary(frame.into())).await?;
                }
            }
            self.input.set_output_until(self.output.until());
        }
        Ok(())
    }

    /// Await `fut` while applying commands, PCM aside; `None` when the voice leaves first.
    async fn between<T>(
        &mut self,
        fut: impl Future<Output = T>,
        cmds: &mut mpsc::Receiver<Cmd>,
    ) -> Option<T> {
        let mut fut = std::pin::pin!(fut);
        loop {
            tokio::select! {
                value = &mut fut => return Some(value),
                cmd = cmds.recv() => match cmd {
                    None | Some(Cmd::Leave) => return None,
                    Some(Cmd::Pcm(_)) => {}
                    Some(cmd) => self.apply(cmd).await,
                },
            }
        }
    }

    async fn apply(&mut self, cmd: Cmd) {
        let now = self.clock.now();
        match cmd {
            Cmd::Pcm(pcm) => self.output.on_pcm(&pcm),
            Cmd::Floor { holder, token } => self.output.set_floor(holder.as_deref(), token),
            Cmd::Awake(awake) => self.input.set_awake(awake),
            Cmd::Secret { from_ms, to_ms } => self.input.add_secret(from_ms, to_ms, now),
            Cmd::Replay {
                index,
                from_ms,
                to_ms,
            } => {
                let outs = self.input.replay(index, from_ms, to_ms, now);
                self.emit(outs).await;
            }
            Cmd::Policy(policy) => self.input.set_policy(policy),
            Cmd::Route(route) => self.input.set_route(route),
            Cmd::Leave => {}
        }
    }

    /// Send Talos the roster if it changed, or `always`.
    async fn roster(&mut self, always: bool) {
        let roster = self.input.roster_frame();
        if always || self.roster.as_ref() != Some(&roster) {
            self.roster = Some(roster.clone());
            self.emit(vec![Out::Frame(roster)]).await;
        }
    }

    // `&mut`, as `&self` across an await would need Input and Output to be Sync.
    async fn emit(&mut self, outs: Vec<Out>) {
        for out in outs {
            if self.outs.send(out).await.is_err() {
                return;
            }
        }
    }
}

/// Read to the end of the close handshake, for at most 2 s.
async fn drain<S: AsyncRead + AsyncWrite + Unpin>(mut socket: WebSocketStream<S>) {
    let _ = tokio::time::timeout(CLOSE_WAIT, async {
        while let Some(Ok(_)) = socket.next().await {}
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Tuning;
    use crate::output::Barge;
    use tokio::io::DuplexStream;
    use tokio::time::timeout;
    use tokio_tungstenite::tungstenite::protocol::Role;

    const ME: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const VERSION: &str = "this huddle is using audio protocol v1; your client requested v2";

    type Socket = WebSocketStream<DuplexStream>;
    type Dialed = Result<(Socket, Vec<Peer>), String>;

    /// The relay side's next message; a closed socket fails the test instead of spinning.
    async fn read(relay: &mut Socket) -> Message {
        relay.next().await.expect("closed").expect("socket error")
    }

    async fn pair() -> (Socket, Socket) {
        let (a, b) = tokio::io::duplex(1 << 16);
        let client = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
        (
            client,
            WebSocketStream::from_raw_socket(b, Role::Server, None).await,
        )
    }

    /// A bridge whose dials answer from `dials`, last first, each stamped with its time. One dial
    /// too many panics, so a retry loop fails the test instead of hanging it.
    async fn bridge(
        mut dials: Vec<Dialed>,
        cmds: &mut mpsc::Receiver<Cmd>,
        outs: &mpsc::Sender<Out>,
    ) -> (End, Vec<Instant>) {
        let mut input = Input::new(Policy::default(), Tuning::default());
        let mut output = Output::new(ME.into(), Barge::default()).unwrap();
        let mut at = Vec::new();
        let dial = || {
            at.push(Instant::now());
            let next = dials.pop().expect("dialed past the end");
            async move { next }
        };
        let end = run(dial, &mut input, &mut output, cmds, outs).await;
        (end, at)
    }

    #[test]
    fn backoff_spreads_a_restart_then_doubles_to_30_s_and_a_version_refusal_is_final() {
        let mut b = Backoff::default();
        let s = Duration::from_secs;
        let failed = End::Refused("audio WS connect failed".into());
        assert_eq!(
            b.next(&End::Closed(Some(1012)), s(60), 0.25),
            Some(s(10) / 4)
        );
        let ladder: Vec<_> = (0..7)
            .map(|_| b.next(&failed, s(0), 0.5).unwrap())
            .collect();
        assert_eq!(ladder, [1, 2, 4, 8, 16, 30, 30].map(s));
        // A 1001 mid-ladder doesn't spread; after a stable socket the ladder restarts.
        assert_eq!(b.next(&End::Closed(Some(1001)), s(5), 1.0), Some(s(30)));
        assert_eq!(b.next(&End::Closed(Some(1001)), s(30), 1.0), Some(s(10)));
        assert_eq!(b.next(&End::Closed(None), s(30), 0.5), Some(s(1)));
        let draining = End::Refused("relay is draining; reconnect".into());
        assert_eq!(Backoff::default().next(&draining, s(0), 0.5), Some(s(5)));
        assert_eq!(b.next(&End::Refused(VERSION.into()), s(0), 0.5), None);
        let unsupported = "huddle audio protocol v3 not supported; relay max is v2";
        assert_eq!(b.next(&End::Refused(unsupported.into()), s(0), 0.5), None);
        assert_eq!(b.next(&End::Left, s(0), 0.5), None);
        assert!((0..100).map(|_| unit()).all(|u| (0.0..1.0).contains(&u)));
    }

    #[tokio::test(start_paused = true)]
    async fn output_is_paced_20_ms_apart_and_leave_closes_with_1000() {
        let (client, mut relay) = pair().await;
        let (cmd, mut cmds) = mpsc::channel(64);
        let (outs, mut talos) = mpsc::channel(1024);
        let room = async {
            let end = bridge(vec![Ok((client, Vec::new()))], &mut cmds, &outs).await;
            assert_eq!(end.0, End::Left);
        };
        let check = async {
            // The join sends a roster, and a relay `joined` another.
            let roster = |humans: &[&str]| {
                let humans = humans.iter().map(|pk| pk.to_string()).collect();
                Some(Out::Frame(ToTalos::Roster {
                    humans,
                    voices: Vec::new(),
                }))
            };
            let second = Duration::from_secs(1);
            assert_eq!(timeout(second, talos.recv()).await.unwrap(), roster(&[]));
            let peer = serde_json::json!({"pubkey": ME, "peer_index": 1});
            let joined = serde_json::json!({"type": "joined", "peers": [peer]});
            relay
                .send(Message::Text(joined.to_string().into()))
                .await
                .unwrap();
            let next = timeout(second, talos.recv()).await;
            assert_eq!(next.expect("a roster"), roster(&[ME]));
            // The voice gets the floor and 200 ms.
            let floor = Cmd::Floor {
                holder: Some(ME.into()),
                token: 1,
            };
            cmd.send(floor).await.unwrap();
            cmd.send(Cmd::Pcm(vec![0x40; 9_600])).await.unwrap();
            let mut at = Vec::new();
            while at.len() < 10 {
                match read(&mut relay).await {
                    Message::Binary(_) => at.push(Instant::now()),
                    other => panic!("{other:?}"),
                }
            }
            for gap in at.windows(2).map(|w| w[1] - w[0]) {
                assert_eq!(gap, Duration::from_millis(20));
            }
            cmd.send(Cmd::Leave).await.unwrap();
            let Message::Close(close) = read(&mut relay).await else {
                panic!("not a Close");
            };
            assert_eq!(close.map(|f| f.code), Some(CloseCode::Normal));
            assert!(relay.next().await.is_none(), "the handshake completes");
        };
        tokio::join!(room, check);
    }

    #[tokio::test(start_paused = true)]
    async fn a_1012_rejoins_within_the_spread_and_a_version_refusal_ends_it() {
        let (client, mut relay) = pair().await;
        let (_cmd, mut cmds) = mpsc::channel(64);
        let (outs, _talos) = mpsc::channel(1024);
        let dials = vec![Err(VERSION.into()), Ok((client, Vec::new()))];
        let room = async {
            let (end, at) = bridge(dials, &mut cmds, &outs).await;
            assert_eq!(end, End::Refused(VERSION.into()));
            assert_eq!(at.len(), 2, "no retry after the refusal");
            assert!(at[1] - at[0] <= SPREAD);
        };
        let restart = async {
            let frame = CloseFrame {
                code: CloseCode::Restart,
                reason: "".into(),
            };
            relay.close(Some(frame)).await.unwrap();
            while let Some(Ok(_)) = relay.next().await {}
        };
        tokio::join!(room, restart);
    }

    /// 8a spawns the room on buzz-acp's runtime, so its future must be `Send`.
    #[allow(dead_code)]
    fn spawnable(
        input: &'static mut Input,
        output: &'static mut Output,
        cmds: &'static mut mpsc::Receiver<Cmd>,
        outs: &'static mpsc::Sender<Out>,
    ) {
        fn send<T: Send>(_: T) {}
        send(run(
            || async { Err::<(Socket, Vec<Peer>), String>("x".into()) },
            input,
            output,
            cmds,
            outs,
        ));
    }
}
