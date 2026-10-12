import assert from "node:assert/strict";
import test from "node:test";

import {
  commitGuestEntry,
  joinGuestStarterChannels,
  prepareGuestEntry,
} from "./guestEntry.ts";
import {
  clearCommunityGuest,
  guestLandingHash,
  hasJoinedGuestStarterChannels,
  isCommunityGuest,
  isGuestAgentHintDone,
  isGuestStarterChannel,
  isInviteJoin,
  joinsAsGuest,
  markCommunityGuest,
  markGuestAgentHintDone,
  markGuestStarterChannelsJoined,
  pickGuestLandingChannel,
  withJoinedStarterChannels,
} from "./guestMode.ts";

function createMemoryStorage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, String(value)),
  };
}

const REFUSING_STORAGE = {
  getItem: () => {
    throw new Error("storage unavailable");
  },
  removeItem: () => {
    throw new Error("storage unavailable");
  },
  setItem: () => {
    throw new Error("quota exceeded");
  },
};

/** Silences the expected warnings of a test that exercises failures. */
function quietWarnings(t) {
  t.mock.method(console, "warn", () => {});
}

function channel(overrides = {}) {
  return {
    archivedAt: null,
    channelType: "stream",
    description: "",
    id: overrides.name ?? "chan",
    isMember: true,
    lastMessageAt: null,
    memberCount: 2,
    memberPubkeys: [],
    name: "chan",
    participantPubkeys: [],
    participants: [],
    purpose: null,
    topic: null,
    ttlDeadline: null,
    ttlSeconds: null,
    visibility: "open",
    ...overrides,
  };
}

const INVITE = { inviteCode: "abc.def" };

test("isInviteJoin is true only when the onboarding carries an invite code", () => {
  const cases = [
    [null, false],
    [undefined, false],
    [{}, false],
    [{ inviteCode: undefined }, false],
    [{ inviteCode: "" }, false],
    [{ inviteCode: "   " }, false],
    [{ inviteCode: "abc.def" }, true],
    [{ inviteCode: "  abc.def  " }, true],
  ];
  for (const [transaction, expected] of cases) {
    assert.equal(
      isInviteJoin(transaction),
      expected,
      JSON.stringify(transaction),
    );
  }
});

test("joinsAsGuest admits invite joins unless the relay lists an owner or admin", () => {
  const transactions = [
    [null, false],
    [{ inviteCode: "" }, false],
    [INVITE, true],
  ];
  const roles = [
    [null, true],
    ["member", true],
    ["admin", false],
    ["owner", false],
  ];
  for (const [transaction, isInvite] of transactions) {
    for (const [role, roleAllowsGuest] of roles) {
      assert.equal(
        joinsAsGuest(transaction, role),
        isInvite && roleAllowsGuest,
        `${JSON.stringify(transaction)} as ${role}`,
      );
    }
  }
});

test("guest mode is remembered per identity and community", () => {
  const storage = createMemoryStorage();
  markCommunityGuest("PUBKEY-A", "wss://Hive.Example.com/", storage);

  assert.equal(
    isCommunityGuest("pubkey-a", "wss://hive.example.com", storage),
    true,
  );
  assert.equal(
    isCommunityGuest("pubkey-a", "wss://other.example.com", storage),
    false,
  );
  assert.equal(
    isCommunityGuest("pubkey-b", "wss://hive.example.com", storage),
    false,
  );
  assert.equal(
    isCommunityGuest(null, "wss://hive.example.com", storage),
    false,
  );
  assert.equal(isCommunityGuest("pubkey-a", null, storage), false);
});

test("an unreadable store means not a guest, and a refused write throws", () => {
  assert.equal(
    isCommunityGuest("pubkey-a", "wss://hive.example.com", REFUSING_STORAGE),
    false,
  );
  assert.throws(
    () =>
      markCommunityGuest(
        "pubkey-a",
        "wss://hive.example.com",
        REFUSING_STORAGE,
      ),
    /quota exceeded/,
  );
});

test("guest mode can be cleared for one identity and community", (t) => {
  const storage = createMemoryStorage();
  markCommunityGuest("pubkey-a", "wss://hive.example.com", storage);
  markCommunityGuest("pubkey-a", "wss://other.example.com", storage);

  clearCommunityGuest("PUBKEY-A", "wss://Hive.Example.com/", storage);

  assert.equal(
    isCommunityGuest("pubkey-a", "wss://hive.example.com", storage),
    false,
  );
  assert.equal(
    isCommunityGuest("pubkey-a", "wss://other.example.com", storage),
    true,
  );
  // A refused removal keeps the record for the next role check to clear.
  quietWarnings(t);
  assert.doesNotThrow(() =>
    clearCommunityGuest("pubkey-a", "wss://hive.example.com", REFUSING_STORAGE),
  );
});

test("the starter-join and agent-hint records are kept per identity and community", (t) => {
  quietWarnings(t);
  const records = [
    [hasJoinedGuestStarterChannels, markGuestStarterChannelsJoined],
    [isGuestAgentHintDone, markGuestAgentHintDone],
  ];
  for (const [isDone, markDone] of records) {
    const storage = createMemoryStorage();
    assert.equal(isDone("pubkey-a", "wss://hive.example.com", storage), false);
    markDone("PUBKEY-A", "wss://Hive.Example.com/", storage);
    assert.equal(isDone("pubkey-a", "wss://hive.example.com", storage), true);
    assert.equal(isDone("pubkey-b", "wss://hive.example.com", storage), false);
    assert.equal(isDone("pubkey-a", "wss://other.example.com", storage), false);
    assert.equal(isDone(null, "wss://hive.example.com", storage), false);
    // Losing either record only repeats harmless work, so a refused write is
    // not an error.
    assert.doesNotThrow(() =>
      markDone("pubkey-a", "wss://hive.example.com", REFUSING_STORAGE),
    );
    assert.equal(
      isDone("pubkey-a", "wss://hive.example.com", REFUSING_STORAGE),
      false,
    );
  }
  // Neither record marks someone as a guest.
  const storage = createMemoryStorage();
  markGuestStarterChannelsJoined("pubkey-a", "wss://hive.example.com", storage);
  markGuestAgentHintDone("pubkey-a", "wss://hive.example.com", storage);
  assert.equal(
    isCommunityGuest("pubkey-a", "wss://hive.example.com", storage),
    false,
  );
});

test("a guest lands in the most recently active channel the owner set up", () => {
  const channels = [
    channel({ name: "general", lastMessageAt: "2026-10-12T10:00:00Z" }),
    channel({
      name: "welcome-everyone",
      lastMessageAt: "2026-10-12T11:00:00Z",
    }),
    channel({ name: "craig-ask-here", lastMessageAt: "2026-10-10T09:00:00Z" }),
    channel({ name: "reset-week-1", lastMessageAt: "2026-10-11T09:00:00Z" }),
    channel({ name: "quiet-room" }),
  ];

  assert.equal(pickGuestLandingChannel(channels)?.name, "reset-week-1");
});

test("without activity the first channel the guest belongs to wins", () => {
  const channels = [
    channel({ name: "general" }),
    channel({ name: "client-room", visibility: "private" }),
    channel({ name: "notes" }),
  ];

  assert.equal(pickGuestLandingChannel(channels)?.name, "client-room");
  assert.equal(
    pickGuestLandingChannel([
      channel({ name: "client-room", lastMessageAt: "not a date" }),
      channel({ name: "notes" }),
    ])?.name,
    "client-room",
  );
});

test("channels a guest cannot land in are skipped", () => {
  const welcomeEveryone = channel({ name: "welcome-everyone" });
  const skipped = [
    channel({ name: "lounge", isMember: false }),
    channel({ name: "old", archivedAt: "2026-01-01T00:00:00Z" }),
    channel({ name: "dm-1", channelType: "dm", visibility: "private" }),
    channel({ name: "huddle-1a2b", visibility: "private", ttlSeconds: 3600 }),
    channel({ name: "Welcome", visibility: "private" }),
    channel({ name: "General", lastMessageAt: "2026-10-12T10:00:00Z" }),
  ];

  assert.equal(
    pickGuestLandingChannel([...skipped, welcomeEveryone])?.name,
    "welcome-everyone",
  );
});

test("with only the starter channels a guest lands in #welcome-everyone, else Inbox", () => {
  assert.equal(
    pickGuestLandingChannel([
      channel({ name: "general" }),
      channel({ name: "welcome-everyone" }),
    ])?.name,
    "welcome-everyone",
  );
  // Guest entry joins #welcome-everyone first; one the guest is not in is no
  // landing place.
  assert.equal(
    pickGuestLandingChannel([
      channel({ name: "general" }),
      channel({ name: "welcome-everyone", isMember: false }),
    ]),
    null,
  );
  assert.equal(pickGuestLandingChannel([channel({ name: "general" })]), null);
  assert.equal(pickGuestLandingChannel([]), null);
});

test("only the community's open, unarchived #general and #welcome-everyone are starters", () => {
  const starters = [
    channel({ name: "general" }),
    channel({ name: "Welcome-Everyone" }),
    channel({ name: " general ", isMember: false }),
  ];
  const others = [
    channel({ name: "general", visibility: "private" }),
    channel({ name: "welcome-everyone", channelType: "forum" }),
    channel({ name: "general", archivedAt: "2026-01-01T00:00:00Z" }),
    channel({ name: "Welcome", visibility: "private" }),
    channel({ name: "craig-ask-here" }),
  ];
  for (const starter of starters) {
    assert.equal(isGuestStarterChannel(starter), true, starter.name);
  }
  for (const other of others) {
    assert.equal(isGuestStarterChannel(other), false, JSON.stringify(other));
  }
});

test("joined starter channels are added to a channel list that trails the join", () => {
  const listed = channel({ id: "general-id", name: "general" });
  const merged = withJoinedStarterChannels(
    [listed, channel({ id: "client-id", name: "client-room" })],
    [
      channel({ id: "general-id", name: "general", isMember: false }),
      channel({ id: "welcome-id", name: "welcome-everyone", isMember: false }),
    ],
  );
  assert.deepEqual(
    merged.map((entry) => [entry.id, entry.isMember]),
    [
      ["general-id", true],
      ["client-id", true],
      ["welcome-id", true],
    ],
  );
  // The listed copy wins, keeping its latest activity.
  assert.equal(merged[0], listed);
});

test("guestLandingHash opens the channel, or Inbox when there is none", () => {
  assert.equal(
    guestLandingHash(channel({ id: "a b/c" })),
    "/channels/a%20b%2Fc",
  );
  assert.equal(guestLandingHash(null), "/");
});

function lookups({
  role = null,
  channels = [],
  starters = [],
  timeoutMs = 1_000,
} = {}) {
  const calls = { channels: 0, role: 0, starters: 0 };
  const order = [];
  return {
    calls,
    order,
    lookups: {
      joinStarterChannels: async () => {
        calls.starters += 1;
        order.push("join starters");
        return typeof starters === "function" ? starters() : starters;
      },
      loadChannels: async () => {
        calls.channels += 1;
        order.push("load channels");
        return typeof channels === "function" ? channels() : channels;
      },
      loadRole: async () => {
        calls.role += 1;
        return typeof role === "function" ? role() : role;
      },
      timeoutMs,
    },
  };
}

test("prepareGuestEntry asks the relay nothing for people who did not use an invite", async () => {
  const { calls, lookups: stub } = lookups({ role: "member" });
  assert.equal(await prepareGuestEntry({}, stub), null);
  assert.equal(await prepareGuestEntry(null, stub), null);
  assert.deepEqual(calls, { channels: 0, role: 0, starters: 0 });
});

test("a brand-new guest joins the starter channels and lands in #welcome-everyone", async () => {
  // A new key belongs to no channel yet, and the relay's member list has not
  // caught up with the joins.
  const { order, lookups: stub } = lookups({
    role: "member",
    channels: [],
    starters: [
      channel({ id: "general-id", name: "general" }),
      channel({ id: "welcome-id", name: "welcome-everyone" }),
    ],
  });
  assert.deepEqual(await prepareGuestEntry(INVITE, stub), {
    landingHash: "/channels/welcome-id",
    startersJoined: true,
  });
  assert.deepEqual(order, ["join starters", "load channels"]);
});

test("the owner's channel for the guest still wins over the starter channels", async () => {
  const entry = await prepareGuestEntry(
    INVITE,
    lookups({
      role: "member",
      channels: [channel({ id: "client-id", name: "client-room" })],
      starters: [channel({ id: "welcome-id", name: "welcome-everyone" })],
    }).lookups,
  );
  assert.deepEqual(entry, {
    landingHash: "/channels/client-id",
    startersJoined: true,
  });
});

test("a failed or slow starter join still lands the guest and is left to retry", async (t) => {
  quietWarnings(t);
  const failing = lookups({
    role: "member",
    channels: [channel({ id: "client-id", name: "client-room" })],
    starters: () => Promise.reject(new Error("join refused")),
  }).lookups;
  assert.deepEqual(await prepareGuestEntry(INVITE, failing), {
    landingHash: "/channels/client-id",
    startersJoined: false,
  });

  const hanging = lookups({
    role: "member",
    channels: [channel({ id: "client-id", name: "client-room" })],
    starters: () => new Promise(() => {}),
    timeoutMs: 5,
  });
  assert.deepEqual(await prepareGuestEntry(INVITE, hanging.lookups), {
    landingHash: "/channels/client-id",
    startersJoined: false,
  });
  // The channel list still gets its own time after the join gives up.
  assert.equal(hanging.calls.channels, 1);
});

test("prepareGuestEntry keeps owners and admins out of guest mode", async () => {
  for (const role of ["owner", "admin"]) {
    assert.equal(
      await prepareGuestEntry(INVITE, lookups({ role }).lookups),
      null,
      role,
    );
  }
});

test("prepareGuestEntry lands an invited member in their channel", async () => {
  const entry = await prepareGuestEntry(
    INVITE,
    lookups({
      role: "member",
      channels: [
        channel({ id: "general-id", name: "general" }),
        channel({ id: "client-id", name: "client-room" }),
      ],
    }).lookups,
  );
  assert.deepEqual(entry, {
    landingHash: "/channels/client-id",
    startersJoined: true,
  });
});

test("prepareGuestEntry falls back to the invite and Inbox when the relay fails", async (t) => {
  quietWarnings(t);
  const unreachable = () => Promise.reject(new Error("relay unreachable"));
  const failing = lookups({
    role: unreachable,
    channels: unreachable,
    starters: unreachable,
  }).lookups;
  assert.deepEqual(await prepareGuestEntry(INVITE, failing), {
    landingHash: "/",
    startersJoined: false,
  });

  const never = () => new Promise(() => {});
  const hanging = lookups({
    role: never,
    channels: never,
    starters: never,
    timeoutMs: 5,
  }).lookups;
  assert.deepEqual(await prepareGuestEntry(INVITE, hanging), {
    landingHash: "/",
    startersJoined: false,
  });
});

function rawChannel(overrides = {}) {
  return {
    archived_at: null,
    channel_type: "stream",
    description: "",
    id: overrides.name ?? "chan",
    is_member: false,
    last_message_at: null,
    member_count: 2,
    member_pubkeys: [],
    name: "chan",
    participant_pubkeys: [],
    participants: [],
    purpose: null,
    topic: null,
    ttl_deadline: null,
    ttl_seconds: null,
    visibility: "open",
    ...overrides,
  };
}

function installTauriInvoke(t, handler) {
  const prior = globalThis.window;
  t.after(() => {
    globalThis.window = prior;
  });
  globalThis.window = { __TAURI_INTERNALS__: { invoke: handler } };
}

test("joinGuestStarterChannels joins only the starters the guest is not in, creating nothing", async (t) => {
  const commands = [];
  installTauriInvoke(t, async (command, args) => {
    commands.push([command, args?.channelId ?? null]);
    if (command === "get_open_channel_directory") {
      return [
        rawChannel({ id: "general-id", name: "general" }),
        rawChannel({ id: "welcome-id", name: "welcome-everyone" }),
        rawChannel({ id: "random-id", name: "random" }),
        rawChannel({
          id: "old-id",
          name: "general",
          archived_at: "2026-01-01T00:00:00Z",
        }),
        rawChannel({
          id: "member-id",
          name: "welcome-everyone",
          is_member: true,
        }),
      ];
    }
    return null;
  });

  const starters = await joinGuestStarterChannels();

  assert.deepEqual(
    starters.map((entry) => [entry.id, entry.isMember]),
    [
      ["general-id", true],
      ["welcome-id", true],
      ["member-id", true],
    ],
  );
  assert.deepEqual(commands, [
    ["get_open_channel_directory", null],
    ["join_channel", "general-id"],
    ["join_channel", "welcome-id"],
  ]);
});

test("joinGuestStarterChannels fails when a join fails, so it is tried again", async (t) => {
  installTauriInvoke(t, async (command) => {
    if (command === "get_open_channel_directory") {
      return [rawChannel({ id: "welcome-id", name: "welcome-everyone" })];
    }
    throw new Error("relay rejected event");
  });

  await assert.rejects(joinGuestStarterChannels(), /relay rejected event/);
});

test("commitGuestEntry records guest mode, a completed starter join and the landing", (t) => {
  const prior = {
    localStorage: globalThis.localStorage,
    window: globalThis.window,
  };
  t.after(() => {
    globalThis.localStorage = prior.localStorage;
    globalThis.window = prior.window;
  });
  globalThis.localStorage = createMemoryStorage();
  globalThis.window = { location: { hash: "" } };

  commitGuestEntry(
    { landingHash: "/channels/welcome-id", startersJoined: false },
    "pubkey-a",
    "wss://hive.example.com",
  );
  assert.equal(isCommunityGuest("pubkey-a", "wss://hive.example.com"), true);
  assert.equal(
    hasJoinedGuestStarterChannels("pubkey-a", "wss://hive.example.com"),
    false,
  );
  assert.equal(globalThis.window.location.hash, "/channels/welcome-id");

  commitGuestEntry(
    { landingHash: "/", startersJoined: true },
    "pubkey-a",
    "wss://hive.example.com",
  );
  assert.equal(
    hasJoinedGuestStarterChannels("pubkey-a", "wss://hive.example.com"),
    true,
  );
});
