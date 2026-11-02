//! One run against a fake relay (`POST /query`, the audio socket) and a fake Talos on loopback
//! sockets, speaking what the bridge's 3c loopback speaks. No network, no OpenAI. Real time, as the
//! sockets are real, so timings here are bounds.

use super::*;
use axum::{extract::State, routing, Json, Router};
use base64::Engine;
use nostr::{EventBuilder, Keys, Kind};
use serde_json::Value;
use std::sync::Mutex;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::{accept_async, accept_hdr_async};

const VERSION: &str = "this huddle is using audio protocol v1; your client requested v2";

fn say(frame: Value) -> Message {
    Message::Text(frame.to_string().into())
}

/// What the fakes hold and saw. `log` keeps each thing the sockets did, the first time, in order.
#[derive(Clone, Default)]
struct Seen {
    log: Arc<Mutex<Vec<String>>>,
    talos_auth: Arc<Mutex<String>>,
    open: Arc<Mutex<Value>>,
    audio_auth: Arc<Mutex<Value>>,
    /// Control frames for the audio socket to send.
    room: Arc<Mutex<Vec<Value>>>,
    record: Arc<Mutex<Vec<Event>>>,
}

impl Seen {
    fn note(&self, what: &str) {
        let mut log = self.log.lock().unwrap();
        if !log.iter().any(|seen| seen == what) {
            log.push(what.to_owned());
        }
    }

    fn saw(&self, what: &str) -> bool {
        self.log.lock().unwrap().iter().any(|seen| seen == what)
    }
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    (listener, address)
}

async fn fake_rest(seen: Seen) -> String {
    let query = |State(seen): State<Seen>| async move {
        let records = seen.record.lock().unwrap().clone();
        Json(records)
    };
    let app = Router::new()
        .route("/query", routing::post(query))
        .with_state(seen);
    let (listener, address) = listen().await;
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}")
}

/// Challenge, the agent's auth, `joined` with the owner in the room or the `refuse` error, then
/// what the voice plays, and the frames tests queue in `room`.
async fn fake_audio(seen: Seen, owner: String, refuse: Option<&'static str>) -> String {
    let (listener, address) = listen().await;
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        let challenge = json!({"type": "challenge", "challenge": "c-1"});
        ws.send(say(challenge)).await.unwrap();
        let Some(Ok(Message::Text(auth))) = ws.next().await else {
            return;
        };
        *seen.audio_auth.lock().unwrap() = serde_json::from_str(&auth).unwrap();
        seen.note("audio:join");
        if let Some(message) = refuse {
            let error = json!({"type": "error", "message": message});
            ws.send(say(error)).await.unwrap();
            let _ = ws.close(None).await;
            return;
        }
        let peers = json!([{"pubkey": owner, "peer_index": 0}]);
        let joined = json!({"type": "joined", "peer_index": 1, "peers": peers});
        ws.send(say(joined)).await.unwrap();
        loop {
            tokio::select! {
                message = ws.next() => match message {
                    Some(Ok(Message::Binary(_))) => seen.note("audio:frame"),
                    Some(Ok(Message::Close(Some(c)))) if u16::from(c.code) == 1000 => {
                        seen.note("audio:close");
                    }
                    Some(Ok(_)) => {}
                    _ => return,
                },
                _ = sleep(Duration::from_millis(10)) => {
                    let queued = std::mem::take(&mut *seen.room.lock().unwrap());
                    for frame in queued {
                        ws.send(say(frame)).await.unwrap();
                    }
                }
            }
        }
    });
    format!("ws://{address}")
}

/// Answers `open` with `refusal`, or `opened`. Once a roster shows audio is joined it sends the
/// floor, a burst of voice, a ping, a tool and a reseed; after the answers it sends `end`, or
/// drops the socket if that's `"drop"`.
async fn fake_talos(
    seen: Seen,
    agent: String,
    refusal: Option<&'static str>,
    end: Value,
) -> String {
    let (listener, address) = listen().await;
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        #[allow(clippy::result_large_err)]
        let header = |request: &Request, response: Response| {
            let auth = request.headers()["Authorization"].to_str().unwrap();
            *seen.talos_auth.lock().unwrap() = auth.to_owned();
            Ok(response)
        };
        let mut ws = accept_hdr_async(stream, header).await.unwrap();
        let Some(Ok(Message::Text(open))) = ws.next().await else {
            return;
        };
        *seen.open.lock().unwrap() = serde_json::from_str(&open).unwrap();
        seen.note("talos:open");
        if let Some(reason) = refusal {
            let close = json!({"v": 1, "type": "close", "reason": reason});
            ws.send(say(close)).await.unwrap();
            return;
        }
        let opened = json!({"v": 1, "type": "opened", "lease_id": "l", "expires_at": 1});
        ws.send(say(opened)).await.unwrap();
        while let Some(Ok(message)) = ws.next().await {
            let Message::Text(frame) = message else {
                continue;
            };
            match frames::parse(&frame).unwrap() {
                ToTalos::Roster { .. } if !seen.saw("talos:roster") => {
                    seen.note("talos:roster");
                    let script = [
                        say(json!({"v": 1, "type": "floor", "holder": agent, "token": 1})),
                        Message::Binary(vec![0x40; 9_600].into()),
                        say(json!({"v": 1, "type": "ping", "t": 7})),
                        say(json!({"v": 1, "type": "tool", "id": "c", "name": "n", "args": {}})),
                        say(json!({"v": 1, "type": "reseed", "reason": "r"})),
                    ];
                    for message in script {
                        ws.send(message).await.unwrap();
                    }
                }
                ToTalos::Pong { t: 7 } => seen.note("talos:pong"),
                ToTalos::ToolResult { ok: false, .. } => seen.note("talos:refused"),
                ToTalos::Seed(_) => {
                    // The last answer: now whatever ends this run.
                    seen.note("talos:seed");
                    if end == "drop" {
                        return;
                    }
                    if !end.is_null() {
                        ws.send(say(end.clone())).await.unwrap();
                    }
                }
                _ => {}
            }
        }
        seen.note("talos:closed");
    });
    format!("ws://{address}")
}

struct Run {
    seen: Seen,
    voicing: Arc<Voicing>,
    owner: Keys,
    agent: Keys,
    huddle: Uuid,
    parent: Uuid,
    added: Event,
}

impl Run {
    async fn new(refusal: Option<&'static str>, refuse: Option<&'static str>, end: Value) -> Self {
        let (seen, owner, agent) = (Seen::default(), Keys::generate(), Keys::generate());
        let (owner_hex, agent_hex) = (owner.public_key().to_hex(), agent.public_key().to_hex());
        let record = EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), "");
        let record = record.tags([Tag::parse(["d", &agent_hex]).unwrap()]);
        seen.record
            .lock()
            .unwrap()
            .push(record.sign_with_keys(&owner).unwrap());
        let voicing = Voicing {
            rest: RestClient {
                http: reqwest::Client::new(),
                base_url: fake_rest(seen.clone()).await,
                keys: agent.clone(),
                auth_tag_json: None,
            },
            agent: agent_hex.clone(),
            owner: owner_hex.clone(),
            auth_tag: Some(Tag::parse(["auth", &owner_hex, "", "sig"]).unwrap()),
            relay_url: fake_audio(seen.clone(), owner_hex, refuse).await,
            talos_url: fake_talos(seen.clone(), agent_hex, refusal, end).await,
            profile: Profile {
                voice: "marin".into(),
                persona: String::new(),
                aliases: Vec::new(),
            },
            owner_grace: Duration::from_millis(300),
        };
        let added = EventBuilder::new(Kind::Custom(44100), "").sign_with_keys(&Keys::generate());
        Self {
            seen,
            voicing: Arc::new(voicing),
            owner,
            agent,
            huddle: Uuid::new_v4(),
            parent: Uuid::new_v4(),
            added: added.unwrap(),
        }
    }

    /// The run on its own task; dropping or sending on the channel is `stop`.
    fn start(&self) -> (oneshot::Sender<()>, tokio::task::JoinHandle<Left>) {
        let (stop, stopped) = oneshot::channel::<()>();
        let (voicing, huddle) = (self.voicing.clone(), self.huddle);
        let (parent, added) = (self.parent, self.added.clone());
        let run = tokio::spawn(async move {
            let stopped = async {
                let _ = stopped.await;
            };
            session(&voicing, huddle, parent, &added, stopped).await
        });
        (stop, run)
    }
}

async fn until(what: &str, done: impl Fn() -> bool) {
    let wait = async {
        while !done() {
            sleep(Duration::from_millis(5)).await;
        }
    };
    let waited = timeout(Duration::from_secs(20), wait).await;
    waited.unwrap_or_else(|_| panic!("timed out on {what}"));
}

async fn left(run: tokio::task::JoinHandle<Left>) -> Left {
    timeout(Duration::from_secs(20), run)
        .await
        .expect("the run never ended")
        .unwrap()
}

#[tokio::test]
async fn talos_opens_before_audio_the_frames_flow_and_stop_closes_both_cleanly() {
    let r = Run::new(None, None, Value::Null).await;
    let (stop, run) = r.start();
    until("played and answered", || {
        ["audio:frame", "talos:pong", "talos:refused", "talos:seed"]
            .iter()
            .all(|s| r.seen.saw(s))
    })
    .await;
    assert_eq!(
        r.seen.log.lock().unwrap()[..2],
        ["talos:open", "audio:join"]
    );

    let open = r.seen.open.lock().unwrap().clone();
    let agent = r.agent.public_key().to_hex();
    assert_eq!(
        (&open["huddle"], &open["agent"]),
        (&json!(r.huddle), &json!(agent))
    );
    assert_eq!(open["profile"]["voice"], "marin");
    assert_eq!(open["proofs"]["added"]["id"], r.added.id.to_hex());
    assert_eq!(open["proofs"]["agent_record"]["kind"], KIND_MANAGED_AGENT);
    assert_eq!(
        open["proofs"]["agent_record"]["pubkey"],
        r.owner.public_key().to_hex()
    );
    let auth = r.seen.talos_auth.lock().unwrap().clone();
    let auth = auth.strip_prefix("Nostr ").unwrap();
    let auth = base64::engine::general_purpose::STANDARD
        .decode(auth)
        .unwrap();
    let auth: Event = serde_json::from_slice(&auth).unwrap();
    auth.verify().unwrap();
    assert_eq!(auth.pubkey.to_hex(), agent);
    let tags: Vec<_> = auth.tags.iter().map(|t| t.clone().to_vec()).collect();
    assert!(tags.contains(&vec!["method".to_owned(), "GET".to_owned()]));
    let url = format!(
        "{}/df/voice?huddle={}&agent={agent}",
        r.voicing.talos_url, r.huddle
    );
    assert!(tags.contains(&vec!["u".to_owned(), url]));
    let audio = r.seen.audio_auth.lock().unwrap().clone();
    assert_eq!(audio["parent_channel_id"], r.parent.to_string());
    assert_eq!(audio["event"]["pubkey"], agent);
    assert!(
        audio["event"]["tags"].to_string().contains("auth"),
        "the NIP-OA tag"
    );

    stop.send(()).unwrap();
    assert_eq!(left(run).await, Left::Done("stopped".into()));
    until("Close 1000", || r.seen.saw("audio:close")).await;
    until("Talos closed", || r.seen.saw("talos:closed")).await;
}

#[tokio::test]
async fn a_second_holder_or_no_record_never_joins_audio() {
    let r = Run::new(Some("duplicate_holder"), None, Value::Null).await;
    let (_stop, run) = r.start();
    assert_eq!(left(run).await, Left::NoLease("duplicate_holder".into()));
    assert!(r.seen.saw("talos:open") && !r.seen.saw("audio:join"));

    let r = Run::new(None, None, Value::Null).await;
    r.seen.record.lock().unwrap().clear();
    let (_stop, run) = r.start();
    assert!(matches!(left(run).await, Left::NoLease(why) if why.contains("30177")));
    assert!(r.seen.log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn talos_closing_or_dropping_leaves_the_room_with_a_clean_close() {
    let close = json!({"v": 1, "type": "close", "reason": "idle"});
    let r = Run::new(None, None, close).await;
    let (_stop, run) = r.start();
    assert_eq!(left(run).await, Left::Done("idle".into()));
    until("Close 1000", || r.seen.saw("audio:close")).await;

    let r = Run::new(None, None, json!("drop")).await;
    let (_stop, run) = r.start();
    assert_eq!(left(run).await, Left::Lost);
    until("Close 1000", || r.seen.saw("audio:close")).await;
}

#[tokio::test]
async fn the_owner_going_for_the_grace_ends_the_run_and_coming_back_cancels_it() {
    let r = Run::new(None, None, Value::Null).await;
    let owner = r.owner.public_key().to_hex();
    let (_stop, mut run) = r.start();
    until("joined", || r.seen.saw("talos:seed")).await;
    let left_room = json!({"type": "left", "peer_index": 0});
    let back = json!({"type": "joined", "peers": [{"pubkey": owner, "peer_index": 2}]});
    r.seen
        .room
        .lock()
        .unwrap()
        .extend([left_room.clone(), back]);
    let waited = timeout(Duration::from_millis(600), &mut run).await;
    assert!(waited.is_err(), "back inside the grace, so it stays");
    r.seen
        .room
        .lock()
        .unwrap()
        .push(json!({"type": "left", "peer_index": 2}));
    assert_eq!(left(run).await, Left::OwnerGone);
    until("Close 1000", || r.seen.saw("audio:close")).await;
}

#[tokio::test]
async fn a_final_relay_refusal_ends_the_run_in_the_relays_words() {
    let r = Run::new(None, Some(VERSION), Value::Null).await;
    let (_stop, run) = r.start();
    assert_eq!(left(run).await, Left::Refused(VERSION.into()));
    until("Talos closed", || r.seen.saw("talos:closed")).await;

    let r = Run::new(None, Some("huddle has ended"), Value::Null).await;
    let (_stop, run) = r.start();
    assert_eq!(left(run).await, Left::Done("the audio room ended".into()));
}

/// The bridge gets each command, the secret window above all: it stops the audio there.
#[tokio::test]
async fn talos_frames_become_the_bridges_commands() {
    let r = Run::new(None, None, Value::Null).await;
    let (cmd, mut cmds) = mpsc::channel(8);
    let mut command = |frame: Value| {
        let frame = frames::parse(&frame.to_string()).unwrap();
        assert!(on_frame(frame, &r.voicing, &cmd).is_none());
        cmds.try_recv().unwrap()
    };
    let secret = command(json!({"v": 1, "type": "secret", "from_ms": 1, "to_ms": 2}));
    assert!(matches!(
        secret,
        Cmd::Secret {
            from_ms: 1,
            to_ms: 2
        }
    ));
    let quiet = command(json!({"v": 1, "type": "quiet"}));
    assert!(matches!(quiet, Cmd::Awake(false)));
    let awake = command(json!({"v": 1, "type": "awake", "expires_at": 1}));
    assert!(matches!(awake, Cmd::Awake(true)));
}
