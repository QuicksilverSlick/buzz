/**
 * The agent panel reads the harness's own durable paused state when the
 * observer event window has nothing to say.
 *
 * That window is the only thing the panel used to read: it is capped,
 * trimmed once it fills, and empty after a restart — and `lastError` stays
 * null for a parked harness, because a parked harness keeps running and
 * never exits with an error. So for most of a multi-day outage the panel
 * showed nothing at all, which is the one surface M1 step 5 exists to fill.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

const tauriMock = {
  invoke() {
    return Promise.reject(new Error("no Tauri backend in this test"));
  },
  transformCallback() {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { useAgentLastError } = await import("./useAgentLastError.ts");

const PUBKEY = "ab".repeat(32);

const weeklyPaused = {
  class: "weekly_limit",
  text: "You've hit your weekly limit · resets Oct 18, 7pm (America/Chicago)",
  provider: "Claude",
  state: "BLOCKED",
  resetsAt: "2026-10-19T00:00:00Z",
  resumesAt: "2026-10-19T00:02:00Z",
  since: "2026-10-16T16:52:00Z",
  expires: "2026-10-19T00:07:00Z",
};

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

/** Render the real hook for one agent and return what the panel would show. */
function shown(agent) {
  let seen = null;
  function Probe() {
    seen = useAgentLastError(agent);
    return null;
  }
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(React.createElement(Probe)));
  return seen;
}

test("a running agent with an empty event window still shows why it is paused", () => {
  const seen = shown({
    pubkey: PUBKEY,
    status: "running",
    lastError: null,
    lastErrorCode: null,
    paused: weeklyPaused,
  });
  assert.equal(seen?.severity, "generic");
  assert.match(seen?.copy ?? "", /^Paused: Claude weekly limit · resets /);
});

test("a paused agent that was stopped still says what stopped it", () => {
  const seen = shown({
    pubkey: PUBKEY,
    status: "stopped",
    lastError: null,
    lastErrorCode: null,
    paused: {
      ...weeklyPaused,
      class: "sign_in",
      state: "DOWN",
      resetsAt: null,
    },
  });
  assert.equal(seen?.severity, "denied");
  assert.match(seen?.copy ?? "", /^Paused: Claude sign-in/);
});

test("no paused state falls through to the error the agent exited with", () => {
  const seen = shown({
    pubkey: PUBKEY,
    status: "stopped",
    lastError: "harness exited with status code 137",
    lastErrorCode: null,
    paused: null,
  });
  assert.deepEqual(seen, {
    severity: "generic",
    copy: "harness exited with status code 137",
  });
});

test("nothing wrong shows nothing", () => {
  assert.equal(
    shown({
      pubkey: PUBKEY,
      status: "running",
      lastError: null,
      lastErrorCode: null,
      paused: null,
    }),
    null,
  );
});
