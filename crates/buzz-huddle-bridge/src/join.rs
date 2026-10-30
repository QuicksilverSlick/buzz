//! Audio admission as the agent: the relay's NIP-42 challenge signed with the agent key and its
//! NIP-OA owner tag, the same tags buzz-acp's relay auth uses. The protocol version comes from
//! `buzz-audio-client`, so the bridge and the desktop always speak the same one.

use buzz_audio_client::connection::{connect, Joined};
use buzz_audio_client::wire::PROTOCOL_VERSION;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, handshake::client::Request};
use uuid::Uuid;

/// The upgrade request for a huddle's audio room. It is a request, not a URL, so the caller
/// can add headers (a `Nostr-Federated-Identity` assertion once NIP-FI is enforced).
pub fn audio_request(relay_url: &str, huddle: Uuid) -> Result<Request, String> {
    let url = format!("{}/huddle/{huddle}/audio", relay_url.trim_end_matches('/'));
    url.into_client_request()
        .map_err(|e| format!("audio request: {e}"))
}

/// The kind 22242 answer to the audio challenge.
pub fn auth_event(
    keys: &Keys,
    relay_url: &str,
    challenge: &str,
    owner_tag: Option<&Tag>,
) -> Result<Event, String> {
    let mut tags = vec![
        Tag::parse(["relay", relay_url]).map_err(|e| format!("relay tag: {e}"))?,
        Tag::parse(["challenge", challenge]).map_err(|e| format!("challenge tag: {e}"))?,
    ];
    tags.extend(owner_tag.cloned());
    EventBuilder::new(Kind::Authentication, "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| format!("sign audio auth: {e}"))
}

/// Join the audio room. A refusal is logged with the version this side speaks, so an
/// `upgrade_required` from a room pinned to another version shows in the logs.
pub async fn join(
    request: Request,
    parent: Uuid,
    relay_url: &str,
    keys: &Keys,
    owner_tag: Option<&Tag>,
) -> Result<Joined, String> {
    let parent = parent.to_string();
    connect(request, Some(&parent), None, |challenge| async move {
        let event = auth_event(keys, relay_url, &challenge, owner_tag)?;
        serde_json::to_value(&event).map_err(|e| format!("audio auth json: {e}"))
    })
    .await
    .inspect_err(|e| {
        tracing::warn!(protocol_version = PROTOCOL_VERSION, error = %e, "huddle audio join refused")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_event_carries_relay_challenge_and_owner_tag() {
        let (agent, owner) = (Keys::generate(), Keys::generate());
        let owner_tag = Tag::parse(["auth", &owner.public_key().to_hex(), "", "sig"]).unwrap();
        let event = auth_event(&agent, "wss://relay.test", "c-1", Some(&owner_tag)).unwrap();
        event.verify().unwrap();
        assert_eq!(event.kind, Kind::Custom(22242));
        assert_eq!(event.pubkey, agent.public_key());
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.clone().to_vec()).collect();
        assert_eq!(tags[0], ["relay", "wss://relay.test"]);
        assert_eq!(tags[1], ["challenge", "c-1"]);
        assert_eq!(tags[2], owner_tag.to_vec());
        assert_eq!(
            auth_event(&agent, "wss://r", "c", None).unwrap().tags.len(),
            2
        );
    }

    #[test]
    fn audio_request_targets_the_room_and_takes_headers() {
        let huddle = Uuid::nil();
        let mut request = audio_request("wss://relay.test/", huddle).unwrap();
        assert_eq!(
            request.uri().to_string(),
            format!("wss://relay.test/huddle/{huddle}/audio")
        );
        request
            .headers_mut()
            .insert("Nostr-Federated-Identity", "token".parse().unwrap());
        assert!(audio_request("not a url", huddle).is_err());
    }
}
