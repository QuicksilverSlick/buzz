//! A one-on-one DM between an agent and its owner is addressed without a tag.
//!
//! Owner decision (29 Oct 2026): in a one-on-one conversation, one person and
//! one agent, no tag is needed; when other agents or people are present, tags
//! pick who answers. Answering is not acting: respond-to, allowlists, the DM
//! author gate and agent permissions are unchanged. Only the owner's own chat
//! messages count, never an agent's, so two agents cannot wake each other.
//!
//! In mentions mode such a DM is subscribed without `#p` and the local rule
//! takes its untagged owner messages as addressed. Each DM keeps a floor, the
//! time it first got the exemption; older untagged messages are not answered,
//! so turning this on never answers what was sent before it.

use std::collections::HashMap;

use buzz_core::kind::KIND_STREAM_MESSAGE;
use uuid::Uuid;

use crate::catch_up::CatchUp;
use crate::config::{ChannelFilter, Config, RespondTo, SubscribeMode};
use crate::pool::ChannelInfoResolver;

/// Whether the exemption applies: the default mentions mode with its mention
/// filter on, and someone to answer (not `respond-to=nobody`).
pub(crate) fn exemption_on(config: &Config) -> bool {
    config.subscribe_mode == SubscribeMode::Mentions
        && !config.no_mention_filter
        && !matches!(config.respond_to, RespondTo::Nobody)
}

/// The owner whose one-on-one DM is addressed: `owner`, when
/// [`exemption_on`].
pub(crate) fn exempt_owner(config: &Config, owner: Option<&str>) -> Option<String> {
    owner.filter(|_| exemption_on(config)).map(str::to_owned)
}

/// Whether a channel is a DM (by its kind:39000 type) whose participants
/// (its kind:39000 `p` tags) are exactly this agent and its owner. Unknown
/// metadata, no owner, or a third participant: no.
pub(crate) fn is_owner_dm(
    channel_type: &str,
    participants: &[String],
    agent: &str,
    owner: Option<&str>,
) -> bool {
    let Some(owner) = owner else {
        return false;
    };
    let one_of = |p: &String| p.eq_ignore_ascii_case(agent) || p.eq_ignore_ascii_case(owner);
    channel_type == "dm"
        && !owner.eq_ignore_ascii_case(agent)
        && participants.iter().any(|p| p.eq_ignore_ascii_case(agent))
        && participants.iter().any(|p| p.eq_ignore_ascii_case(owner))
        && participants.iter().all(one_of)
}

/// [`is_owner_dm`] from the resolver's kind:39000, which each prompt turn
/// refreshes. Not `is_dm_channel`, which counts an unresolved channel as a DM
/// (right for the author gate, wrong here): unresolved is not an owner DM.
pub(crate) async fn channel_is_owner_dm(
    channel_info: &ChannelInfoResolver,
    channel_id: Uuid,
    agent: &str,
    owner: &str,
) -> bool {
    channel_info
        .resolve_channel_metadata(channel_id)
        .await
        .is_some_and(|info| is_owner_dm(&info.channel_type, &info.participants, agent, Some(owner)))
}

/// Whether `event` is the owner talking: a chat message (kind 9; edits,
/// reminders and approvals still need the tag) that the owner signed, tagging
/// nobody but the agent and the owner, and carrying no `client` marker. Bench
/// marks what it posts with one (`["client", "bench:…"]`), and owner-signed
/// automation is not the owner talking; no client a person types in adds one.
pub(crate) fn is_owner_message(event: &nostr::Event, agent: &str, owner: &str) -> bool {
    event.kind.as_u16() as u32 == KIND_STREAM_MESSAGE
        && event.pubkey.to_hex().eq_ignore_ascii_case(owner)
        && event.tags.iter().all(|tag| match tag.as_slice() {
            [k, pk, ..] if k == "p" => {
                pk.eq_ignore_ascii_case(agent) || pk.eq_ignore_ascii_case(owner)
            }
            [k, ..] => k != "client",
            [] => true,
        })
}

/// Whether `event`, in an owner DM whose floor is `floor`, is addressed to
/// the agent without a tag: [`is_owner_message`], sent no earlier than the
/// floor.
pub(crate) fn addresses_agent(event: &nostr::Event, agent: &str, owner: &str, floor: u64) -> bool {
    event.created_at.as_secs() >= floor && is_owner_message(event, agent, owner)
}

/// Subscribe an owner DM without `#p`, keeping its kinds, and keep its floor:
/// the one saved when it first got the exemption, or else `first`.
pub(crate) fn exempt(
    floors: &mut HashMap<Uuid, u64>,
    catch_up: &CatchUp,
    channel: Uuid,
    filter: &mut ChannelFilter,
    first: u64,
) {
    filter.require_mention = false;
    floors
        .entry(channel)
        .or_insert_with(|| catch_up.owner_dm_floor(channel, first));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    const AT: u64 = 1_700_000_000;

    fn message(signer: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> nostr::Event {
        EventBuilder::new(Kind::Custom(kind), "hi")
            .tags(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(signer)
            .unwrap()
    }

    fn mentions_config() -> Config {
        let mut config = crate::build_mcp_servers_tests::test_config();
        config.subscribe_mode = SubscribeMode::Mentions;
        config
    }

    #[test]
    fn only_a_dm_of_exactly_agent_and_owner_is_an_owner_dm() {
        let (a, o, c) = ("a".repeat(64), "b".repeat(64), "c".repeat(64));
        let dm = |ty: &str, people: &[&str], owner: Option<&str>| {
            let people: Vec<String> = people.iter().map(|p| p.to_string()).collect();
            is_owner_dm(ty, &people, &a, owner)
        };
        assert!(dm("dm", &[&a, &o.to_uppercase()], Some(&o)));
        assert!(!dm("dm", &[&a, &o, &c], Some(&o)), "3 participants");
        assert!(!dm("dm", &[&a, &c], Some(&o)), "a sibling's DM");
        assert!(!dm("dm", &[&a, &o], None), "no resolved owner");
        assert!(!dm("unknown", &[], Some(&o)), "unknown metadata");
        assert!(!dm("dm", &[], Some(&o)), "no p tags");
        assert!(!dm("stream", &[&a, &o], Some(&o)), "not a DM");
        assert!(!dm("dm", &[&a], Some(&a)), "the agent is not its own owner");
    }

    /// The local rule end to end: the default mentions rule, fed the
    /// exemption verdict, admits only the owner's own untagged chat message.
    #[tokio::test]
    async fn local_rule_admits_only_the_owners_untagged_chat_message() {
        let config = mentions_config();
        let rules = crate::startup_subscription_rules(&config).unwrap();
        let agent = config.keys.public_key().to_hex();
        let (owner, sibling) = (Keys::generate(), Keys::generate());
        let owner_hex = owner.public_key().to_hex();
        let other = Keys::generate().public_key().to_hex();
        let floor = AT;
        let ch = Uuid::new_v4();
        let cases = [
            ("untagged owner message", message(&owner, 9, AT, &[]), true),
            (
                "owner message tagging the agent and owner",
                message(&owner, 9, AT, &[&["p", &agent], &["p", &owner_hex]]),
                true,
            ),
            ("sibling agent", message(&sibling, 9, AT, &[]), false),
            ("owner edit", message(&owner, 40003, AT, &[]), false),
            ("owner reminder", message(&owner, 40007, AT, &[]), false),
            ("owner approval", message(&owner, 46010, AT, &[]), false),
            (
                "owner tagging another pubkey",
                message(&owner, 9, AT, &[&["p", &other]]),
                false,
            ),
            (
                "Bench marker",
                message(&owner, 9, AT, &[&["client", "bench:route:x"]]),
                false,
            ),
            ("before the floor", message(&owner, 9, AT - 1, &[]), false),
        ];
        for (name, event, admitted) in cases {
            let addressed = addresses_agent(&event, &agent, &owner_hex, floor);
            let matched =
                crate::filter::match_event_addressed(&event, ch, &rules, &agent, addressed).await;
            assert_eq!(matched.is_some(), admitted, "{name}");
        }
        let untagged = message(&owner, 9, AT, &[]);
        assert!(
            crate::filter::match_event(&untagged, ch, &rules, &agent)
                .await
                .is_none(),
            "without the exemption the mention rule still drops it"
        );
    }

    #[test]
    fn owner_dm_subscribes_without_p_and_keeps_the_default_kinds() {
        let config = mentions_config();
        let ch = Uuid::new_v4();
        let mut filter = crate::config::resolve_dynamic_channel_filter(&config, ch, &[]).unwrap();
        assert!(filter.require_mention);
        let mut floors = HashMap::new();
        exempt(&mut floors, &CatchUp::default(), ch, &mut filter, AT);
        assert!(!filter.require_mention);
        assert_eq!(filter.kinds, Some(crate::config::default_mention_kinds()));
        exempt(&mut floors, &CatchUp::default(), ch, &mut filter, AT + 60);
        assert_eq!(floors[&ch], AT, "the first floor is kept");
    }

    #[test]
    fn exemption_needs_mentions_mode_someone_to_answer_and_an_owner() {
        let owner = Some("b".repeat(64));
        let mut config = mentions_config();
        assert_eq!(exempt_owner(&config, owner.as_deref()), owner);
        assert_eq!(exempt_owner(&config, None), None);
        config.respond_to = RespondTo::Nobody;
        assert_eq!(exempt_owner(&config, owner.as_deref()), None);
        let mut config = mentions_config();
        config.no_mention_filter = true;
        assert_eq!(exempt_owner(&config, owner.as_deref()), None);
        config.no_mention_filter = false;
        config.subscribe_mode = SubscribeMode::All;
        assert_eq!(exempt_owner(&config, owner.as_deref()), None);
    }

    /// Metadata from the relay's kind:39000: the lazy resolver reads the
    /// participants, and an unresolved channel is not an owner DM even though
    /// the author gate counts it as a DM.
    #[tokio::test]
    async fn resolver_reads_participants_and_unresolved_is_not_an_owner_dm() {
        let (agent, owner) = ("a".repeat(64), "b".repeat(64));
        let ch = Uuid::new_v4();
        let (rest, server) = crate::author_gate_tests::nip11_server(serde_json::json!([{
            "tags": [["d", ch.to_string()], ["t", "dm"], ["p", agent], ["p", owner], ["P", owner]]
        }]))
        .await;
        let resolver = ChannelInfoResolver::new(HashMap::new(), rest);
        assert!(channel_is_owner_dm(&resolver, ch, &agent, &owner).await);
        server.abort();

        let unreachable = crate::relay::RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:1".into(),
            keys: Keys::generate(),
            auth_tag_json: None,
        };
        let resolver = ChannelInfoResolver::new(HashMap::new(), unreachable);
        assert!(!channel_is_owner_dm(&resolver, ch, &agent, &owner).await);
        assert!(crate::is_dm_channel(ch, &resolver).await);
    }
}
