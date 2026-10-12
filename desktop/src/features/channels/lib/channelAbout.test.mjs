import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import {
  CHANNEL_ABOUT_SEEN_LIMIT,
  hasChannelCanvasContent,
  hasSeenChannelAbout,
  markChannelAboutSeen,
  shouldAutoOpenChannelAbout,
} from "./channelAbout.ts";

const STORAGE_KEY = "buzz-channel-about-seen.v1";
let restoreWindow = () => {};
let values = new Map();
let nextChannel = 0;

function freshChannelId() {
  nextChannel += 1;
  return `channel-${nextChannel}-${Math.random().toString(16).slice(2)}`;
}

function installStorage(storage) {
  const hadWindow = "window" in globalThis;
  const previous = globalThis.window;
  globalThis.window = { localStorage: storage };
  restoreWindow = () => {
    if (hadWindow) globalThis.window = previous;
    else delete globalThis.window;
  };
}

beforeEach(() => {
  values = new Map();
  installStorage({
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  });
});

afterEach(() => restoreWindow());

test("only a canvas with visible text counts as content", () => {
  assert.equal(hasChannelCanvasContent(undefined), false);
  assert.equal(hasChannelCanvasContent(null), false);
  assert.equal(hasChannelCanvasContent({ content: null }), false);
  assert.equal(hasChannelCanvasContent({ content: "" }), false);
  assert.equal(hasChannelCanvasContent({ content: " \n\t " }), false);
  assert.equal(hasChannelCanvasContent({ content: "# Start here" }), true);
});

test("auto-open needs content, a first visit and a free side panel", () => {
  for (const hasContent of [false, true]) {
    for (const seen of [false, true]) {
      for (const otherPanelOpen of [false, true]) {
        assert.equal(
          shouldAutoOpenChannelAbout({ hasContent, otherPanelOpen, seen }),
          hasContent && !seen && !otherPanelOpen,
          JSON.stringify({ hasContent, otherPanelOpen, seen }),
        );
      }
    }
  }
});

test("a channel is remembered per channel once marked", () => {
  const first = freshChannelId();
  const second = freshChannelId();
  assert.equal(hasSeenChannelAbout(first), false);

  markChannelAboutSeen(first);

  assert.equal(hasSeenChannelAbout(first), true);
  assert.equal(hasSeenChannelAbout(second), false);
  assert.deepEqual(JSON.parse(values.get(STORAGE_KEY)), [first]);
});

test("the seen list survives a new session through storage", () => {
  const stored = freshChannelId();
  values.set(STORAGE_KEY, JSON.stringify([stored]));
  assert.equal(hasSeenChannelAbout(stored), true);
});

test("marking again moves a channel to the newest end without duplicates", () => {
  const first = freshChannelId();
  const second = freshChannelId();
  markChannelAboutSeen(first);
  markChannelAboutSeen(second);
  markChannelAboutSeen(first);
  assert.deepEqual(JSON.parse(values.get(STORAGE_KEY)), [second, first]);
});

test("the seen list keeps only the most recent channels", () => {
  const old = Array.from(
    { length: CHANNEL_ABOUT_SEEN_LIMIT },
    (_, index) => `old-${index}`,
  );
  values.set(STORAGE_KEY, JSON.stringify(old));
  const newest = freshChannelId();

  markChannelAboutSeen(newest);

  const stored = JSON.parse(values.get(STORAGE_KEY));
  assert.equal(stored.length, CHANNEL_ABOUT_SEEN_LIMIT);
  assert.equal(stored[0], "old-1");
  assert.equal(stored.at(-1), newest);
});

test("unreadable stored values count as nothing seen", () => {
  const channelId = freshChannelId();
  values.set(STORAGE_KEY, "not json");
  assert.equal(hasSeenChannelAbout(channelId), false);
  values.set(STORAGE_KEY, JSON.stringify({ [channelId]: true }));
  assert.equal(hasSeenChannelAbout(channelId), false);

  markChannelAboutSeen(channelId);
  assert.deepEqual(JSON.parse(values.get(STORAGE_KEY)), [channelId]);
});

test("a refused write still keeps the panel from reopening this session", (t) => {
  t.mock.method(console, "warn", () => {});
  installStorage({
    getItem: () => {
      throw new Error("SecurityError");
    },
    setItem: () => {
      throw new Error("SecurityError");
    },
    removeItem: () => {},
  });
  const channelId = freshChannelId();
  assert.equal(hasSeenChannelAbout(channelId), false);

  markChannelAboutSeen(channelId);

  assert.equal(hasSeenChannelAbout(channelId), true);
});
