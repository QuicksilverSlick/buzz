import assert from "node:assert/strict";
import test from "node:test";

import { prepareGuestEntry } from "./guestEntry.ts";
import {
  guestLandingHash,
  isCommunityGuest,
  isInviteJoin,
  joinsAsGuest,
  markCommunityGuest,
  pickGuestLandingChannel,
} from "./guestMode.ts";

function createMemoryStorage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
  };
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
  const broken = {
    getItem: () => {
      throw new Error("storage unavailable");
    },
    setItem: () => {
      throw new Error("quota exceeded");
    },
  };
  assert.equal(
    isCommunityGuest("pubkey-a", "wss://hive.example.com", broken),
    false,
  );
  assert.throws(
    () => markCommunityGuest("pubkey-a", "wss://hive.example.com", broken),
    /quota exceeded/,
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
  // An open #welcome-everyone the guest has not joined yet still beats Inbox.
  assert.equal(
    pickGuestLandingChannel([
      channel({ name: "general" }),
      channel({ name: "welcome-everyone", isMember: false }),
    ])?.name,
    "welcome-everyone",
  );
  assert.equal(pickGuestLandingChannel([channel({ name: "general" })]), null);
  assert.equal(pickGuestLandingChannel([]), null);
});

test("guestLandingHash opens the channel, or Inbox when there is none", () => {
  assert.equal(
    guestLandingHash(channel({ id: "a b/c" })),
    "/channels/a%20b%2Fc",
  );
  assert.equal(guestLandingHash(null), "/");
});

function lookups({ role = null, channels = [], timeoutMs = 1_000 } = {}) {
  const calls = { channels: 0, role: 0 };
  return {
    calls,
    lookups: {
      loadChannels: async () => {
        calls.channels += 1;
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
  assert.deepEqual(calls, { channels: 0, role: 0 });
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
  assert.deepEqual(entry, { landingHash: "/channels/client-id" });
});

test("prepareGuestEntry falls back to the invite and Inbox when the relay fails", async () => {
  const failing = lookups({
    role: () => Promise.reject(new Error("relay unreachable")),
    channels: () => Promise.reject(new Error("relay unreachable")),
  }).lookups;
  assert.deepEqual(await prepareGuestEntry(INVITE, failing), {
    landingHash: "/",
  });

  const hanging = lookups({
    role: () => new Promise(() => {}),
    channels: () => new Promise(() => {}),
    timeoutMs: 5,
  }).lookups;
  assert.deepEqual(await prepareGuestEntry(INVITE, hanging), {
    landingHash: "/",
  });
});
