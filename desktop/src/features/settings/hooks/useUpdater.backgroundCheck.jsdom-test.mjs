/**
 * A running app notices a published update by itself.
 *
 * Releases publish 1-2 h after every merge, but the background check ran
 * every 6 h, so the update button only ever appeared after a quit and
 * reopen. These tests pin the interval, the check on return to the window,
 * and that a huddle pop-out never runs its own check and download.
 */
import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

let checkCalls = 0;
const tauriMock = {
  metadata: { currentWindow: { label: "main" } },
  invoke(cmd) {
    if (cmd === "plugin:updater|check") {
      checkCalls += 1;
      return Promise.resolve(null);
    }
    return Promise.reject(new Error(`unexpected command ${cmd}`));
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
const { useUpdater } = await import("./use-updater.ts");

const MINUTE = 60 * 1000;
const T0 = 1_800_000_000_000;
const realDateNow = Date.now;
const realSetInterval = window.setInterval;
const realClearInterval = window.clearInterval;

let now;
let intervals;
let root;
let container;

beforeEach(() => {
  checkCalls = 0;
  now = T0;
  intervals = [];
  Date.now = () => now;
  window.setInterval = (callback, delay) => {
    intervals.push({ callback, delay });
    return intervals.length;
  };
  window.clearInterval = () => {};
});

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  Date.now = realDateNow;
  window.setInterval = realSetInterval;
  window.clearInterval = realClearInterval;
  delete globalThis.isTauri;
  tauriMock.metadata.currentWindow.label = "main";
});

/** Let the in-flight check settle so the next one is not swallowed. */
async function settle() {
  await act(async () => {
    for (let i = 0; i < 10; i += 1) await Promise.resolve();
  });
}

async function mount() {
  function Probe() {
    useUpdater();
    return null;
  }
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(React.createElement(Probe)));
  await settle();
}

test("checks again within 30 minutes while running", async () => {
  await mount();
  assert.equal(checkCalls, 1, "startup check");
  assert.equal(intervals.length, 1);
  assert.ok(
    intervals[0].delay <= 30 * MINUTE,
    `interval is ${intervals[0].delay / MINUTE} min`,
  );

  intervals[0].callback();
  await settle();
  assert.equal(checkCalls, 2);
});

test("returning to the window checks only after a long time away", async () => {
  await mount();
  assert.equal(checkCalls, 1);

  now = T0 + 29 * MINUTE;
  window.dispatchEvent(new window.Event("focus"));
  await settle();
  assert.equal(checkCalls, 1, "focus soon after a check stays quiet");

  now = T0 + 31 * MINUTE;
  window.dispatchEvent(new window.Event("focus"));
  await settle();
  assert.equal(checkCalls, 2, "focus after a long gap checks");
});

test("a huddle pop-out window never checks for updates", async () => {
  globalThis.isTauri = true;
  tauriMock.metadata.currentWindow.label =
    "huddle-0f8fad5b-d9cb-469f-a165-70867728950e";
  await mount();

  for (const { callback } of intervals) callback();
  now = T0 + 120 * MINUTE;
  window.dispatchEvent(new window.Event("focus"));
  await settle();
  assert.equal(checkCalls, 0);
});
