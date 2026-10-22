import assert from "node:assert/strict";
import {
  after,
  afterEach,
  before,
  beforeEach,
  describe,
  it,
  mock,
} from "node:test";

import { JSDOM } from "jsdom";

/**
 * A member who joins, or edits their profile, while the app is open must reach
 * every picker without a restart. These cases drive the production
 * `startLiveDirectoryUpdates` with a fake relay socket and a real QueryClient,
 * and check which caches each live event marks for refetch.
 */

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const NEW_MEMBER = "a".repeat(64);
const OTHER = "b".repeat(64);
const VIEWER = "c".repeat(64);
const DIRECTORY = ["user-search", "infinite", "", 50];

let QueryClient;
let start;
let windowMs;

before(async () => {
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      throw new Error(`Unexpected command in this test: ${command}`);
    },
  };
  Object.assign(globalThis, {
    document: dom.window.document,
    localStorage: dom.window.localStorage,
    window: dom.window,
  });
  ({ QueryClient } = await import("@tanstack/react-query"));
  ({ startLiveDirectoryUpdates: start, DIRECTORY_REFRESH_WINDOW_MS: windowMs } =
    await import("./useLiveDirectoryUpdates.ts"));
});

after(() => dom.window.close());

function fakeRelay({ failures = 0 } = {}) {
  const relay = { attempts: 0, filter: null, onEvent: null, unsubscribed: 0 };
  relay.subscribeLive = async (filter, onEvent) => {
    relay.attempts += 1;
    if (relay.attempts <= failures) throw new Error("relay unreachable");
    relay.filter = filter;
    relay.onEvent = onEvent;
    return async () => {
      relay.unsubscribed += 1;
    };
  };
  return relay;
}

function event(kind, pubkey, { createdAt = 100, tags = [] } = {}) {
  return {
    content: "",
    created_at: createdAt,
    id: "e".repeat(64),
    kind,
    pubkey,
    sig: "f".repeat(128),
    tags,
  };
}

function snapshot(createdAt, members) {
  return event(13534, "f".repeat(64), {
    createdAt,
    tags: members.map(([pubkey, role]) => ["member", pubkey, role]),
  });
}

const invalidated = (client, key) =>
  client.getQueryState(key)?.isInvalidated ?? false;

const settle = () => new Promise((resolve) => setImmediate(resolve));

describe("startLiveDirectoryUpdates", () => {
  let client;
  let relay;
  let stop;

  beforeEach(async () => {
    mock.timers.enable({ apis: ["setTimeout"] });
    client = new QueryClient();
    for (const key of [
      ["user-search", "nova", 8],
      ["user-profile", NEW_MEMBER],
      ["user-profile", OTHER],
      ["users-batch", NEW_MEMBER, OTHER],
      ["users-batch", OTHER],
      ["users-batch-entry", NEW_MEMBER],
    ]) {
      client.setQueryData(key, {});
    }
    // A directory scrolled three pages deep.
    client.setQueryData(DIRECTORY, {
      pages: [{ users: [] }, { users: [] }, { users: [] }],
      pageParams: [null, "2", "3"],
    });
    const viewer = { pubkey: VIEWER, role: "member" };
    client.setQueryData(["myRelayMembership"], viewer);
    client.setQueryData(["myRelayMembershipLookup"], {
      membership: viewer,
      membershipRequired: true,
      snapshotFound: true,
    });
    relay = fakeRelay();
    stop = start(client, VIEWER, relay);
    await settle();
  });

  afterEach(() => {
    stop();
    client.clear();
    mock.timers.reset();
  });

  it("subscribes live-only to profiles and the membership snapshot", () => {
    assert.deepEqual(relay.filter, { kinds: [0, 13534], limit: 0 });
  });

  it("refreshes the directory and that member's caches after the window", () => {
    // Uppercase on the wire still has to match the lowercase cache keys.
    relay.onEvent(event(0, NEW_MEMBER.toUpperCase()));
    mock.timers.tick(windowMs - 1);
    assert.equal(invalidated(client, ["user-search", "nova", 8]), false);

    mock.timers.tick(1);

    assert.ok(invalidated(client, DIRECTORY));
    assert.ok(invalidated(client, ["user-search", "nova", 8]));
    assert.ok(invalidated(client, ["user-profile", NEW_MEMBER]));
    assert.ok(invalidated(client, ["users-batch", NEW_MEMBER, OTHER]));
    // The per-key entry is evicted, so users-batch cannot re-resolve stale.
    assert.equal(
      client.getQueryData(["users-batch-entry", NEW_MEMBER]),
      undefined,
    );
    // Unrelated people and the viewer's own role are left alone.
    assert.equal(invalidated(client, ["user-profile", OTHER]), false);
    assert.equal(invalidated(client, ["users-batch", OTHER]), false);
    assert.equal(invalidated(client, ["myRelayMembership"]), false);
  });

  it("refetches a scrolled directory as one page, not every page", () => {
    relay.onEvent(event(0, NEW_MEMBER));
    mock.timers.tick(windowMs);

    assert.deepEqual(client.getQueryData(DIRECTORY).pageParams, [null]);
  });

  it("does not let a burst postpone the refresh", () => {
    relay.onEvent(event(0, NEW_MEMBER));
    mock.timers.tick(windowMs / 2);
    relay.onEvent(event(0, OTHER));
    mock.timers.tick(windowMs / 2);

    // Both land in the refresh timed from the first event.
    assert.ok(invalidated(client, ["user-profile", NEW_MEMBER]));
    assert.ok(invalidated(client, ["user-profile", OTHER]));
  });

  it("writes each roster at once and refetches only a changed own role", () => {
    relay.onEvent(
      snapshot(200, [
        [VIEWER, "member"],
        [NEW_MEMBER, "member"],
      ]),
    );
    assert.deepEqual(
      client.getQueryData(["relayMembers"]).map((m) => m.pubkey),
      [VIEWER, NEW_MEMBER],
    );
    mock.timers.tick(windowMs);
    // A join alone costs no request: the newcomer's kind:0 refreshes the
    // directory, and the viewer's role did not move.
    assert.equal(invalidated(client, ["myRelayMembership"]), false);
    assert.equal(invalidated(client, DIRECTORY), false);

    relay.onEvent(
      snapshot(201, [
        [VIEWER, "admin"],
        [NEW_MEMBER, "member"],
      ]),
    );
    mock.timers.tick(windowMs);
    assert.ok(invalidated(client, ["myRelayMembership"]));
    assert.ok(invalidated(client, ["myRelayMembershipLookup"]));
  });

  it("ignores a roster older than the one already applied", () => {
    relay.onEvent(
      snapshot(200, [
        [VIEWER, "member"],
        [NEW_MEMBER, "member"],
      ]),
    );
    relay.onEvent(snapshot(150, [[VIEWER, "member"]]));

    assert.equal(client.getQueryData(["relayMembers"]).length, 2);
  });

  it("stops refreshing and unsubscribes when disposed", () => {
    relay.onEvent(event(0, NEW_MEMBER));
    stop();
    mock.timers.tick(windowMs);

    assert.equal(relay.unsubscribed, 1);
    assert.equal(invalidated(client, ["user-search", "nova", 8]), false);
    relay.onEvent(snapshot(300, [[VIEWER, "member"]]));
    assert.equal(client.getQueryData(["relayMembers"]), undefined);
  });

  it("retries a subscription that failed at launch", async (t) => {
    t.mock.method(console, "error", () => {});
    const flaky = fakeRelay({ failures: 2 });
    const stopFlaky = start(client, VIEWER, flaky);
    await settle();
    assert.equal(flaky.attempts, 1);

    mock.timers.tick(1_000);
    await settle();
    mock.timers.tick(2_000);
    await settle();

    assert.equal(flaky.attempts, 3);
    assert.equal(typeof flaky.onEvent, "function");
    stopFlaky();
    assert.equal(flaky.unsubscribed, 1);
  });
});
