//! One huddle voice's run in the room (design 2026-10-29 §3.3-§3.5; slice 8a): `open` on
//! `/df/voice` with NIP-98, audio joined through the bridge only after `opened`, the frame client
//! between the two, and a clean Close on the way out.
//!
//! The `voice` module decides when to run one: the 44100 actor gate, the owner-present rule, the
//! decline line and kind 9, behind slice 9's switch. `session` does no check on `added`.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use buzz_core::kind::KIND_MANAGED_AGENT;
use buzz_huddle_bridge::frames::{self, Profile, Proofs, Seed, ToPc, ToTalos};
use buzz_huddle_bridge::input::{Input, Out, Policy, Tuning};
use buzz_huddle_bridge::join::{audio_request, join};
use buzz_huddle_bridge::link::{run, Cmd, End};
use buzz_huddle_bridge::output::{Barge, Output};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, Tag};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinError;
use tokio::time::{sleep_until, timeout, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use crate::relay::RestClient;

const OPEN_WAIT: Duration = Duration::from_secs(10);
const SEND_WAIT: Duration = Duration::from_secs(5);
/// Long enough for the bridge's Close and drain, 2 s each.
const LEAVE_WAIT: Duration = Duration::from_secs(5);
/// Pump to bridge. Sized for one voice at Talos's real-time pace: about 5 s of 20 ms chunks, far
/// past the 320 ms the bridge keeps, so a queue this full means the bridge has stalled.
const CMD_SLOTS: usize = 256;
/// The slots voice never takes, so a `secret` still gets in behind a burst of it.
const CONTROL_SLOTS: usize = 16;
/// Bridge to pump: about 20 s of room audio at link's 20 ms chunks. When it fills, the whole
/// bridge waits, so its alarm fires at half.
const ROOM_SLOTS: usize = 1_024;

type Talos = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// What every run of one agent's voice needs.
pub(crate) struct Voicing {
    pub rest: RestClient,
    pub agent: String,
    pub owner: String,
    pub auth_tag: Option<Tag>,
    pub relay_url: String,
    /// Pinned by the harness's config: the host never comes from a frame.
    pub talos_url: String,
    pub profile: Profile,
    /// How long the voice stays with nothing showing the owner in the room (60 s).
    pub owner_grace: Duration,
}

/// How a run ended. The strings are remote text: never post them as they are.
#[derive(Debug, PartialEq)]
pub(crate) enum Left {
    /// Never in the room: Talos's refusal reason (`duplicate_holder` when another harness holds
    /// this voice), or what failed on the way.
    NoLease(String),
    /// Nothing showed the owner in the room for `owner_grace`: their last peer left, or the audio
    /// join kept failing.
    OwnerGone,
    /// `stop` fired, Talos closed the voice (its reason), the huddle ended, or the bridge couldn't
    /// take a command.
    Done(String),
    /// The Talos socket dropped or sent a frame over `frames::MAX_FRAME`, which 8e resumes within
    /// the lease's 20 s, or the bridge panicked.
    Lost,
    /// The relay refused the audio join for good, in its words.
    Refused(String),
}

pub(crate) fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Nothing seeded yet (8e builds it), for the only human this PC knows is here at open.
fn seed(me: &Voicing) -> Seed {
    let intersection_hash = hex::encode(Sha256::digest(&me.owner));
    let input = Vec::new();
    Seed {
        intersection_hash,
        input,
    }
}

/// The socket to Talos, and the `open` frame to send on it.
async fn dial(me: &Voicing, huddle: Uuid, added: &Event) -> Result<(Talos, ToTalos), String> {
    let agent = &me.agent;
    let record = json!({
        "kinds": [KIND_MANAGED_AGENT], "authors": [me.owner], "#d": [agent], "limit": 1,
    });
    let records = me.rest.query_raw(&[record]).await.map_err(text)?;
    let agent_record = records.get(0).cloned();
    let agent_record = agent_record.ok_or("the owner has no 30177 for this agent")?;
    let added = serde_json::to_value(added).map_err(text)?;
    // Signed as dialed: T3b has to rebuild this wss:// form to check `u`.
    let url = format!("{}/df/voice?huddle={huddle}&agent={agent}", me.talos_url);
    let mut request = url.as_str().into_client_request().map_err(text)?;
    let auth = me.rest.nip98_header("GET", &url, None).map_err(text)?;
    let auth = auth.parse().map_err(text)?;
    request.headers_mut().insert("Authorization", auth);
    let config = WebSocketConfig::default()
        .max_message_size(Some(frames::MAX_FRAME))
        .max_frame_size(Some(frames::MAX_FRAME));
    let connect = connect_async_with_config(request, Some(config), false);
    let (talos, _) = timeout(OPEN_WAIT, connect)
        .await
        .map_err(text)?
        .map_err(text)?;
    let open = ToTalos::Open {
        huddle: huddle.to_string(),
        agent: agent.clone(),
        proofs: Proofs {
            agent_record,
            added,
        },
        seed: seed(me),
        profile: me.profile.clone(),
    };
    Ok((talos, open))
}

/// Ask for the lease; `Err` carries Talos's refusal reason, or what failed on the way.
async fn open(talos: &mut Talos, frame: ToTalos) -> Result<(), String> {
    let frame = Message::Text(frames::to_text(&frame).into());
    talos.send(frame).await.map_err(text)?;
    while let Some(Ok(message)) = talos.next().await {
        let Message::Text(frame) = message else {
            continue;
        };
        match frames::parse(&frame) {
            Ok(ToPc::Opened { .. }) => return Ok(()),
            Ok(ToPc::Close { reason }) => return Err(reason),
            _ => {}
        }
    }
    Err("the socket closed before `opened`".to_owned())
}

/// One run: open, join, pump, leave. `stop` ends it with a clean Close on both sockets.
pub(crate) async fn session(
    me: &Arc<Voicing>,
    huddle: Uuid,
    parent: Uuid,
    added: &Event,
    stop: impl Future<Output = ()>,
) -> Left {
    tokio::pin!(stop);
    let dialed = tokio::select! {
        _ = &mut stop => return Left::Done("stopped".into()),
        dialed = dial(me, huddle, added) => dialed,
    };
    let (mut talos, frame) = match dialed {
        Ok(dialed) => dialed,
        Err(why) => return Left::NoLease(why),
    };
    // Talos may hold a lease from here. A bare drop keeps it, billed, for 20 s, and a retry in that
    // time meets `duplicate_holder`, so every way out sends a Close.
    let refused = tokio::select! {
        _ = &mut stop => Some(Left::Done("stopped".into())),
        opened = timeout(OPEN_WAIT, open(&mut talos, frame)) => match opened {
            Ok(Ok(())) => None,
            Ok(Err(why)) => Some(Left::NoLease(why)),
            Err(late) => Some(Left::NoLease(text(late))),
        },
    };
    if let Some(left) = refused {
        let _ = timeout(LEAVE_WAIT, talos.close(None)).await;
        return left;
    }
    // The pump never waits on the bridge, which may be waiting on the pump.
    let (cmd, cmds) = mpsc::channel(CMD_SLOTS);
    let (outs, mut from_room) = mpsc::channel(ROOM_SLOTS);
    let bridge = tokio::spawn(room(me.clone(), huddle, parent, cmds, outs));
    // `opened` carries `expires_at`: the voice is live from here.
    let _ = cmd.try_send(Cmd::Awake(true));
    let left = pump(me, huddle, &mut talos, &cmd, &mut from_room, &mut stop).await;
    // A full queue must not hold up the Close.
    drop(from_room);
    let leave = async {
        let _ = cmd.send(Cmd::Leave).await;
        bridge.await
    };
    // Both at once: a relay slow to close must not keep Talos billing the lease.
    let (end, _) = tokio::join!(
        timeout(LEAVE_WAIT, leave),
        timeout(LEAVE_WAIT, talos.close(None))
    );
    how_it_ended(end.ok(), left)
}

/// How a run ended, now the bridge has had its say.
fn how_it_ended(bridge: Option<Result<End, JoinError>>, left: Left) -> Left {
    match bridge {
        Some(Ok(End::Refused(why))) if !why.contains("huddle has ended") => Left::Refused(why),
        // It panicked: the room is gone, but the pump's "room ended" would read as a clean end.
        Some(Err(_)) => Left::Lost,
        _ => left,
    }
}

/// Between Talos's socket and the bridge until the run ends, and why it did.
async fn pump(
    me: &Voicing,
    huddle: Uuid,
    talos: &mut Talos,
    cmd: &mpsc::Sender<Cmd>,
    from_room: &mut mpsc::Receiver<Out>,
    mut stop: impl Future<Output = ()> + Unpin,
) -> Left {
    // Armed before any roster: a join that keeps failing must not hold the lease open.
    let mut owner_gone = Some(Instant::now() + me.owner_grace);
    let (mut voice_warned, mut room_warned) = (false, false);
    loop {
        let gone_at = owner_gone.unwrap_or_else(Instant::now);
        let message = tokio::select! {
            _ = &mut stop => return Left::Done("stopped".into()),
            _ = sleep_until(gone_at), if owner_gone.is_some() => return Left::OwnerGone,
            message = talos.next() => match message {
                Some(Ok(Message::Binary(pcm))) => {
                    // Only this pump sends, so the room seen here is still there for the send.
                    if cmd.capacity() > CONTROL_SLOTS {
                        // The bridge holds 320 ms and drops the rest.
                        let _ = cmd.try_send(Cmd::Pcm(pcm.into()));
                    } else if !std::mem::replace(&mut voice_warned, true) {
                        tracing::warn!(
                            %huddle,
                            "huddle voice: the bridge stalled, dropping the voice's audio"
                        );
                    }
                    continue;
                }
                Some(Ok(Message::Text(frame))) => match frames::parse(&frame) {
                    Ok(ToPc::Close { reason }) => return Left::Done(reason),
                    Ok(frame) => match on_frame(frame, me, cmd) {
                        Ok(Some(answer)) => Message::Text(frames::to_text(&answer).into()),
                        Ok(None) => continue,
                        Err(()) => return Left::Done("the bridge stopped taking commands".into()),
                    },
                    // Serde's error quotes the frame.
                    Err(_) => {
                        tracing::warn!(%huddle, len = frame.len(), "huddle voice: a bad frame");
                        continue;
                    }
                },
                Some(Ok(Message::Close(_)) | Err(_)) | None => return Left::Lost,
                Some(Ok(_)) => continue,
            },
            out = from_room.recv() => {
                if from_room.len() > ROOM_SLOTS / 2 && !std::mem::replace(&mut room_warned, true) {
                    tracing::warn!(
                        %huddle,
                        "huddle voice: Talos's socket is falling behind; the bridge stalls next"
                    );
                }
                match out {
                    Some(Out::Pcm(pcm)) => Message::Binary(pcm.into()),
                    Some(Out::Frame(frame)) => {
                        if let ToTalos::Roster { humans, .. } = &frame {
                            let here = humans.iter().any(|h| h.eq_ignore_ascii_case(&me.owner));
                            let grace = Instant::now() + me.owner_grace;
                            owner_gone = (!here).then(|| owner_gone.unwrap_or(grace));
                        }
                        Message::Text(frames::to_text(&frame).into())
                    }
                    // The bridge ended; what it returned, in `session`, says why.
                    None => return Left::Done("the audio room ended".into()),
                }
            }
        };
        if !matches!(timeout(SEND_WAIT, talos.send(message)).await, Ok(Ok(()))) {
            return Left::Lost;
        }
    }
}

/// One frame from Talos, past `close`, and what to answer. `Err` when the bridge can't take a
/// command: a voice that can't honour `secret` or `quiet` must not stay in the room. Never log
/// frame text.
fn on_frame(frame: ToPc, me: &Voicing, cmd: &mpsc::Sender<Cmd>) -> Result<Option<ToTalos>, ()> {
    let command = match frame {
        ToPc::Floor { holder, token } => Cmd::Floor { holder, token },
        ToPc::Awake { .. } => Cmd::Awake(true),
        ToPc::Quiet {} => Cmd::Awake(false),
        ToPc::Secret { from_ms, to_ms } => Cmd::Secret { from_ms, to_ms },
        ToPc::Ping { t } => return Ok(Some(ToTalos::Pong { t })),
        ToPc::Reseed { .. } => return Ok(Some(ToTalos::Seed(seed(me)))),
        // 8c runs tools, under its authority and the room rule.
        ToPc::Tool { id, .. } => {
            let (ok, text) = (false, "This voice's tools aren't connected yet.".to_owned());
            return Ok(Some(ToTalos::ToolResult { id, ok, text }));
        }
        // 8c posts these; until then the log is the alarm.
        ToPc::Notice { kind, .. } => {
            tracing::warn!(?kind, "huddle voice: a notice with nowhere to go yet");
            return Ok(None);
        }
        // 8c: transcript, elevated, summary. 10b: meter. T5b: replay. Talos owns the mute.
        _ => return Ok(None),
    };
    cmd.try_send(command).map(|()| None).map_err(|refused| {
        let command = refused.into_inner();
        tracing::warn!(
            ?command,
            "huddle voice: the bridge's queue refused a command, so the voice is leaving"
        );
    })
}

/// The bridge for one run; it rejoins on its own. Until the peer policy lands, only the owner is
/// heard: every other peer is outside `admitted`.
async fn room(
    me: Arc<Voicing>,
    huddle: Uuid,
    parent: Uuid,
    mut cmds: mpsc::Receiver<Cmd>,
    outs: mpsc::Sender<Out>,
) -> End {
    let policy = Policy {
        owner: me.owner.clone(),
        agents: HashSet::from([me.agent.clone()]),
        admitted: HashSet::new(),
    };
    let mut input = Input::new(policy, Tuning::default());
    let mut output = match Output::new(me.agent.clone(), Barge::default()) {
        Ok(output) => output,
        Err(error) => return End::Refused(error),
    };
    let dial = || async {
        let joined = async {
            let request = audio_request(&me.relay_url, huddle)?;
            join(request, parent, &me.rest.keys, me.auth_tag.as_ref()).await
        };
        match joined.await {
            Ok(joined) => Ok((joined.socket, joined.peers)),
            Err(why) => {
                // Out of the room nothing shows the owner there, so the pump's grace runs.
                tracing::warn!(%huddle, %why, "huddle voice: the audio join failed");
                let (humans, voices) = (Vec::new(), Vec::new());
                let empty = ToTalos::Roster { humans, voices };
                let _ = outs.send(Out::Frame(empty)).await;
                Err(why)
            }
        }
    };
    run(dial, &mut input, &mut output, &mut cmds, &outs).await
}

#[cfg(test)]
pub(crate) mod tests;
