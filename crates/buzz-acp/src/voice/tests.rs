//! When a voice runs, against a fake relay and the fake sockets of `voice_session`'s tests. No
//! network, no OpenAI. Real time, as the sockets are real, so timings here are bounds.

use super::*;
use crate::voice_session::tests::{fake_audio, fake_talos, listen, say, until, Seen, GRACE};
use axum::{extract::State, routing, Json, Router};
use buzz_core::kind::{KIND_MANAGED_AGENT, KIND_MEMBER_ADDED_NOTIFICATION as ADDED};
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, Timestamp};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use tokio_tungstenite::accept_async;

const NINE: u32 = KIND_STREAM_MESSAGE;
const JOINED: u32 = KIND_HUDDLE_PARTICIPANT_JOINED;
const LEFT: u32 = KIND_HUDDLE_PARTICIPANT_LEFT;
const REMOVED: u32 = KIND_MEMBER_REMOVED_NOTIFICATION;
const ONLY_OWNER: RespondTo = RespondTo::OwnerOnly;

/// What the fake relay holds and saw.
#[derive(Clone, Default)]
struct Rest {
    store: Arc<Mutex<Vec<Event>>>,
    posts: Arc<Mutex<Vec<(String, String)>>>,
}

/// `GET /`, `POST /query` by kind, and `POST /events`.
async fn fake_rest(rest: Rest, relay: String) -> String {
    let query = |State(rest): State<Rest>, Json(filters): Json<Value>| async move {
        let kinds = filters[0]["kinds"].as_array().unwrap().clone();
        let store = rest.store.lock().unwrap().clone();
        let found = store.into_iter();
        Json(
            found
                .filter(|e| kinds.contains(&e.kind.as_u16().into()))
                .collect::<Vec<_>>(),
        )
    };
    let events = |State(rest): State<Rest>, Json(event): Json<Event>| async move {
        let line = (tag(&event, "h").unwrap().to_owned(), event.content.clone());
        rest.posts.lock().unwrap().push(line);
        Json(json!({"accepted": true, "event_id": event.id.to_hex(), "message": ""}))
    };
    let info = move || async move { Json(json!({"self": relay})) };
    let app = Router::new()
        .route("/", routing::get(info))
        .route("/query", routing::post(query))
        .route("/events", routing::post(events))
        .with_state(rest);
    let (listener, address) = listen().await;
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}")
}

/// Talos for a voice that comes back: every socket gets `opened`, and is counted.
async fn counting_talos(opens: Arc<AtomicUsize>) -> String {
    let (listener, address) = listen().await;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let opens = opens.clone();
            tokio::spawn(async move {
                let mut ws = accept_async(stream).await.unwrap();
                ws.next().await;
                opens.fetch_add(1, SeqCst);
                let opened = json!({"v": 1, "type": "opened", "lease_id": "l", "expires_at": 1});
                let _ = ws.send(say(opened)).await;
                while let Some(Ok(_)) = ws.next().await {}
            });
        }
    });
    format!("ws://{address}")
}

/// Signed `later` seconds from now.
fn signed(keys: &Keys, kind: u32, content: Value, later: u64, tags: &[[&str; 2]]) -> Event {
    let tags = tags.iter().map(|t| Tag::parse(*t).unwrap());
    let at = Timestamp::from(Timestamp::now().as_secs() + later);
    let builder = EventBuilder::new(Kind::Custom(kind as u16), content.to_string());
    builder
        .tags(tags)
        .custom_created_at(at)
        .sign_with_keys(keys)
        .unwrap()
}

struct World {
    seen: Seen,
    rest: Rest,
    relay: Keys,
    owner: Keys,
    agent: Keys,
    parent: Uuid,
    huddle: Uuid,
    voice: Voice,
    _shutdown: watch::Sender<()>,
}

impl World {
    /// A huddle with its 48100, and the owner's 30177 for the agent. Talos is part 1's fake with
    /// this `refusal`; or, with `opens`, a counting one, a relay refusing audio and a short grace.
    async fn new(
        respond_to: RespondTo,
        refusal: Option<&'static str>,
        opens: Option<Arc<AtomicUsize>>,
    ) -> Self {
        let (seen, rest) = (Seen::default(), Rest::default());
        let (relay, owner, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
        let (owner_hex, agent_hex) = (owner.public_key().to_hex(), agent.public_key().to_hex());
        let (parent, huddle) = (Uuid::new_v4(), Uuid::new_v4());
        let started = json!({"ephemeral_channel_id": huddle});
        rest.store.lock().unwrap().extend([
            signed(
                &owner,
                KIND_HUDDLE_STARTED,
                started,
                0,
                &[["h", &parent.to_string()]],
            ),
            signed(
                &owner,
                KIND_MANAGED_AGENT,
                json!(""),
                0,
                &[["d", &agent_hex]],
            ),
        ]);
        let (talos_url, refuse, owner_grace) = match opens {
            Some(opens) => (counting_talos(opens).await, Some("not a member"), GRACE),
            None => {
                let talos = fake_talos(seen.clone(), agent_hex.clone(), refusal, Value::Null);
                (talos.await, None, Duration::from_secs(30))
            }
        };
        let voicing = Voicing {
            rest: RestClient {
                http: reqwest::Client::new(),
                base_url: fake_rest(rest.clone(), String::new()).await,
                keys: agent.clone(),
                auth_tag_json: None,
            },
            agent: agent_hex,
            owner: owner_hex.clone(),
            auth_tag: None,
            relay_url: fake_audio(seen.clone(), owner_hex.clone(), refuse).await,
            talos_url,
            profile: VoiceProfile::default().wire().unwrap(),
            owner_grace,
        };
        let shared = Shared {
            voicing: Arc::new(voicing),
            relay: relay.public_key().to_hex(),
            owner_cache: OwnerCache::new(Some(owner_hex)),
            respond_to,
            allowlist: HashSet::new(),
            poll: Duration::from_millis(10),
            voiced: Mutex::default(),
            told: Mutex::default(),
        };
        let (_shutdown, stop) = watch::channel(());
        let voice = Voice::new(shared, stop);
        Self {
            seen,
            rest,
            relay,
            owner,
            agent,
            parent,
            huddle,
            voice,
            _shutdown,
        }
    }

    fn owner_hex(&self) -> String {
        self.owner.public_key().to_hex()
    }

    fn agent_hex(&self) -> String {
        self.agent.public_key().to_hex()
    }

    /// A lifecycle event for the owner, as `signer` would post it.
    fn owner_event(&self, signer: &Keys, kind: u32, admission: &str, later: u64) -> Event {
        let content = json!({"ephemeral_channel_id": self.huddle, "admission_id": admission});
        let (h, p) = (self.parent.to_string(), self.owner_hex());
        signed(signer, kind, content, later, &[["h", &h], ["p", &p]])
    }

    /// The relay commits a lifecycle event for the owner.
    fn relay_says(&self, kind: u32, admission: &str, later: u64) {
        let event = self.owner_event(&self.relay, kind, admission, later);
        self.rest.store.lock().unwrap().push(event);
    }

    fn notice_by(&self, signer: &Keys, kind: u32, about: &str, actor: &str) -> Event {
        let content = json!({"channel_id": self.huddle, "actor": actor});
        let h = self.huddle.to_string();
        let tags = [["p", about], ["h", &h], ["t", "stream"]];
        signed(signer, kind, content, 0, &tags)
    }

    /// The relay's notice about this agent, handed over as the main loop does.
    fn notice(&mut self, kind: u32, actor: &str) {
        let notice = self.notice_by(&self.relay, kind, &self.agent_hex(), actor);
        self.voice.on_membership(&notice);
    }

    fn voiced(&self) -> bool {
        self.voice.ignores(NINE, self.huddle)
    }

    fn lines(&self) -> Vec<(String, String)> {
        self.rest.posts.lock().unwrap().clone()
    }

    fn line(&self, line: &str) -> Vec<(String, String)> {
        vec![(self.parent.to_string(), line.to_owned())]
    }

    fn dialed_nothing(&self) -> bool {
        self.seen.log.lock().unwrap().is_empty()
    }
}

async fn settle() {
    sleep(Duration::from_millis(150)).await;
}

async fn starts(
    profile: Option<VoiceProfile>,
    talos: &str,
    owner: Option<&str>,
    base_url: &str,
) -> bool {
    let mut config = crate::build_mcp_servers_tests::test_config();
    (config.voice_profile, config.voice_talos_url) = (profile, talos.to_owned());
    let rest = RestClient {
        // A switch that failed open must fail this test, not hang on the silent listener.
        http: reqwest::Client::builder().timeout(GRACE).build().unwrap(),
        base_url: base_url.to_owned(),
        keys: Keys::generate(),
        auth_tag_json: None,
    };
    let (_shutdown, stop) = watch::channel(());
    let voice = Voice::start(&config, owner, None, rest, stop).await;
    voice.is_some()
}

/// With the switch off nothing is spawned and nothing is dialed; the main loop's hooks are `Option`s
/// of this, so kind 9 goes on as today. A switch that's on still needs a sound setup.
#[tokio::test]
async fn the_switch_off_starts_nothing_and_a_bad_setup_stays_off() {
    let (listener, address) = listen().await;
    let watched = format!("http://{address}");
    let (wss, o) = ("wss://t.test", Some("o"));
    assert!(!starts(None, wss, o, &watched).await);
    assert!(!starts(Some(VoiceProfile::default()), wss, o, &watched).await);
    let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(accepted.is_err(), "no socket and no query");

    let relay = fake_rest(Rest::default(), "a".repeat(64)).await;
    let on = |persona: String| {
        Some(VoiceProfile {
            enabled: true,
            persona: Some(persona),
            ..VoiceProfile::default()
        })
    };
    assert!(starts(on(String::new()), wss, o, &relay).await);
    assert!(!starts(on(String::new()), wss, None, &relay).await);
    assert!(!starts(on(String::new()), "ws://t.test", o, &relay).await);
    assert!(!starts(on("x".repeat(1_001)), wss, o, &relay).await);
}

#[test]
fn talos_is_only_ever_wss() {
    assert_eq!(pinned_talos("wss://t.test/").unwrap(), "wss://t.test");
    for bad in [
        "ws://t.test",
        "https://t.test",
        "wss://",
        "wss://t.test?x=1",
        "t.test",
        "",
    ] {
        assert!(pinned_talos(bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn the_owner_arriving_opens_talos_then_audio_and_only_the_relays_removal_leaves() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("voiced", || w.voiced()).await;
    assert!(!w.voice.ignores(5, w.huddle) && !w.voice.ignores(NINE, w.parent));
    settle().await;
    assert!(w.dialed_nothing(), "nothing dials before the owner");
    assert!(w.lines().is_empty(), "an owner who added it is on the way");

    w.relay_says(JOINED, "a-1", 0);
    until("joined", || w.seen.saw("talos:roster")).await;
    assert_eq!(
        w.seen.log.lock().unwrap()[..2],
        ["talos:open", "audio:join"]
    );

    let forged = w.notice_by(&w.owner, REMOVED, &w.agent_hex(), &owner);
    w.voice.on_membership(&forged);
    settle().await;
    assert!(!w.seen.saw("audio:close") && w.voiced(), "a forged removal");

    w.notice(REMOVED, &owner);
    until("Close 1000", || w.seen.saw("audio:close")).await;
    until("Talos's Close", || w.seen.saw("talos:close")).await;
    until("the text path back", || !w.voiced()).await;
    assert!(w.lines().is_empty(), "leaving says nothing");
}

#[tokio::test]
async fn the_huddle_ending_leaves_through_stop() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("joined", || w.seen.saw("talos:roster")).await;
    w.relay_says(KIND_HUDDLE_ENDED, "", 0);
    until("Close 1000", || w.seen.saw("audio:close")).await;
    until("Talos's Close", || w.seen.saw("talos:close")).await;
    until("the text path back", || !w.voiced()).await;
    assert!(w.lines().is_empty());
}

#[tokio::test]
async fn forged_or_misaddressed_notices_start_nothing() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    let (owner, about) = (w.owner_hex(), w.agent_hex());
    let forged = w.notice_by(&w.owner, ADDED, &about, &owner);
    let someone_else = w.notice_by(&w.relay, ADDED, &owner, &owner);
    let mut tampered = w.notice_by(&w.relay, ADDED, &about, "stranger");
    tampered.content = json!({"actor": owner}).to_string();
    let h = w.huddle.to_string();
    let no_actor = signed(&w.relay, ADDED, json!({}), 0, &[["p", &about], ["h", &h]]);
    let actor = json!({"actor": owner});
    let tags = [["p", about.as_str()], ["h", h.as_str()], ["t", "dm"]];
    let not_a_huddle = signed(&w.relay, ADDED, actor, 0, &tags);
    // In this order: a notice that's accepted tracks the huddle, and the rest are then repeats.
    for notice in [not_a_huddle, forged, someone_else, tampered, no_actor] {
        w.voice.on_membership(&notice);
    }
    settle().await;
    assert!(w.dialed_nothing() && !w.voiced() && w.lines().is_empty());
}

#[tokio::test]
async fn an_actor_outside_the_gate_gets_one_line_in_the_parent_and_nothing_dials() {
    let stranger = Keys::generate();
    let actor = stranger.public_key().to_hex();
    // No kind 0 yet: a failed check, so no claim about who they are.
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    w.notice(ADDED, &actor);
    until("the line", || !w.lines().is_empty()).await;
    assert_eq!(w.lines(), w.line(LINE_OFF));
    assert!(w.dialed_nothing() && !w.voiced());

    // With one they're proven outside, and told once per huddle, however often they add it.
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    let profile = signed(&stranger, 0, json!({}), 0, &[]);
    w.rest.store.lock().unwrap().push(profile);
    w.notice(ADDED, &actor);
    until("the line", || !w.lines().is_empty()).await;
    settle().await;
    w.notice(ADDED, &actor);
    settle().await;
    assert_eq!(w.lines(), w.line(LINE_GATE));
    assert!(w.dialed_nothing() && !w.voiced());
}

#[tokio::test]
async fn anyone_else_adding_it_says_it_waits_for_the_owner_then_joins_on_their_48101() {
    let mut w = World::new(RespondTo::Anyone, None, None).await;
    w.notice(ADDED, &Keys::generate().public_key().to_hex());
    until("the line", || !w.lines().is_empty()).await;
    settle().await;
    assert_eq!(w.lines(), w.line(LINE_WAITING));
    assert!(w.dialed_nothing() && w.voiced());

    // A client can post this kind; only the relay's word counts.
    let forged = w.owner_event(&Keys::generate(), JOINED, "a-0", 0);
    w.rest.store.lock().unwrap().push(forged);
    settle().await;
    assert!(w.dialed_nothing());

    w.relay_says(JOINED, "a-1", 0);
    until("the join", || w.seen.saw("audio:join")).await;
}

/// Two harnesses for one agent make one voice, so this one neither talks nor takes kind 9.
#[tokio::test]
async fn a_second_holder_never_joins_audio_says_nothing_and_stays_voiced() {
    let mut w = World::new(ONLY_OWNER, Some("duplicate_holder"), None).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the refusal", || w.seen.saw("talos:open")).await;
    settle().await;
    assert!(!w.seen.saw("audio:join") && w.lines().is_empty() && w.voiced());
}

/// The join keeps failing with the owner still in the room: `OwnerGone`, a line that says nothing
/// about them, and no paid retry until they join again. A second run isn't told again.
#[tokio::test]
async fn after_a_failing_join_only_a_newer_48101_brings_the_voice_back() {
    let opens = Arc::new(AtomicUsize::new(0));
    let mut w = World::new(ONLY_OWNER, None, Some(opens.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the open", || opens.load(SeqCst) == 1).await;
    // Seen before the run ended, so spent with it.
    w.relay_says(JOINED, "a-2", 1);
    until("the line", || !w.lines().is_empty()).await;
    until("the text path back", || !w.voiced()).await;
    assert_eq!(w.lines(), w.line(LINE_OFF));
    settle().await;
    assert_eq!(opens.load(SeqCst), 1, "no newer 48101");

    w.relay_says(JOINED, "a-3", 5);
    until("the second open", || opens.load(SeqCst) == 2).await;
    assert!(w.voiced() && w.lines().len() == 1);
}

/// The owner really left: no line at all, kind 9 stays out, and it waits for them.
#[tokio::test]
async fn the_owner_leaving_says_nothing_and_stays_voiced() {
    let opens = Arc::new(AtomicUsize::new(0));
    let mut w = World::new(ONLY_OWNER, None, Some(opens.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the open", || opens.load(SeqCst) == 1).await;
    w.relay_says(LEFT, "a-1", 0);
    sleep(GRACE + Duration::from_secs(1)).await;
    assert!(w.lines().is_empty() && w.voiced());
    assert_eq!(opens.load(SeqCst), 1);
}

#[test]
fn the_decline_line_never_carries_remote_text_or_says_the_owner_left() {
    let secret = "my pass phrase";
    for left in [
        Left::NoLease(secret.into()),
        Left::Done(secret.into()),
        Left::Refused(secret.into()),
        Left::Lost,
        Left::OwnerGone,
    ] {
        let line = line_for(&left, true).unwrap();
        assert_eq!(line, LINE_OFF, "{left:?}");
        assert!(!line.contains(secret) && !line.contains("left"));
    }
    assert_eq!(line_for(&Left::OwnerGone, false), None);
    assert_eq!(
        line_for(&Left::NoLease("duplicate_holder".into()), true),
        None
    );
    assert_eq!(line_for(&Left::Done("stopped".into()), true), None);
}

#[tokio::test]
async fn lifecycle_counts_only_the_relays_word_on_the_owners_open_admissions() {
    let w = World::new(ONLY_OWNER, None, None).await;
    let (relay, owner) = (w.relay.public_key().to_hex(), w.owner_hex());
    let read = |events: &[Event]| read_lifecycle(events, w.huddle, &relay, &owner);
    let join = w.owner_event(&w.relay, JOINED, "a-1", 0);
    let phone = w.owner_event(&w.relay, JOINED, "a-2", 5);
    let leave = w.owner_event(&w.relay, LEFT, "a-1", 0);
    let (joined, later) = (join.created_at.as_secs(), phone.created_at.as_secs());
    assert_eq!(read(&[]), (false, None));
    assert_eq!(read(std::slice::from_ref(&join)), (false, Some(joined)));
    assert_eq!(
        read(&[leave.clone(), join.clone()]),
        (false, None),
        "a same-second leave"
    );
    assert_eq!(
        read(&[join, leave, phone]),
        (false, Some(later)),
        "still on the phone"
    );
    assert_eq!(
        read(&[w.owner_event(&Keys::generate(), JOINED, "a-3", 0)]),
        (false, None),
        "forged"
    );
    let content = json!({"ephemeral_channel_id": w.huddle, "admission_id": "a-5"});
    let h = w.parent.to_string();
    let guest = signed(&w.relay, JOINED, content, 0, &[["h", &h], ["p", &relay]]);
    assert_eq!(read(&[guest]), (false, None), "someone else");
    let mut tampered = w.owner_event(&w.relay, JOINED, "a-3", 0);
    tampered.tags = w.owner_event(&w.relay, JOINED, "a-4", 1).tags;
    tampered.created_at = Timestamp::from(1);
    assert_eq!(read(&[tampered]), (false, None));
    let (h, content) = (
        w.parent.to_string(),
        json!({"ephemeral_channel_id": Uuid::new_v4()}),
    );
    let elsewhere = signed(&w.relay, JOINED, content, 0, &[["h", &h], ["p", &owner]]);
    assert_eq!(read(&[elsewhere]), (false, None));
    assert!(read(&[w.owner_event(&w.relay, KIND_HUDDLE_ENDED, "", 0)]).0);
}
