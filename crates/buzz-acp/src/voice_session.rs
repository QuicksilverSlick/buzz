//! One huddle voice's run in the room (design 2026-10-29 §3.3-§3.5; slice 8a, part 1 of 2):
//! `open` on `/df/voice` with NIP-98, audio joined through the bridge only after `opened`, the
//! frame client between the two, and a clean Close on the way out.
//!
//! Part 2 decides when to run one: the 44100 actor gate, the owner-present rule, the decline line
//! and kind 9, behind slice 9's switch. Nothing calls this until then.

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
use tokio::time::{sleep_until, timeout, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use crate::relay::RestClient;

const OPEN_WAIT: Duration = Duration::from_secs(10);
const SEND_WAIT: Duration = Duration::from_secs(5);
/// Long enough for the bridge's Close and drain, 2 s each.
const LEAVE_WAIT: Duration = Duration::from_secs(5);

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
    /// How long the voice stays after the owner's last peer goes (60 s).
    pub owner_grace: Duration,
}

/// How a run ended.
#[derive(Debug, PartialEq)]
pub(crate) enum Left {
    /// Never in the room: Talos's refusal reason (`duplicate_holder` when another harness holds
    /// this voice), or what failed on the way.
    NoLease(String),
    /// The owner's last peer left the room, `owner_grace` ago.
    OwnerGone,
    /// `stop` fired, Talos closed the voice (its reason), or the huddle ended.
    Done(String),
    /// The Talos socket dropped. 8e resumes within the lease's 20 s.
    Lost,
    /// The relay refused the audio join for good, in its words.
    Refused(String),
}

fn text(error: impl std::fmt::Display) -> String {
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

/// Open the lease; `Err` carries Talos's refusal reason, or what failed on the way.
async fn open(me: &Voicing, huddle: Uuid, added: &Event) -> Result<Talos, String> {
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
    let dial = timeout(OPEN_WAIT, connect_async(request)).await;
    let (mut talos, _) = dial.map_err(text)?.map_err(text)?;
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
    let open = Message::Text(frames::to_text(&open).into());
    talos.send(open).await.map_err(text)?;
    let answer = async {
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
    };
    timeout(OPEN_WAIT, answer).await.map_err(text)??;
    Ok(talos)
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
    let opened = tokio::select! {
        _ = &mut stop => return Left::Done("stopped".into()),
        opened = open(me, huddle, added) => opened,
    };
    let mut talos = match opened {
        Ok(talos) => talos,
        Err(why) => return Left::NoLease(why),
    };
    // The pump never waits on the bridge, which may be waiting on the pump.
    let (cmd, cmds) = mpsc::channel(256);
    let (outs, mut from_room) = mpsc::channel(1_024);
    let bridge = tokio::spawn(room(me.clone(), huddle, parent, cmds, outs));
    // `opened` carries `expires_at`: the voice is live from here.
    let _ = cmd.try_send(Cmd::Awake(true));
    let mut owner_gone = None;
    let left = loop {
        let gone_at = owner_gone.unwrap_or_else(Instant::now);
        let message = tokio::select! {
            _ = &mut stop => break Left::Done("stopped".into()),
            _ = sleep_until(gone_at), if owner_gone.is_some() => break Left::OwnerGone,
            message = talos.next() => match message {
                // The bridge holds 320 ms and drops the rest.
                Some(Ok(Message::Binary(pcm))) => {
                    let _ = cmd.try_send(Cmd::Pcm(pcm.into()));
                    continue;
                }
                Some(Ok(Message::Text(frame))) => match frames::parse(&frame) {
                    Ok(ToPc::Close { reason }) => break Left::Done(reason),
                    Ok(frame) => match on_frame(frame, me, &cmd) {
                        Some(answer) => Message::Text(frames::to_text(&answer).into()),
                        None => continue,
                    },
                    Err(why) => {
                        tracing::warn!(%huddle, %why, "huddle voice: a bad frame");
                        continue;
                    }
                },
                Some(Ok(Message::Close(_)) | Err(_)) | None => break Left::Lost,
                Some(Ok(_)) => continue,
            },
            out = from_room.recv() => match out {
                Some(Out::Pcm(pcm)) => Message::Binary(pcm.into()),
                Some(Out::Frame(frame)) => {
                    if let ToTalos::Roster { humans, .. } = &frame {
                        let here = humans.iter().any(|h| h.eq_ignore_ascii_case(&me.owner));
                        let grace = Instant::now() + me.owner_grace;
                        owner_gone = (!here).then(|| owner_gone.unwrap_or(grace));
                    }
                    Message::Text(frames::to_text(&frame).into())
                }
                // The bridge ended; what it returned, below, says why.
                None => break Left::Done("the audio room ended".into()),
            },
        };
        if !matches!(timeout(SEND_WAIT, talos.send(message)).await, Ok(Ok(()))) {
            break Left::Lost;
        }
    };
    // A full queue must not hold up the Close.
    drop(from_room);
    let _ = cmd.send(Cmd::Leave).await;
    let end = timeout(LEAVE_WAIT, bridge).await;
    let _ = timeout(LEAVE_WAIT, talos.close(None)).await;
    match end {
        Ok(Ok(End::Refused(why))) if !why.contains("huddle has ended") => Left::Refused(why),
        _ => left,
    }
}

/// One frame from Talos, past `close`, and what to answer. Never log frame text.
fn on_frame(frame: ToPc, me: &Voicing, cmd: &mpsc::Sender<Cmd>) -> Option<ToTalos> {
    let command = match frame {
        ToPc::Floor { holder, token } => Cmd::Floor { holder, token },
        ToPc::Awake { .. } => Cmd::Awake(true),
        ToPc::Quiet {} => Cmd::Awake(false),
        ToPc::Secret { from_ms, to_ms } => Cmd::Secret { from_ms, to_ms },
        ToPc::Ping { t } => return Some(ToTalos::Pong { t }),
        ToPc::Reseed { .. } => return Some(ToTalos::Seed(seed(me))),
        // 8c runs tools, under its authority and the room rule.
        ToPc::Tool { id, .. } => {
            let (ok, text) = (false, "This voice's tools aren't connected yet.".to_owned());
            return Some(ToTalos::ToolResult { id, ok, text });
        }
        // 8c posts these; until then the log is the alarm.
        ToPc::Notice { kind, .. } => {
            tracing::warn!(?kind, "huddle voice: a notice with nowhere to go yet");
            return None;
        }
        // 8c: transcript, elevated, summary. 10b: meter. T5b: replay. Talos owns the mute.
        _ => return None,
    };
    if let Err(error) = cmd.try_send(command) {
        tracing::warn!(%error, "huddle voice: the bridge's queue refused a command");
    }
    None
}

/// The bridge for one run; it rejoins on its own. Until part 2's peer policy, only the owner is
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
        let request = audio_request(&me.relay_url, huddle)?;
        let joined = join(request, parent, &me.rest.keys, me.auth_tag.as_ref()).await?;
        Ok((joined.socket, joined.peers))
    };
    run(dial, &mut input, &mut output, &mut cmds, &outs).await
}

#[cfg(test)]
mod tests;
