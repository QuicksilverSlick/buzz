//! When a voice runs, against a fake relay and the fake sockets of `voice_session`'s tests. No
//! network, no OpenAI. Real time, as the sockets are real, so timings here are bounds.

use super::*;
use crate::voice_session::tests::{fake_audio, fake_talos, listen, say, until, Seen, GRACE};
use axum::{extract::State, http::StatusCode, routing, Json, Router};
use buzz_core::kind::{
    KIND_MANAGED_AGENT, KIND_MEMBER_ADDED_NOTIFICATION as ADDED, KIND_STREAM_MESSAGE as NINE,
    KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_REMINDER,
};
use futures_util::{SinkExt, StreamExt};
use nostr::filter::MatchEventOptions;
use nostr::{EventBuilder, Filter, Keys, Kind};
use std::cmp::Reverse;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use tokio_tungstenite::accept_async;

const JOINED: u32 = KIND_HUDDLE_PARTICIPANT_JOINED;
const LEFT: u32 = KIND_HUDDLE_PARTICIPANT_LEFT;
const REMOVED: u32 = KIND_MEMBER_REMOVED_NOTIFICATION;
const ONLY_OWNER: RespondTo = RespondTo::OwnerOnly;
/// Words for a [`Counted`] Talos; any other word is a refusal.
const OPENED: &str = "opened";
const DROP: &str = "drop";

/// What the fake relay holds and saw.
#[derive(Clone, Default)]
struct Rest {
    store: Arc<Mutex<Vec<Event>>>,
    posts: Arc<Mutex<Vec<(String, String)>>>,
    /// `POST /query` answers 500.
    down: Arc<AtomicBool>,
}

/// `GET /`, `POST /query` as the relay reads filters (newest first, each to its own limit), and
/// `POST /events`.
async fn fake_rest(rest: Rest, relay: String) -> String {
    let query = |State(rest): State<Rest>, Json(filters): Json<Vec<Filter>>| async move {
        if rest.down.load(SeqCst) {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let mut store = rest.store.lock().unwrap().clone();
        store.sort_by_key(|e| Reverse(e.created_at));
        let mut found = Vec::new();
        for filter in filters {
            let matching = store
                .iter()
                .filter(|e| filter.match_event(e, MatchEventOptions::new()));
            found.extend(matching.take(filter.limit.unwrap_or(usize::MAX)).cloned());
        }
        Ok(Json(found))
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

/// Talos for a voice that comes back: it counts sockets and Closes. Each socket takes the script's
/// next word, the last one repeating: `opened`, `drop` (opened, then a dropped socket) or a refusal.
#[derive(Clone)]
struct Counted {
    opens: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    script: &'static [&'static str],
}

impl Counted {
    fn new(script: &'static [&'static str]) -> Self {
        let (opens, closes) = (Arc::default(), Arc::default());
        Self {
            opens,
            closes,
            script,
        }
    }

    fn opens(&self) -> usize {
        self.opens.load(SeqCst)
    }

    fn closes(&self) -> usize {
        self.closes.load(SeqCst)
    }

    async fn serve(self) -> String {
        let (listener, address) = listen().await;
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let talos = self.clone();
                tokio::spawn(talos.answer(stream));
            }
        });
        format!("ws://{address}")
    }

    async fn answer(self, stream: tokio::net::TcpStream) {
        let mut ws = accept_async(stream).await.unwrap();
        ws.next().await;
        let at = self.opens.fetch_add(1, SeqCst);
        let word = self.script[at.min(self.script.len() - 1)];
        if word != OPENED && word != DROP {
            let close = json!({"v": 1, "type": "close", "reason": word});
            let _ = ws.send(say(close)).await;
            return;
        }
        let opened = json!({"v": 1, "type": "opened", "lease_id": "l", "expires_at": 1});
        let _ = ws.send(say(opened)).await;
        if word == DROP {
            return;
        }
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                self.closes.fetch_add(1, SeqCst);
            }
        }
    }
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
    shutdown: watch::Sender<()>,
}

impl World {
    /// A huddle with its 48100, and the owner's 30177 for the agent. Talos is part 1's fake with
    /// this `refusal`; or, with `counted`, that one, a relay refusing audio and a short grace.
    async fn new(
        respond_to: RespondTo,
        refusal: Option<&'static str>,
        counted: Option<Counted>,
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
        let (talos_url, refuse, owner_grace) = match counted {
            Some(counted) => (counted.serve().await, Some("not a member"), GRACE),
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
            started: 0,
            slots: Semaphore::new(MAX_HUDDLES),
            voiced: Mutex::default(),
            told: Mutex::default(),
            spent: Mutex::default(),
        };
        let (shutdown, stop) = watch::channel(());
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
            shutdown,
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

    fn holds(&self, event: Event) {
        self.rest.store.lock().unwrap().push(event);
    }

    /// The relay commits a lifecycle event for the owner.
    fn relay_says(&self, kind: u32, admission: &str, later: u64) {
        self.holds(self.owner_event(&self.relay, kind, admission, later));
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

    /// The relay adds this agent to an ordinary channel, which no 48100 will ever name.
    fn ordinary_add(&mut self, actor: &str) {
        let (agent, h) = (self.agent_hex(), Uuid::new_v4().to_string());
        let tags = [["p", agent.as_str()], ["h", &h], ["t", "stream"]];
        let notice = signed(&self.relay, ADDED, json!({"actor": actor}), 0, &tags);
        self.voice.on_membership(&notice);
    }

    /// A stranger the gate can prove is outside, as they have a kind 0.
    fn stranger(&self) -> String {
        let stranger = Keys::generate();
        self.holds(signed(&stranger, 0, json!({}), 0, &[]));
        stranger.public_key().to_hex()
    }

    fn voiced(&self) -> bool {
        lock(&self.voice.shared.voiced).contains_key(&self.huddle)
    }

    fn working(&self) -> usize {
        let tracked = self.voice.tracked.values();
        tracked.filter(|sender| !sender.is_closed()).count()
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
/// of this, so a huddle's events go on as today. A switch that's on still needs a sound setup.
#[tokio::test]
async fn the_switch_off_starts_nothing_and_a_bad_setup_stays_off() {
    let (listener, address) = listen().await;
    let watched = format!("http://{address}");
    let o = Some("o");
    assert!(!starts(None, TALOS_URL, o, &watched).await);
    assert!(!starts(Some(VoiceProfile::default()), TALOS_URL, o, &watched).await);
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
    assert!(starts(on(String::new()), TALOS_URL, o, &relay).await);
    assert!(!starts(on(String::new()), TALOS_URL, None, &relay).await);
    assert!(!starts(on(String::new()), "wss://evil.test", o, &relay).await);
    assert!(!starts(on("x".repeat(1_001)), TALOS_URL, o, &relay).await);
}

#[test]
fn talos_is_only_ever_the_pinned_wss_host() {
    let host = TALOS_URL.strip_prefix("wss://").unwrap();
    assert_eq!(pinned_talos(TALOS_URL).unwrap(), TALOS_URL);
    assert_eq!(pinned_talos(&format!("{TALOS_URL}/")).unwrap(), TALOS_URL);
    assert_eq!(
        pinned_talos(&format!("WSS://{}", host.to_uppercase())).unwrap(),
        TALOS_URL
    );
    for bad in [
        format!("ws://{host}"),
        format!("https://{host}"),
        format!("wss://{host}:8443"),
        format!("wss://{host}/other"),
        format!("{TALOS_URL}?x=1"),
        format!("wss://user:pass@{host}"),
        format!("wss://{host}@evil.test"),
        format!("wss://{host}.evil.test"),
        "wss://evil.test".to_owned(),
        host.to_owned(),
        String::new(),
    ] {
        assert!(pinned_talos(&bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn the_owner_arriving_opens_talos_then_audio_and_only_the_relays_removal_leaves() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    settle().await;
    assert!(w.dialed_nothing(), "nothing dials before the owner");
    assert!(!w.voiced(), "typed messages still take the text path");
    assert!(w.lines().is_empty(), "an owner who added it is on the way");

    w.relay_says(JOINED, "a-1", 0);
    until("joined", || w.seen.saw("talos:roster")).await;
    assert_eq!(
        w.seen.log.lock().unwrap()[..2],
        ["talos:open", "audio:join"]
    );
    assert!(w.voiced());

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
    let actor = Keys::generate().public_key().to_hex();
    // No kind 0 yet: a failed check, so no claim about who they are.
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    w.notice(ADDED, &actor);
    until("the line", || !w.lines().is_empty()).await;
    assert_eq!(w.lines(), w.line(LINE_OFF));
    assert!(w.dialed_nothing() && !w.voiced());
    // That task's over, so its huddle isn't a repeat when the owner adds it.
    until("the task's end", || w.working() == 0).await;
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the owner's add", || w.seen.saw("audio:join")).await;

    // With one they're proven outside, and told once per huddle, however often they add it.
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    let actor = w.stranger();
    // A 48100 naming the huddle as its own parent, newest, can't draw the line into the huddle.
    let started = json!({"ephemeral_channel_id": w.huddle});
    let h = w.huddle.to_string();
    w.holds(signed(
        &w.owner,
        KIND_HUDDLE_STARTED,
        started,
        1,
        &[["h", &h]],
    ));
    w.notice(ADDED, &actor);
    until("the line", || !w.lines().is_empty()).await;
    settle().await;
    w.notice(ADDED, &actor);
    settle().await;
    assert_eq!(w.lines(), w.line(LINE_GATE));
    assert!(w.dialed_nothing() && !w.voiced());
}

#[tokio::test]
async fn no_gate_line_for_a_huddle_already_over_or_a_notice_replayed_after_a_restart() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(KIND_HUDDLE_ENDED, "", 0);
    let actor = w.stranger();
    w.notice(ADDED, &actor);
    until("the task's end", || w.working() == 0).await;
    assert!(w.lines().is_empty());

    let mut w = World::new(ONLY_OWNER, None, None).await;
    Arc::get_mut(&mut w.voice.shared).unwrap().started = u64::MAX;
    let actor = w.stranger();
    w.notice(ADDED, &actor);
    until("the task's end", || w.working() == 0).await;
    assert!(w.lines().is_empty());
}

#[tokio::test]
async fn anyone_else_adding_it_says_it_waits_for_the_owner_then_joins_on_their_48101() {
    let mut w = World::new(RespondTo::Anyone, None, None).await;
    w.notice(ADDED, &Keys::generate().public_key().to_hex());
    until("the line", || !w.lines().is_empty()).await;
    settle().await;
    assert_eq!(w.lines(), w.line(LINE_WAITING));
    assert!(w.dialed_nothing() && !w.voiced());

    // A client can post this kind; only the relay's word counts.
    w.holds(w.owner_event(&Keys::generate(), JOINED, "a-0", 0));
    settle().await;
    assert!(w.dialed_nothing());

    w.relay_says(JOINED, "a-1", 0);
    until("the join", || w.seen.saw("audio:join")).await;
}

/// Two harnesses for one agent make one voice, so this one neither talks nor takes kind 9. It looks
/// once more per 48101, in case the lease was its own.
#[tokio::test]
async fn a_second_holder_never_joins_audio_says_nothing_and_stays_voiced() {
    let talos = Counted::new(&[DUPLICATE_HOLDER]);
    let mut w = World::new(ONLY_OWNER, None, Some(talos.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the second look", || talos.opens() == 2).await;
    settle().await;
    assert_eq!(talos.opens(), 2, "one look again, not a loop");
    assert!(!w.seen.saw("audio:join") && w.lines().is_empty() && w.voiced());

    w.relay_says(JOINED, "a-2", 5);
    until("a newer 48101's two looks", || talos.opens() == 4).await;
    settle().await;
    assert_eq!(talos.opens(), 4);
}

/// Talos's socket drops, and the owner's next 48101 lands inside the lease's 20 s: the lease reads
/// as a second holder until it runs out, and that mustn't leave the huddle dead.
#[tokio::test]
async fn a_lease_this_pc_just_lost_doesnt_leave_the_huddle_dead() {
    let talos = Counted::new(&[DROP, DUPLICATE_HOLDER, OPENED]);
    let mut w = World::new(ONLY_OWNER, None, Some(talos.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the line", || !w.lines().is_empty()).await;
    assert_eq!(w.lines(), w.line(LINE_OFF));
    until("the text path back", || !w.voiced()).await;

    w.relay_says(JOINED, "a-2", 5);
    until("the look again", || talos.opens() == 3).await;
    settle().await;
    assert!(w.voiced() && w.lines().len() == 1);
}

/// The join keeps failing with the owner still in the room: `OwnerGone`, a line that says nothing
/// about them, and no paid retry until they join again. A second run isn't told again.
#[tokio::test]
async fn after_a_failing_join_only_a_newer_48101_brings_the_voice_back() {
    let talos = Counted::new(&[OPENED]);
    let mut w = World::new(ONLY_OWNER, None, Some(talos.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the open", || talos.opens() == 1).await;
    // Seen before the run ended, so spent with it.
    w.relay_says(JOINED, "a-2", 1);
    until("the line", || !w.lines().is_empty()).await;
    until("the text path back", || !w.voiced()).await;
    assert_eq!(w.lines(), w.line(LINE_OFF));
    settle().await;
    assert_eq!(talos.opens(), 1, "no newer 48101");

    w.relay_says(JOINED, "a-3", 5);
    until("the second open", || talos.opens() == 2).await;
    assert!(w.voiced() && w.lines().len() == 1);
}

/// A read that fails isn't proof the owner left, so the run that ended still gets its line.
#[tokio::test]
async fn a_failed_read_after_a_run_still_says_the_line() {
    let talos = Counted::new(&[OPENED]);
    let mut w = World::new(ONLY_OWNER, None, Some(talos.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the open", || talos.opens() == 1).await;
    w.rest.down.store(true, SeqCst);
    until("the line", || !w.lines().is_empty()).await;
    assert_eq!(w.lines(), w.line(LINE_OFF));
}

/// A removal's run isn't spent, so the owner can add the agent straight back. A run that ended any
/// other way is, and a remove and re-add can't buy a second paid open on the same 48101.
#[tokio::test]
async fn re_adding_it_opens_again_only_on_a_48101_no_run_has_spent() {
    let talos = Counted::new(&[OPENED]);
    let mut w = World::new(ONLY_OWNER, None, Some(talos.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the open", || talos.opens() == 1).await;
    w.notice(REMOVED, &owner);
    until("that task's end", || !w.voiced()).await;
    w.notice(ADDED, &owner);
    until("the open after a removal", || talos.opens() == 2).await;

    until("the line", || !w.lines().is_empty()).await;
    w.notice(REMOVED, &owner);
    w.notice(ADDED, &owner);
    settle().await;
    assert_eq!(talos.opens(), 2, "that 48101 is spent");
}

/// The owner really left: no line at all, kind 9 stays out, and it waits for them.
#[tokio::test]
async fn the_owner_leaving_says_nothing_and_stays_voiced() {
    let talos = Counted::new(&[OPENED]);
    let mut w = World::new(ONLY_OWNER, None, Some(talos.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the open", || talos.opens() == 1).await;
    w.relay_says(LEFT, "a-1", 0);
    until("the run's end", || talos.closes() == 1).await;
    settle().await;
    assert!(w.lines().is_empty() && w.voiced());
    assert_eq!(talos.opens(), 1);
}

#[tokio::test]
async fn a_repeat_add_changes_nothing_and_a_removal_while_waiting_opens_nothing() {
    let talos = Counted::new(&[OPENED]);
    let mut w = World::new(ONLY_OWNER, None, Some(talos.clone())).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the open", || talos.opens() == 1).await;
    w.notice(ADDED, &owner);
    settle().await;
    assert_eq!((talos.opens(), talos.closes()), (1, 0));

    let mut w = World::new(ONLY_OWNER, None, None).await;
    let actor = w.owner_hex();
    w.notice(ADDED, &actor);
    settle().await;
    w.notice(REMOVED, &actor);
    settle().await;
    w.relay_says(JOINED, "a-1", 0);
    settle().await;
    assert!(w.dialed_nothing());
}

/// Every slot is on ordinary channels, each until its polls for a 48100 run out. The real huddle
/// waits for one, as its 44100 won't come again, and someone refused doesn't queue in front of it.
#[tokio::test]
async fn a_full_house_makes_a_real_huddle_wait_and_refused_adds_never_queue() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    let (owner, stranger) = (w.owner_hex(), w.stranger());
    for _ in 0..MAX_HUDDLES {
        w.ordinary_add(&owner);
    }
    let slots = w.voice.shared.clone();
    until("every slot taken", || slots.slots.available_permits() == 0).await;
    w.notice(ADDED, &owner);
    w.ordinary_add(&stranger);
    until("the refused add's end", || w.working() == MAX_HUDDLES + 1).await;
    assert!(w.dialed_nothing(), "the real huddle is waiting");

    until("the voice", || w.seen.saw("audio:join")).await;
    assert!(w.voiced() && w.lines().is_empty());
    until("every other slot back", || {
        slots.slots.available_permits() == MAX_HUDDLES - 1
    })
    .await;
}

#[tokio::test]
async fn a_voiced_huddle_drops_every_kind_and_a_restart_wont_answer_them() {
    let now = Timestamp::now().as_secs();
    let dir = tempfile::tempdir().unwrap();
    let catch_up = CatchUp::open(dir.path(), "pk", "wss://r.test", 86_400, now);
    let w = World::new(ONLY_OWNER, None, None).await;
    let guest = Keys::generate();
    for kind in [NINE, KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_REMINDER] {
        let event = signed(&guest, kind, json!("hi"), 0, &[]);
        assert!(!w.voice.ignores(&event, w.huddle, &catch_up), "not voiced");
        assert!(!catch_up.is_done(&event.id.to_hex()));

        lock(&w.voice.shared.voiced).insert(w.huddle, event.id);
        assert!(!w.voice.ignores(&event, w.parent, &catch_up), "elsewhere");
        assert!(w.voice.ignores(&event, w.huddle, &catch_up));
        assert!(catch_up.is_done(&event.id.to_hex()));
        lock(&w.voice.shared.voiced).clear();
    }
}

/// Removed and re-added inside a run's few seconds of closing: the old task's end can't hand the
/// new run's huddle back to the text path.
#[tokio::test]
async fn an_old_tasks_end_leaves_a_newer_runs_huddle_voiced() {
    let w = World::new(ONLY_OWNER, None, None).await;
    let old = signed(&w.relay, ADDED, json!({}), 0, &[]);
    let new = signed(&w.relay, ADDED, json!({"new": true}), 0, &[]);
    lock(&w.voice.shared.voiced).insert(w.huddle, new.id);
    w.voice.shared.unvoice(w.huddle, &old);
    assert!(w.voiced());
    w.voice.shared.unvoice(w.huddle, &new);
    assert!(!w.voiced());
}

#[tokio::test]
async fn noise_in_the_parent_cant_hide_the_owners_48101() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    let guest = Keys::generate();
    for i in 0..LIFECYCLE_PAGE {
        w.holds(w.owner_event(&guest, JOINED, &format!("n-{i}"), 1));
    }
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the join", || w.seen.saw("audio:join")).await;
}

/// The relay takes a 48100 stamped up to 15 minutes ahead of its own clock.
#[tokio::test]
async fn a_desktop_clock_ahead_of_the_relay_cant_hide_the_owners_48101() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    let is_start = |e: &Event| e.kind.as_u16() as u32 == KIND_HUDDLE_STARTED;
    w.rest.store.lock().unwrap().retain(|e| !is_start(e));
    let started = json!({"ephemeral_channel_id": w.huddle});
    let h = w.parent.to_string();
    w.holds(signed(
        &w.owner,
        KIND_HUDDLE_STARTED,
        started,
        600,
        &[["h", &h]],
    ));
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("the join", || w.seen.saw("audio:join")).await;
}

/// The harness returning from `run_harness` ends the runtime, and every task with it.
#[tokio::test]
async fn shutdown_waits_for_each_run_to_close_both_sockets() {
    let mut w = World::new(ONLY_OWNER, None, None).await;
    w.relay_says(JOINED, "a-1", 0);
    let owner = w.owner_hex();
    w.notice(ADDED, &owner);
    until("joined", || w.seen.saw("talos:roster")).await;
    w.shutdown.send(()).unwrap();
    w.voice.drain().await;
    assert!(w.voice.tasks.is_empty(), "every run has finished leaving");
    until("Close 1000", || w.seen.saw("audio:close")).await;
    until("Talos's Close", || w.seen.saw("talos:close")).await;
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
        line_for(&Left::NoLease(DUPLICATE_HOLDER.into()), true),
        None
    );
    assert_eq!(line_for(&Left::Done("stopped".into()), true), None);
    assert_eq!(
        line_for(&Left::Done("the audio room ended".into()), true),
        None
    );
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
    let ended = w.owner_event(&w.relay, KIND_HUDDLE_ENDED, "", 0);
    assert!(read(std::slice::from_ref(&ended)).0);
    let mut tampered = ended;
    tampered.content = json!({"ephemeral_channel_id": w.huddle, "x": 1}).to_string();
    assert!(!read(&[tampered]).0, "an end that fails its signature");
}
