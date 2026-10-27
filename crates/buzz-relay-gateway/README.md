# Buzz relay gateway

`buzz-relay-gateway` is a keyless forwarder between one Worker link and one
Buzz relay. It exists because a Cloudflare Worker cannot reach a relay that
sits behind Cloudflare itself (error 526), while a Cloudflare Container can.
The Worker keeps every key and signs every event; the gateway only checks the
link bearer and moves bytes.

## Trust boundary

- The gateway holds no secret. Its environment is four public values.
- The Worker opens `GET /v1/link` with `Authorization: Nostr <base64 event>`:
  a kind 27235 event signed by a dedicated gateway-auth key (never a relay
  member), `u` = the configured link URL, `method` = `GET`, within ±60 s, with
  one `nonce`. Each bearer works once, and a link lives at most one hour.
- The gateway authenticates the Worker by signature. The Worker authenticates
  the gateway only by transport (the Durable Object to container channel, or
  TLS on a VM).
- Only one relay origin is reachable, through an exact route table:
  `POST /query`, `/events`, `/count`, `GET /` (NIP-11) and the WebSocket on
  `/`. Request and response headers are allowlisted, redirects are not
  followed, nothing is retried, and bodies are never re-encoded, so NIP-98
  `payload` tags still verify at the relay.
- NIP-42 is transparent: the relay's `AUTH` challenge reaches the Worker as an
  ordinary `ws_msg`, and the Worker signs the reply.
- Logs carry ids, status codes, close codes and timings. They never carry
  bearers, bodies, relay frames or close-reason text.

The frames are in `src/frame.rs`, the route table in `src/policy.rs`, the
limits in `src/config.rs`, and the link close codes in `src/link.rs`.

## Environment

| Variable | Required | Value |
|---|---|---|
| `BUZZ_RELAY_GATEWAY_BIND_ADDR` | no | default `127.0.0.1:8080`; the image sets `0.0.0.0:8080` |
| `BUZZ_RELAY_GATEWAY_RELAY_URL` | yes | `wss://host[:port]`; `ws://` only on loopback |
| `BUZZ_RELAY_GATEWAY_LINK_URL` | yes | the `u` every link bearer must carry |
| `BUZZ_RELAY_GATEWAY_LINK_PUBKEY` | yes | the gateway-auth public key, 64 lowercase hex |
| `RUST_LOG` | no | `info`; `tungstenite` and `tokio_tungstenite` stay off at any level, since at trace they print whole frames |

`GET /healthz` answers `ok`, or 503 once shutdown has begun. SIGTERM or
Ctrl-C drains in-flight requests for up to 10 s, closes every relay socket,
then closes each link with 1012.

## Testing

```
cargo test -p buzz-relay-gateway
cargo clippy -p buzz-relay-gateway --all-targets -- -D warnings
```

The tests run offline against an in-process mock relay (HTTP routes and a
NIP-42 socket) and a mock Worker. They are not yet in the `Justfile` unit-test
list, so CI does not run them; run them locally on every change.

The image builds from the repo root. Pushing it is a deployment step, not a
build step:

```
docker build -f Dockerfile.relay-gateway --platform linux/amd64 -t buzz-relay-gateway:dev .
```
