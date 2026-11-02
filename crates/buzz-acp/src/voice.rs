//! Huddle voices: when one runs (design 2026-10-29 §3.4; slice 8a, part 2). `voice_session` is the
//! run itself.
//!
//! Off unless the definition's switch is on: then [`Voice::start`] returns `None`, and the harness
//! makes no socket, no query and no bill, and takes a huddle's events as it always did.
//!
//! With it on, a relay-signed 44100 about this agent starts one task per huddle: check the actor
//! against the author gate, find the parent from the huddle's 48100, wait for the owner's 48101,
//! then run the voice. It leaves on 44101 and 48103, both through the run's `stop`. After a run
//! that ends any other way, only a newer 48101 brings it back. A decline says one line in the
//! parent, never in the huddle, and hands the huddle back to the text path.
//!
//! Still to come: the peer policy and the route feed. Until then only the owner is heard, other
//! agents read as humans in `roster`, and every peer gets half-duplex. All three fail safe.
//!
//! ponytail: lifecycle comes from polling `POST /query`, so a voice can trail the owner by 5 s. A
//! subscription on the parent is the upgrade if that proves too slow.
//!
//! ponytail: a huddle stays on the text path until its first run is due, so a typed message isn't
//! lost while the owner is away. A text turn begun just before can still post once in the huddle.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use buzz_core::filter::NOTICE_CHANNEL_TYPE_TAG;
use buzz_core::kind::{
    KIND_HUDDLE_ENDED, KIND_HUDDLE_PARTICIPANT_JOINED, KIND_HUDDLE_PARTICIPANT_LEFT,
    KIND_HUDDLE_STARTED, KIND_MEMBER_REMOVED_NOTIFICATION,
};
use nostr::{Event, EventId, Tag, Timestamp};
use serde_json::{json, Value};
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::catch_up::CatchUp;
use crate::config::{Config, RespondTo};
use crate::inbound_author_gate::voice_admits;
use crate::relay::RestClient;
use crate::voice_profile::VoiceProfile;
use crate::voice_session::{session, text, Left, Voicing};
use crate::{remember_gate_notice, OwnerCache};

/// The only Talos a voice dials. Its socket carries the NIP-98, the proofs and room audio, and a
/// definition's env can set `--voice-talos-url`, so the flag can't name another host. S3's custom
/// domain joins here.
pub(crate) const TALOS_URL: &str = "wss://dreamforge-talos.russelledeming.workers.dev";
const OWNER_GRACE: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_secs(5);
/// A minute of polls for the 48100, which a desktop posts just after adding its agents.
const LINK_POLLS: u32 = 12;
/// An hour of polls for an absent owner: the huddle channel's own TTL.
const OWNER_POLLS: u32 = 720;
/// Polls until a lease this PC lost has run out, which it does 20 s after its socket dropped.
const LEASE_POLLS: u32 = 5;
/// Huddles working at once on one agent, each costing a query per poll: 1.6/s of the relay's ~5/s
/// `/query` budget. Past this a new one waits for a slot, with a warning, as its 44100 won't come
/// again. Anyone the gate refuses never waits.
const MAX_HUDDLES: usize = 8;
/// The relay's own `MAX_TIMESTAMP_DRIFT_SECS`: how far a 48100 can be ahead of the relay's clock.
const MAX_DRIFT_SECS: u64 = 900;
/// Past `voice_session`'s 5 s for both Closes.
const DRAIN_WAIT: Duration = Duration::from_secs(6);
const DUPLICATE_HOLDER: &str = "duplicate_holder";
const STARTS_PAGE: usize = 200;
const LIFECYCLE_PAGE: usize = 500;

const LINE_GATE: &str = "I answer only certain people by voice, and whoever added me to this \
     huddle isn't one of them, so I'm staying out of its audio.";
const LINE_WAITING: &str = "I'll join this huddle by voice once my owner is in it.";
const LINE_OFF: &str =
    "I can't be in this huddle's audio right now, so I'll answer typed messages there instead.";

struct Shared {
    voicing: Arc<Voicing>,
    /// ponytail: read once at start, so a relay key change needs a restart. Notices fail until then.
    relay: String,
    owner_cache: OwnerCache,
    respond_to: RespondTo,
    allowlist: HashSet<String>,
    poll: Duration,
    /// When this process started. A notice older than this is a replay and earns no line, as `told`
    /// doesn't survive a restart.
    started: u64,
    slots: Semaphore,
    /// Huddles whose events are ignored, and the 44100 of the task that said so: a run was due and
    /// no line handed them back.
    voiced: Mutex<HashMap<Uuid, EventId>>,
    /// Huddles the parent has had a line about.
    told: Mutex<HashSet<(Uuid, String)>>,
    /// Each huddle's newest 48101 a finished run saw, across a remove and re-add. ponytail: an entry
    /// a removal leaves stays until restart, at a few bytes per huddle.
    spent: Mutex<HashMap<Uuid, u64>>,
}

/// The harness's handle on its voices; the main loop owns it.
pub(crate) struct Voice {
    shared: Arc<Shared>,
    shutdown: watch::Receiver<()>,
    /// Dropping a huddle's sender is what removes the agent from it.
    tracked: HashMap<Uuid, watch::Sender<()>>,
    tasks: JoinSet<()>,
}

impl Voice {
    /// `None` when the switch is off, before anything happens. Also when it's on and the setup
    /// can't be trusted, which logs an error.
    pub(crate) async fn start(
        config: &Config,
        owner: Option<&str>,
        auth_tag: Option<Tag>,
        rest: RestClient,
        shutdown: watch::Receiver<()>,
    ) -> Option<Self> {
        let profile = config.voice_profile.as_ref().filter(|p| p.enabled)?;
        let shared = Shared::new(config, profile, owner, auth_tag, rest).await;
        let shared = shared.inspect_err(|why| tracing::error!("huddle voice stays off: {why}"));
        Some(Self::new(shared.ok()?, shutdown))
    }

    fn new(shared: Shared, shutdown: watch::Receiver<()>) -> Self {
        let (shared, tracked, tasks) = (Arc::new(shared), HashMap::new(), JoinSet::new());
        Self {
            shared,
            shutdown,
            tracked,
            tasks,
        }
    }

    /// A 44100 or 44101, past the main loop's replay dedup.
    pub(crate) fn on_membership(&mut self, notice: &Event) {
        self.tracked.retain(|_, sender| !sender.is_closed());
        while self.tasks.try_join_next().is_some() {}
        let huddle = match self.shared.notice_huddle(notice) {
            Ok(huddle) => huddle,
            Err(why) => return tracing::warn!("huddle voice: ignoring a notice: {why}"),
        };
        if notice.kind.as_u16() as u32 == KIND_MEMBER_REMOVED_NOTIFICATION {
            self.tracked.remove(&huddle);
        } else if tag(notice, NOTICE_CHANNEL_TYPE_TAG).is_some_and(|t| t != "stream")
            || self.tracked.contains_key(&huddle)
        {
            // Not a huddle, or a repeat add, which changes nothing.
        } else {
            let (sender, removed) = watch::channel(());
            self.tracked.insert(huddle, sender);
            let stop = Stop(removed, self.shutdown.clone());
            self.tasks
                .spawn(self.shared.clone().track(huddle, notice.clone(), stop));
        }
    }

    /// Whether the main loop drops this event: all of a voiced huddle's until the rip-out, since
    /// speech-to-text posts carry no marker, and an edit or reminder would post in the huddle. Noted
    /// as refused, so a restart's catch-up doesn't answer it on the text path either.
    pub(crate) fn ignores(&self, event: &Event, channel: Uuid, catch_up: &CatchUp) -> bool {
        let ignored = lock(&self.shared.voiced).contains_key(&channel);
        if ignored {
            catch_up.note_refused(event.id.to_hex(), event.created_at.as_secs());
        }
        ignored
    }

    /// At harness exit, once the shutdown watch has fired: time for every run's Close, or Talos
    /// bills each lease for 20 s more and a relaunch meets `duplicate_holder`.
    pub(crate) async fn drain(&mut self) {
        let all = async { while self.tasks.join_next().await.is_some() {} };
        if timeout(DRAIN_WAIT, all).await.is_err() {
            tracing::warn!("huddle voice: a run didn't close in time, so its lease runs out alone");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Talos's socket base, once the flag has named [`TALOS_URL`].
fn pinned_talos(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(text)?;
    if url.as_str().trim_end_matches('/') != TALOS_URL {
        return Err(format!("--voice-talos-url must be {TALOS_URL}"));
    }
    Ok(TALOS_URL.to_owned())
}

fn tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [key, value, ..] if key == name => Some(value.as_str()),
        _ => None,
    })
}

fn names_huddle(event: &Event, huddle: Uuid) -> bool {
    let content = serde_json::from_str::<Value>(&event.content);
    content.is_ok_and(|c| c["ephemeral_channel_id"] == huddle.to_string())
}

/// From a parent's lifecycle events: whether `huddle` ended, and when the newest of the owner's
/// open admissions began. A participant event counts only signed by the relay, as these aren't
/// relay-only kinds. A relay crash can leave an admission open, but a run's roster rule ends it.
fn read_lifecycle(events: &[Event], huddle: Uuid, relay: &str, owner: &str) -> (bool, Option<u64>) {
    let mut events: Vec<&Event> = events.iter().filter(|e| names_huddle(e, huddle)).collect();
    // In a tie a join comes before its leave.
    events.sort_by_key(|e| (e.created_at.as_secs(), e.kind.as_u16()));
    let (mut ended, mut open) = (false, HashMap::new());
    for event in events {
        let kind = event.kind.as_u16() as u32;
        if kind == KIND_HUDDLE_ENDED {
            ended |= event.verify().is_ok();
        } else if event.pubkey.to_hex() == relay
            && tag(event, "p").is_some_and(|p| p.eq_ignore_ascii_case(owner))
            && event.verify().is_ok()
        {
            let content: Value = serde_json::from_str(&event.content).unwrap_or_default();
            let admission = content["admission_id"].as_str().unwrap_or("").to_owned();
            if kind == KIND_HUDDLE_PARTICIPANT_JOINED {
                open.insert(admission, event.created_at.as_secs());
            } else {
                open.remove(&admission);
            }
        }
    }
    (ended, open.into_values().max())
}

/// The parent's one line for how a run ended, if it earns one. Only these constants are ever
/// posted, as `Left`'s strings are remote text. An owner who really left needs no telling, and no
/// line says the owner left, as `OwnerGone` also covers a join that kept failing.
fn line_for(left: &Left, owner_here: bool) -> Option<&'static str> {
    match left {
        Left::NoLease(why) if why == DUPLICATE_HOLDER => None,
        Left::Done(why) if why == "stopped" || why == "the audio room ended" => None,
        Left::OwnerGone if !owner_here => None,
        _ => Some(LINE_OFF),
    }
}

struct Link {
    parent: Uuid,
    started: u64,
    actor: String,
    /// The notice came before this process did.
    replayed: bool,
}

/// What ends a huddle's task: the tracked sender dropping (the agent was removed), or shutdown.
struct Stop(watch::Receiver<()>, watch::Receiver<()>);

impl Stop {
    async fn fired(&self) {
        let (mut removed, mut shutdown) = (self.0.clone(), self.1.clone());
        tokio::select! {
            _ = removed.changed() => {}
            _ = shutdown.changed() => {}
        }
    }
}

impl Shared {
    async fn new(
        config: &Config,
        profile: &VoiceProfile,
        owner: Option<&str>,
        auth_tag: Option<Tag>,
        rest: RestClient,
    ) -> Result<Self, String> {
        let owner = owner.ok_or("there's no owner to be present")?;
        if profile.voice_without_owner {
            tracing::warn!("voice_without_owner stays off until Phase 4: the owner must be here");
        }
        let (talos_url, profile) = (pinned_talos(&config.voice_talos_url)?, profile.wire()?);
        let relay = rest.relay_self().await.map_err(text)?;
        let voicing = Voicing {
            agent: rest.keys.public_key().to_hex(),
            owner: owner.to_owned(),
            auth_tag,
            relay_url: config.relay_url.clone(),
            talos_url,
            profile,
            owner_grace: OWNER_GRACE,
            rest,
        };
        Ok(Self {
            voicing: Arc::new(voicing),
            relay: relay.ok_or("the relay names no identity")?,
            owner_cache: OwnerCache::new(Some(owner.to_owned())),
            respond_to: config.respond_to.clone(),
            allowlist: config.respond_to_allowlist.clone(),
            poll: POLL,
            started: Timestamp::now().as_secs(),
            slots: Semaphore::new(MAX_HUDDLES),
            voiced: Mutex::default(),
            told: Mutex::default(),
            spent: Mutex::default(),
        })
    }

    /// The huddle a membership notice is about, if the relay signed it and it's about this agent.
    fn notice_huddle(&self, notice: &Event) -> Result<Uuid, &'static str> {
        if notice.pubkey.to_hex() != self.relay || notice.verify().is_err() {
            return Err("the relay didn't sign it");
        }
        if tag(notice, "p") != Some(self.voicing.agent.as_str()) {
            return Err("it's about someone else");
        }
        let channel = tag(notice, "h").and_then(|h| h.parse().ok());
        channel.ok_or("it names no channel")
    }

    async fn track(self: Arc<Self>, huddle: Uuid, added: Event, stop: Stop) {
        if let Err(why) = self.run(huddle, &added, &stop).await {
            tracing::info!(%huddle, "huddle voice stays out: {why}");
            lock(&self.spent).remove(&huddle);
        }
        self.unvoice(huddle, &added);
    }

    /// Back to the text path, unless a task from a later 44100 holds the huddle by now.
    fn unvoice(&self, huddle: Uuid, added: &Event) {
        let mut voiced = lock(&self.voiced);
        if voiced.get(&huddle) == Some(&added.id) {
            voiced.remove(&huddle);
        }
    }

    /// `Err` is why this agent stays out, for the log.
    async fn run(&self, huddle: Uuid, added: &Event, stop: &Stop) -> Result<(), String> {
        let content: Value = serde_json::from_str(&added.content).unwrap_or_default();
        let actor = content["actor"].as_str().ok_or("a notice with no actor")?;
        let gate = (&self.respond_to, &self.allowlist);
        let rest = &self.voicing.rest;
        let admitted = voice_admits(gate, actor, &self.owner_cache, rest).await;
        // Someone refused takes a slot only to say so, and never stands in front of a real huddle.
        let _slot = if admitted != Some(true) {
            self.slots.try_acquire().map_err(text)?
        } else {
            if self.slots.available_permits() == 0 {
                tracing::warn!(
                    %huddle,
                    "huddle voice: {MAX_HUDDLES} huddles hold every slot, so this one's voice \
                     waits for one to end; typed messages take the text path until then"
                );
            }
            tokio::select! {
                _ = stop.fired() => return Ok(()),
                slot = self.slots.acquire() => slot.map_err(text)?,
            }
        };
        let link = tokio::select! {
            _ = stop.fired() => return Ok(()),
            link = self.link(huddle, added, actor) => link?,
        };
        if admitted != Some(true) {
            // `None` is a failed lookup, not proof they're outside the gate.
            let line = admitted.map_or(LINE_OFF, |_| LINE_GATE);
            if !matches!(self.room_state(huddle, &link).await, Ok((true, _))) {
                self.say(huddle, &link, line).await;
            }
            return Err("the actor didn't pass the author gate".into());
        }
        let mut since = lock(&self.spent).get(&huddle).copied().unwrap_or(0);
        let mut looked_again = false;
        loop {
            let joined = tokio::select! {
                _ = stop.fired() => return Ok(()),
                joined = self.owner_join(huddle, &link, since) => joined?,
            };
            lock(&self.voiced).insert(huddle, added.id);
            let until = async {
                tokio::select! {
                    _ = stop.fired() => {}
                    _ = self.ended(huddle, &link) => {}
                }
            };
            let left = session(&self.voicing, huddle, link.parent, added, until).await;
            // The variant only: its strings are remote text.
            let why = format!("{left:?}");
            let why = why.split('(').next();
            tracing::info!(%huddle, ?why, "huddle voice: the run ended");
            // A lease this PC just lost outlives it and reads as a second holder, so look once more
            // when it has run out; 8e's resume replaces this. A real holder just refuses again.
            if matches!(&left, Left::NoLease(why) if why == DUPLICATE_HOLDER) {
                if !std::mem::replace(&mut looked_again, true) {
                    tokio::select! {
                        _ = stop.fired() => return Ok(()),
                        _ = sleep(self.poll * LEASE_POLLS) => continue,
                    }
                }
                tracing::warn!(%huddle, "huddle voice: another harness holds this voice");
            }
            looked_again = false;
            let room = self.room_state(huddle, &link).await;
            let owner_here = room.as_ref().map_or(true, |(_, open)| open.is_some());
            // What the run saw is spent: only a newer 48101 brings the voice back. A removal's run
            // isn't, as an owner who adds the agent back wants it back.
            let newest = room.ok().and_then(|(_, open)| open);
            since = newest.map_or(joined, |newest| newest.max(joined));
            if !matches!(&left, Left::Done(why) if why == "stopped") {
                lock(&self.spent).insert(huddle, since);
            }
            if let Some(line) = line_for(&left, owner_here) {
                self.say(huddle, &link, line).await;
                self.unvoice(huddle, added);
            }
        }
    }

    /// One page per filter; a full one warns, as older events are then unread.
    async fn read(&self, mut filters: Vec<Value>, page: usize) -> Result<Vec<Event>, String> {
        for filter in &mut filters {
            filter["limit"] = page.into();
        }
        let events = self.voicing.rest.query_raw(&filters).await.map_err(text)?;
        let events: Vec<Event> = serde_json::from_value(events).map_err(text)?;
        if events.len() >= page {
            tracing::warn!("huddle voice: {page}+ events in one read, so older ones may be unread");
        }
        Ok(events)
    }

    /// The parent from the 48100 naming this huddle. The relay checked at ingest that its signer
    /// created that private ephemeral channel. Its creator could still name the huddle itself, and
    /// nothing posts there.
    async fn link(&self, huddle: Uuid, notice: &Event, actor: &str) -> Result<Link, String> {
        let since = notice.created_at.as_secs().saturating_sub(86_400);
        let starts = json!({"kinds": [KIND_HUDDLE_STARTED], "since": since});
        for _ in 0..LINK_POLLS {
            let events = self.read(vec![starts.clone()], STARTS_PAGE).await;
            let events = events.inspect_err(|why| tracing::warn!(%huddle, %why, "huddle voice"));
            for event in events.iter().flatten().filter(|e| names_huddle(e, huddle)) {
                let parent = tag(event, "h").and_then(|h| h.parse().ok());
                if let Some(parent) = parent.filter(|parent| *parent != huddle) {
                    let (started, actor) = (event.created_at.as_secs(), actor.to_owned());
                    let replayed = notice.created_at.as_secs() < self.started;
                    return Ok(Link {
                        parent,
                        started,
                        actor,
                        replayed,
                    });
                }
            }
            sleep(self.poll).await;
        }
        Err("no 48100 I can read: not a huddle, or a parent I'm not in".into())
    }

    /// At most one line per huddle, in the parent.
    async fn say(&self, huddle: Uuid, link: &Link, line: &str) {
        if !link.replayed && remember_gate_notice(&mut lock(&self.told), (huddle, String::new())) {
            let thread = crate::queue::ThreadTags::default();
            crate::pool::post_notice(&self.voicing.rest, link.parent, &thread, line, &[]).await;
        }
    }

    async fn room_state(&self, huddle: Uuid, link: &Link) -> Result<(bool, Option<u64>), String> {
        // The 48100's time is the desktop's clock and these are the relay's.
        let (h, since) = ([link.parent], link.started.saturating_sub(MAX_DRIFT_SECS));
        let joins = [KIND_HUDDLE_PARTICIPANT_JOINED, KIND_HUDDLE_PARTICIPANT_LEFT];
        let lifecycle = vec![
            // Only the relay's: any member can post these kinds, and could bury its off the page.
            json!({"kinds": joins, "authors": [self.relay], "#h": h, "since": since}),
            json!({"kinds": [KIND_HUDDLE_ENDED], "#h": h, "since": since}),
        ];
        let events = self.read(lifecycle, LIFECYCLE_PAGE).await?;
        Ok(read_lifecycle(
            &events,
            huddle,
            &self.relay,
            &self.voicing.owner,
        ))
    }

    /// Poll until the owner holds an admission newer than `since`, and return when it began: the
    /// re-check on their 48101, for an agent added before they arrived or a run that's over.
    async fn owner_join(&self, huddle: Uuid, link: &Link, since: u64) -> Result<u64, String> {
        for _ in 0..OWNER_POLLS {
            match self.room_state(huddle, link).await {
                Ok((true, _)) => return Err("the huddle ended".into()),
                Ok((_, Some(joined))) if joined > since => return Ok(joined),
                // An owner who added it is on the way; anyone else may be alone a while.
                Ok(_) if since == 0 && link.actor != self.voicing.owner => {
                    self.say(huddle, link, LINE_WAITING).await;
                }
                Ok(_) => {}
                Err(why) => tracing::warn!(%huddle, %why, "huddle voice"),
            }
            sleep(self.poll).await;
        }
        Err("stopped waiting for the owner after an hour".into())
    }

    async fn ended(&self, huddle: Uuid, link: &Link) {
        loop {
            sleep(self.poll).await;
            if matches!(self.room_state(huddle, link).await, Ok((true, _))) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
