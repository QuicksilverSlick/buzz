//! Talk to Talos: the pinned identity, the signed `POST /df/talk`, and the
//! Talos-only huddle add. Nothing here runs before a click, and with no pin every
//! Talk to Talos control stays hidden.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    commands::admin::client::ADMIN_CLIENT,
    events,
    huddle::HuddlePhase,
    relay::{build_nip98_auth_header, submit_event},
};

/// Talos's public key (64 hex) and origin, side by side. Empty until setup: S3
/// gives the host (Talos checks the exact NIP-98 `u`, so this must be `https://`
/// plus its `PUBLIC_HOST`) and S4 makes and admits the key. Filling both is the
/// one change that lets the button show.
const TALOS_PUBKEY: &str = "";
const TALOS_URL: &str = "";

/// Talos refuses a snapshot over 8 KB, measured after NFKC, so the page normalizes
/// before this cap sees it. The newest text is kept and the cut is marked, so Talos
/// knows it has only the end.
const SNAPSHOT_MAX_BYTES: usize = 8 * 1024;
const SNAPSHOT_CUT_MARK: &str = "[earlier text cut at 8 KB]\n";
const TALK_TIMEOUT: Duration = Duration::from_secs(15);
const TRY_AGAIN: &str = "Try again in a minute. If it keeps failing, check that Talos is deployed.";

/// What was on screen at the click. Talos treats `snapshot` as untrusted data.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TalkContext {
    route: String,
    channel_id: Option<String>,
    thread_root: Option<String>,
    card_id: Option<String>,
    project_id: Option<String>,
    snapshot: Option<String>,
}

#[derive(Deserialize)]
struct Refusal {
    message: String,
}

/// The env value counts in debug builds only, so a launch check can aim at a
/// member key and a fake `/df/talk`. A release build always uses the pin.
fn pinned(env_name: &str, built_in: &str) -> String {
    std::env::var(env_name)
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| built_in.to_string())
}

/// Both values or neither: the button never shows with nowhere to send a click.
fn parse_pin(url: &str, pubkey: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let key = nostr::PublicKey::from_hex(pubkey.trim()).ok()?;
    Some(key.to_hex())
}

#[tauri::command]
pub fn get_talos_pubkey() -> Option<String> {
    parse_pin(
        &pinned("BUZZ_TALOS_URL", TALOS_URL),
        &pinned("BUZZ_TALOS_PUBKEY", TALOS_PUBKEY),
    )
}

fn cap_snapshot(text: String) -> String {
    if text.len() <= SNAPSHOT_MAX_BYTES {
        return text;
    }
    let mut start = text.len() - (SNAPSHOT_MAX_BYTES - SNAPSHOT_CUT_MARK.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("{SNAPSHOT_CUT_MARK}{}", &text[start..])
}

/// Talos words its own refusals. Anything else (the route not deployed yet, a
/// platform error) still gets a sentence, never a bare status.
fn refusal_sentence(status: StatusCode, body: &str) -> String {
    match serde_json::from_str::<Refusal>(body) {
        Ok(refusal) => refusal.message.chars().take(300).collect(),
        Err(_) if status == StatusCode::NOT_FOUND => {
            format!("Talos can't take this yet: its /df/talk isn't live. Nothing was started. {TRY_AGAIN}")
        }
        Err(_) => format!(
            "Talos answered with an error ({}). Nothing was started. {TRY_AGAIN}",
            status.as_u16()
        ),
    }
}

async fn post_talk(
    base_url: &str,
    mut context: TalkContext,
    state: &AppState,
) -> Result<(), String> {
    // The admin client doesn't follow redirects: that would carry the NIP-98
    // header and the snapshot off the pinned host.
    let client = ADMIN_CLIENT.get().ok_or("The HTTP client isn't ready.")?;
    context.snapshot = context.snapshot.map(cap_snapshot);
    let body = serde_json::to_vec(&context).map_err(|error| error.to_string())?;

    let url = format!("{base_url}/df/talk");
    let auth_header = build_nip98_auth_header(&Method::POST, &url, &body, state)?;
    let response = client
        .post(&url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .timeout(TALK_TIMEOUT)
        .body(body)
        .send()
        .await
        .map_err(|_| format!("Talos didn't answer. Nothing was started. {TRY_AGAIN}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(refusal_sentence(status, &body))
}

async fn add_to_huddle(pubkey: &str, state: &AppState) -> Result<(), String> {
    let ephemeral_id = {
        let huddle = state.huddle()?;
        huddle
            .ephemeral_channel_id
            .clone()
            .filter(|_| matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active))
            .ok_or("There's no active huddle to add Talos to.")?
    };
    let channel = Uuid::parse_str(&ephemeral_id).map_err(|error| error.to_string())?;
    // One kind 9000, role `member`. Never `add_agent_to_huddle`: that also adds
    // the parent as a `bot`, permanently, and a `bot` member turns speech-to-text
    // on. The roster poll picks Talos up within 15 s.
    let add = events::build_add_member(channel, pubkey, Some("member"))?;
    submit_event(add, state)
        .await
        .map(|_| ())
        .map_err(|error| format!("Talos couldn't be added to this huddle: {error}"))
}

#[tauri::command]
pub async fn talk_to_talos(context: TalkContext, state: State<'_, AppState>) -> Result<(), String> {
    post_talk(&pinned("BUZZ_TALOS_URL", TALOS_URL), context, &state).await
}

#[tauri::command]
pub async fn add_talos_to_huddle(state: State<'_, AppState>) -> Result<(), String> {
    let pubkey = get_talos_pubkey().ok_or("This build has no Talos key.")?;
    add_to_huddle(&pubkey, &state).await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{body::to_bytes, extract::Request, http};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use nostr::{Event, JsonUtil, Keys};
    use sha2::{Digest, Sha256};

    use super::*;

    type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

    /// A fake `/df/talk` or relay: one answer to every request, and a record of
    /// each (path, Authorization, body).
    async fn fake_server(status: u16, reply: &'static str) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let log = Arc::clone(&seen);
        let app = axum::Router::new().fallback(move |request: Request| {
            let log = Arc::clone(&log);
            async move {
                let (parts, body) = request.into_parts();
                let auth = parts.headers.get("authorization");
                let body = to_bytes(body, usize::MAX).await.unwrap().to_vec();
                log.lock().unwrap().push((
                    parts.uri.path().to_string(),
                    auth.map_or("", |value| value.to_str().unwrap()).to_string(),
                    body,
                ));
                http::Response::builder()
                    .status(status)
                    .header("location", "https://elsewhere.invalid/")
                    .body(axum::body::Body::from(reply))
                    .unwrap()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), seen)
    }

    fn test_state() -> AppState {
        crate::commands::admin::client::init_admin_client().unwrap();
        crate::app_state::build_app_state()
    }

    fn context() -> TalkContext {
        TalkContext {
            route: "channels".to_string(),
            snapshot: Some("the card".to_string()),
            ..TalkContext::default()
        }
    }

    fn tag(event: &Event, name: &str) -> String {
        let found = event.tags.iter().find(|tag| tag.as_slice()[0] == name);
        found.unwrap().as_slice()[1].clone()
    }

    /// Mutation evidence: hashing anything but the sent bytes, or signing another
    /// URL, flips the `payload` or `u` assertion.
    #[tokio::test]
    async fn talk_signs_the_exact_url_and_body() {
        let (base, seen) = fake_server(200, r#"{"ticket":"t","expires_at":1}"#).await;
        let state = test_state();
        let keys = Keys::generate();
        *state.keys.lock().unwrap() = keys.clone();

        post_talk(&base, context(), &state).await.unwrap();

        let seen = seen.lock().unwrap();
        let [(path, auth, body)] = seen.as_slice() else {
            panic!("exactly one request");
        };
        assert_eq!(path, "/df/talk");
        let signed = BASE64.decode(auth.strip_prefix("Nostr ").unwrap()).unwrap();
        let event = Event::from_json(signed).unwrap();
        event.verify().unwrap();
        assert_eq!(event.kind.as_u16(), 27235);
        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(tag(&event, "u"), format!("{base}/df/talk"));
        assert_eq!(tag(&event, "method"), "POST");
        assert_eq!(tag(&event, "payload"), hex::encode(Sha256::digest(body)));

        let json: serde_json::Value = serde_json::from_slice(body).unwrap();
        assert_eq!(json["route"], "channels");
        assert_eq!(json["card_id"], serde_json::Value::Null);
        assert_eq!(json["snapshot"], "the card");
    }

    #[tokio::test]
    async fn a_refusal_is_a_sentence_and_a_redirect_is_not_followed() {
        let state = test_state();
        for (status, reply, expected) in [
            (404, "Not found", "isn't live"),
            (500, "", "(500)"),
            (307, "", "(307)"),
            (522, "", "(522)"),
            (
                503,
                r#"{"error":"off","message":"Switched off."}"#,
                "Switched off.",
            ),
        ] {
            let (base, seen) = fake_server(status, reply).await;
            let error = post_talk(&base, context(), &state).await.unwrap_err();
            assert!(error.contains(expected), "{status}: {error}");
            assert_eq!(seen.lock().unwrap().len(), 1, "{status}");
            if !reply.contains("message") {
                assert!(error.contains(TRY_AGAIN), "{status}: {error}");
            }
        }
        let error = post_talk("http://127.0.0.1:9", context(), &state)
            .await
            .unwrap_err();
        assert!(error.contains("didn't answer"), "{error}");
    }

    #[test]
    fn the_pin_needs_both_values_and_a_real_key() {
        let key = Keys::generate().public_key().to_hex();
        let url = "https://talos.test";
        assert_eq!(parse_pin(url, &key).as_deref(), Some(key.as_str()));
        assert_eq!(parse_pin("", &key), None);
        assert_eq!(parse_pin(url, ""), None);
        assert_eq!(parse_pin(url, "not-hex"), None);
    }

    #[test]
    fn the_snapshot_keeps_the_newest_text_and_marks_the_cut() {
        assert_eq!(cap_snapshot("short".to_string()), "short");
        let capped = cap_snapshot(format!("{}END", "é".repeat(SNAPSHOT_MAX_BYTES)));
        assert!(capped.len() <= SNAPSHOT_MAX_BYTES, "{}", capped.len());
        assert!(capped.starts_with(SNAPSHOT_CUT_MARK) && capped.ends_with("END"));
    }

    /// Mutation evidence: calling `agents::add_agent_to_huddle` here publishes a
    /// second event and role `bot`, and flips this RED.
    #[tokio::test]
    async fn the_add_is_one_member_9000_to_the_huddle_only() {
        let accepted = r#"{"event_id":"0","accepted":true,"message":""}"#;
        let (base, seen) = fake_server(200, accepted).await;
        let state = test_state();
        *state.relay_url_override.lock().unwrap() = Some(base.replacen("http", "ws", 1));
        let ephemeral = Uuid::new_v4();
        state.huddle().unwrap().ephemeral_channel_id = Some(ephemeral.to_string());
        let talos = "ab".repeat(32);

        // No active huddle yet: nothing is published.
        assert!(add_to_huddle(&talos, &state).await.is_err());
        assert!(seen.lock().unwrap().is_empty());

        state.huddle().unwrap().phase = HuddlePhase::Active;
        add_to_huddle(&talos, &state).await.unwrap();

        let seen = seen.lock().unwrap();
        let [(_, _, body)] = seen.as_slice() else {
            panic!("exactly one event");
        };
        let event = Event::from_json(body).unwrap();
        assert_eq!(event.kind.as_u16(), 9000);
        assert_eq!(tag(&event, "h"), ephemeral.to_string());
        assert_eq!(tag(&event, "p"), talos);
        assert_eq!(tag(&event, "role"), "member");
    }

    #[tokio::test]
    async fn a_failed_add_names_what_failed() {
        let (base, _seen) = fake_server(500, "nope").await;
        let state = test_state();
        *state.relay_url_override.lock().unwrap() = Some(base.replacen("http", "ws", 1));
        state.huddle().unwrap().ephemeral_channel_id = Some(Uuid::new_v4().to_string());
        state.huddle().unwrap().phase = HuddlePhase::Active;

        let error = add_to_huddle(&"ab".repeat(32), &state).await.unwrap_err();
        assert!(error.starts_with("Talos couldn't be added"), "{error}");
    }
}
