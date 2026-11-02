//! Huddle voices: when one runs (design 2026-10-29 §3.4; slice 8a, part 2). `voice_session` is the
//! run itself.
//!
//! Off unless the definition's switch is on: then [`Voice::start`] returns `None`, and the harness
//! makes no socket, no query and no bill, and takes kind 9 as it always did.
//!
//! With it on, a relay-signed 44100 about this agent starts one task per huddle: find the parent
//! from the huddle's 48100, check the actor against the author gate, wait for the owner's 48101,
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
//! ponytail: a kind 9 landing between the 44100 and the 48100 lookup still takes the text path, as
//! most 44100s are for ordinary channels. That's a moment at the start, before any audio.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use buzz_core::filter::NOTICE_CHANNEL_TYPE_TAG;
use buzz_core::kind::{
    KIND_HUDDLE_ENDED, KIND_HUDDLE_PARTICIPANT_JOINED, KIND_HUDDLE_PARTICIPANT_LEFT,
    KIND_HUDDLE_STARTED, KIND_MEMBER_REMOVED_NOTIFICATION, KIND_STREAM_MESSAGE,
};
use nostr::{Event, Tag};
use serde_json::{json, Value};
use tokio::sync::watch;
use tokio::time::sleep;
use uuid::Uuid;

use crate::config::{Config, RespondTo};
use crate::inbound_author_gate::voice_admits;
use crate::relay::RestClient;
use crate::voice_profile::VoiceProfile;
use crate::voice_session::{session, text, Left, Voicing};
use crate::{remember_gate_notice, OwnerCache};

const OWNER_GRACE: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_secs(5);
/// A minute of polls for the 48100, which a desktop posts just after adding its agents.
const LINK_POLLS: u32 = 12;
/// An hour of polls for an absent owner: the huddle channel's own TTL.
const OWNER_POLLS: u32 = 720;
/// Huddles tracked at once on one agent, each costing a query per poll: 1.6/s of the relay's ~5/s
/// `/query` budget. Past this a new one gets no voice, with a warning.
const MAX_HUDDLES: usize = 8;
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
    /// Huddles whose kind 9 is ignored: the gate passed and no line handed them back.
    voiced: Mutex<HashSet<Uuid>>,
    /// Huddles the parent has had a line about.
    told: Mutex<HashSet<(Uuid, String)>>,
}

/// The harness's handle on its voices; the main loop owns it.
pub(crate) struct Voice {
    shared: Arc<Shared>,
    shutdown: watch::Receiver<()>,
    /// Dropping a huddle's sender is what removes the agent from it.
    tracked: HashMap<Uuid, watch::Sender<()>>,
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
        let (shared, tracked) = (Arc::new(shared), HashMap::new());
        Self {
            shared,
            shutdown,
            tracked,
        }
    }

    /// A 44100 or 44101, past the main loop's replay dedup.
    pub(crate) fn on_membership(&mut self, notice: &Event) {
        self.tracked.retain(|_, sender| !sender.is_closed());
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
        } else if self.tracked.len() >= MAX_HUDDLES {
            tracing::warn!(%huddle, "huddle voice: {MAX_HUDDLES} tracked, so no voice here");
        } else {
            let (sender, removed) = watch::channel(());
            self.tracked.insert(huddle, sender);
            let stop = Stop(removed, self.shutdown.clone());
            tokio::spawn(self.shared.clone().track(huddle, notice.clone(), stop));
        }
    }

    /// Whether the main loop drops this event: every kind 9 in a voiced huddle until the rip-out,
    /// since speech-to-text posts carry no marker and can't be told from typing.
    pub(crate) fn ignores(&self, kind: u32, channel: Uuid) -> bool {
        kind == KIND_STREAM_MESSAGE && lock(&self.shared.voiced).contains(&channel)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Talos's socket base. Only `wss://`: the socket carries the NIP-98, the proofs and room audio.
fn pinned_talos(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(text)?;
    if url.scheme() != "wss" || !url.has_host() || url.query().is_some() || url.fragment().is_some()
    {
        return Err("--voice-talos-url must be a plain wss:// address".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
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
            ended = true;
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
        Left::NoLease(why) if why == "duplicate_holder" => None,
        Left::Done(why) if why == "stopped" || why == "the audio room ended" => None,
        Left::OwnerGone if !owner_here => None,
        _ => Some(LINE_OFF),
    }
}

struct Link {
    parent: Uuid,
    started: u64,
    actor: String,
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
            voiced: Mutex::default(),
            told: Mutex::default(),
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
        }
        lock(&self.voiced).remove(&huddle);
    }

    /// `Err` is why this agent stays out, for the log.
    async fn run(&self, huddle: Uuid, added: &Event, stop: &Stop) -> Result<(), String> {
        let link = tokio::select! {
            _ = stop.fired() => return Ok(()),
            link = self.link(huddle, added) => link?,
        };
        let gate = (&self.respond_to, &self.allowlist);
        let rest = &self.voicing.rest;
        let admitted = voice_admits(gate, &link.actor, &self.owner_cache, rest).await;
        if admitted != Some(true) {
            // `None` is a failed lookup, not proof they're outside the gate.
            let line = admitted.map_or(LINE_OFF, |_| LINE_GATE);
            self.say(huddle, link.parent, line).await;
            return Err("the actor didn't pass the author gate".into());
        }
        lock(&self.voiced).insert(huddle);
        let mut since = 0;
        loop {
            since = tokio::select! {
                _ = stop.fired() => return Ok(()),
                joined = self.owner_join(huddle, &link, since) => joined?,
            };
            lock(&self.voiced).insert(huddle);
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
            let room = self.room_state(huddle, &link).await;
            let owner_here = room.as_ref().map_or(true, |(_, open)| open.is_some());
            // What the run saw is spent: only a newer 48101 brings the voice back.
            let newest = room.ok().and_then(|(_, open)| open);
            since = newest.map_or(since, |newest| newest.max(since));
            if let Some(line) = line_for(&left, owner_here) {
                self.say(huddle, link.parent, line).await;
                lock(&self.voiced).remove(&huddle);
            }
        }
    }

    /// One page of events; a full page warns, as older ones are then unread.
    async fn read(&self, mut filter: Value, page: usize) -> Result<Vec<Event>, String> {
        filter["limit"] = page.into();
        let events = self.voicing.rest.query_raw(&[filter]).await.map_err(text)?;
        let events: Vec<Event> = serde_json::from_value(events).map_err(text)?;
        if events.len() >= page {
            tracing::warn!("huddle voice: {page} events on one page, so older ones are unread");
        }
        Ok(events)
    }

    /// The actor, and the parent from the 48100 naming this huddle. The relay checked at ingest
    /// that its signer created that private ephemeral channel.
    async fn link(&self, huddle: Uuid, notice: &Event) -> Result<Link, String> {
        let content: Value = serde_json::from_str(&notice.content).unwrap_or_default();
        let actor = content["actor"].as_str().ok_or("a notice with no actor")?;
        let since = notice.created_at.as_secs().saturating_sub(86_400);
        let starts = json!({"kinds": [KIND_HUDDLE_STARTED], "since": since});
        for _ in 0..LINK_POLLS {
            let events = self.read(starts.clone(), STARTS_PAGE).await;
            let events = events.inspect_err(|why| tracing::warn!(%huddle, %why, "huddle voice"));
            for event in events.iter().flatten().filter(|e| names_huddle(e, huddle)) {
                if let Some(parent) = tag(event, "h").and_then(|h| h.parse().ok()) {
                    let (started, actor) = (event.created_at.as_secs(), actor.to_owned());
                    return Ok(Link {
                        parent,
                        started,
                        actor,
                    });
                }
            }
            sleep(self.poll).await;
        }
        Err("no 48100 I can read: not a huddle, or a parent I'm not in".into())
    }

    /// At most one line per huddle, in the parent.
    async fn say(&self, huddle: Uuid, parent: Uuid, line: &str) {
        if remember_gate_notice(&mut lock(&self.told), (huddle, String::new())) {
            let thread = crate::queue::ThreadTags::default();
            crate::pool::post_notice(&self.voicing.rest, parent, &thread, line, &[]).await;
        }
    }

    async fn room_state(&self, huddle: Uuid, link: &Link) -> Result<(bool, Option<u64>), String> {
        let lifecycle = json!({
            "kinds": [KIND_HUDDLE_PARTICIPANT_JOINED, KIND_HUDDLE_PARTICIPANT_LEFT, KIND_HUDDLE_ENDED],
            "#h": [link.parent],
            // The 48100's time is the desktop's clock and these are the relay's.
            "since": link.started.saturating_sub(300),
        });
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
                    self.say(huddle, link.parent, LINE_WAITING).await;
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
