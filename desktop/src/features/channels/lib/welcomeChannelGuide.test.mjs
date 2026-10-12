import assert from "node:assert/strict";
import test from "node:test";

import {
  canBuildCommunity,
  selectWelcomeChannelGuide,
  showsCreateActions,
} from "./welcomeChannelGuide.ts";

function channel(overrides = {}) {
  return {
    archivedAt: null,
    channelType: "stream",
    description: "",
    id: overrides.name ?? "chan",
    isMember: true,
    name: "chan",
    purpose: null,
    topic: null,
    ttlDeadline: null,
    ttlSeconds: null,
    visibility: "open",
    ...overrides,
  };
}

test("selectWelcomeChannelGuide keeps only channels the viewer can open", () => {
  const welcome = channel({ name: "welcome-everyone" });
  const channels = [
    welcome,
    channel({ name: "general", description: "Community updates." }),
    channel({ name: "lounge", isMember: false }),
    channel({ name: "client-room", visibility: "private" }),
    channel({ name: "bench", visibility: "private", isMember: false }),
    channel({ name: "old", archivedAt: "2026-01-01T00:00:00Z" }),
    channel({ name: "dm-1", channelType: "dm", visibility: "private" }),
    channel({ name: "ideas", channelType: "forum", topic: "Long posts." }),
  ];

  assert.deepEqual(selectWelcomeChannelGuide(channels, welcome.id), [
    { description: null, id: "client-room", name: "client-room" },
    { description: "Community updates.", id: "general", name: "general" },
    { description: "Long posts.", id: "ideas", name: "ideas" },
    { description: null, id: "lounge", name: "lounge" },
  ]);
});

test("selectWelcomeChannelGuide leaves out temporary channels such as huddle rooms", () => {
  const channels = [
    channel({ name: "general" }),
    // A huddle room: private, temporary, and the viewer is in it.
    channel({ name: "huddle-1a2b", visibility: "private", ttlSeconds: 3600 }),
    channel({ name: "launch-week", ttlDeadline: "2026-01-08T00:00:00Z" }),
  ];

  assert.deepEqual(
    selectWelcomeChannelGuide(channels, "welcome").map((entry) => entry.name),
    ["general"],
  );
});

test("selectWelcomeChannelGuide uses the description before topic and purpose", () => {
  const [entry] = selectWelcomeChannelGuide(
    [
      channel({
        description: "What the channel is for.",
        name: "reset",
        purpose: "Purpose",
        topic: "Topic",
      }),
    ],
    "welcome",
  );

  assert.equal(entry.description, "What the channel is for.");
});

test("selectWelcomeChannelGuide is empty when the welcome channel is alone", () => {
  assert.deepEqual(
    selectWelcomeChannelGuide([channel({ name: "welcome" })], "welcome"),
    [],
  );
});

test("canBuildCommunity keeps the create cards for owners, admins and open relays", () => {
  const membership = (role) => ({
    addedBy: null,
    createdAt: "2026-01-01T00:00:00Z",
    pubkey: "a".repeat(64),
    role,
  });
  const cases = [
    ["unknown", undefined, false],
    [
      "open relay",
      { membershipRequired: false, membership: null, snapshotFound: false },
      true,
    ],
    [
      "owner",
      {
        membershipRequired: true,
        membership: membership("owner"),
        snapshotFound: true,
      },
      true,
    ],
    [
      "admin",
      {
        membershipRequired: true,
        membership: membership("admin"),
        snapshotFound: true,
      },
      true,
    ],
    [
      "member",
      {
        membershipRequired: true,
        membership: membership("member"),
        snapshotFound: true,
      },
      false,
    ],
    [
      "not in the list",
      { membershipRequired: true, membership: null, snapshotFound: true },
      false,
    ],
    [
      "no snapshot",
      { membershipRequired: true, membership: null, snapshotFound: false },
      false,
    ],
  ];

  for (const [name, lookup, expected] of cases) {
    assert.equal(canBuildCommunity(lookup), expected, name);
  }
});

test("showsCreateActions hides the create cards from guests only in #welcome-everyone", () => {
  assert.equal(showsCreateActions(true, false), false);
  assert.equal(showsCreateActions(true, true), true);
  assert.equal(showsCreateActions(false, false), true);
  assert.equal(showsCreateActions(false, true), true);
});
