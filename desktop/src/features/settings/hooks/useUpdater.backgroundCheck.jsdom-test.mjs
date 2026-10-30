/**
 * A running app notices a published update by itself.
 *
 * Releases publish 1-2 h after every merge, but the background check ran
 * every 6 h, so the update button only ever appeared after a quit and
 * reopen. These tests pin the interval, the check on return to the window,
 * and that a huddle pop-out never runs its own check and download. A
 * downloaded update is re-checked against the feed so a rollback or a newer
 * release replaces it, and a failed download is not refetched every check.
 */
import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

let checkCalls = 0;
/** Version latest.json offers, or null when nothing is newer. */
let feed = null;
let checkGate = null;
let installGate = null;
let failDownload = false;
let nextRid = 1;
let downloads = [];
let installs = [];
let closed = [];
const tauriMock = {
  metadata: { currentWindow: { label: "main" } },
  async invoke(cmd, args) {
    switch (cmd) {
      case "plugin:updater|check":
        checkCalls += 1;
        await checkGate;
        return (
          feed && { rid: nextRid++, currentVersion: "0.5.40", version: feed }
        );
      case "is_auto_update_supported":
        return true;
      case "plugin:updater|download":
        downloads.push(args.rid);
        if (failDownload) throw new Error("signature verification failed");
        return nextRid++;
      case "plugin:updater|install":
        installs.push(args.updateRid);
        return installGate;
      case "plugin:resources|close":
        closed.push(args.rid);
        return;
      case "plugin:process|restart":
        return;
    }
    throw new Error(`unexpected command ${cmd}`);
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
let updater;

beforeEach(() => {
  checkCalls = 0;
  feed = null;
  checkGate = null;
  installGate = null;
  failDownload = false;
  downloads = [];
  installs = [];
  closed = [];
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
    await new Promise((resolve) => setImmediate(resolve));
  });
}

async function tick() {
  intervals[0].callback();
  await settle();
}

function deferred() {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

async function mount() {
  function Probe() {
    updater = useUpdater();
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

test("a downloaded update is dropped when the feed rolls back", async () => {
  feed = "0.5.41";
  await mount();
  assert.equal(updater.status.state, "ready");
  const [heldRid] = downloads;

  feed = null;
  await tick();
  assert.equal(updater.status.state, "idle", "the update button goes away");
  assert.ok(closed.includes(heldRid), "the held download is released");
});

test("a newer release replaces the downloaded one before the click", async () => {
  feed = "0.5.41";
  await mount();
  assert.equal(updater.status.state, "ready");
  const [heldRid] = downloads;

  await tick();
  assert.equal(downloads.length, 1, "the same version is not fetched again");
  assert.ok(!closed.includes(heldRid), "the held download is kept");
  assert.equal(updater.status.state, "ready");

  feed = "0.5.42";
  await tick();
  assert.equal(downloads.length, 2);
  assert.equal(updater.status.state, "ready");

  await act(() => updater.installAndRelaunch());
  assert.deepEqual(installs, [downloads[1]], "the click installs 0.5.42");
});

test("a re-check that lands mid-install keeps the update being installed", async () => {
  feed = "0.5.41";
  await mount();
  const [heldRid] = downloads;

  const check = deferred();
  checkGate = check.promise;
  intervals[0].callback();
  const install = deferred();
  installGate = install.promise;
  act(() => {
    void updater.installAndRelaunch();
  });
  await settle();
  assert.deepEqual(installs, [heldRid]);

  feed = "0.5.42";
  check.resolve();
  await settle();
  assert.equal(downloads.length, 1, "no new download during the install");
  assert.ok(!closed.includes(heldRid), "the installing update stays open");

  install.resolve();
  await settle();
});

test("a failed download is not refetched on every background check", async () => {
  failDownload = true;
  feed = "0.5.41";
  await mount();
  assert.equal(updater.status.state, "error");
  assert.equal(downloads.length, 1);

  now = T0 + 30 * MINUTE;
  await tick();
  now = T0 + 60 * MINUTE;
  await tick();
  assert.equal(checkCalls, 3);
  assert.equal(downloads.length, 1, "the failed version is not refetched");

  now = T0 + 6 * 60 * MINUTE;
  await tick();
  assert.equal(downloads.length, 2, "retried at the old 6 h cadence");
});
