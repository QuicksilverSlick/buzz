import assert from "node:assert/strict";
import test from "node:test";

import {
  DM_NON_MEMBER_MENTION_ERROR,
  DM_THREAD_MEMBERS_LOADING_ERROR,
} from "./dmThreadAgentMentionError.ts";
import {
  inboxReplyRecipientPubkeys,
  messageMentionPubkeys,
} from "./messageMentionPubkeys.ts";

function channel(overrides = {}) {
  return {
    id: "dm-1",
    name: "DM",
    channelType: "dm",
    visibility: "private",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: ["OWNER", "AGENT"],
    participantPubkeys: ["owner", "agent"],
    participants: [],
    lastMessageAt: null,
    archivedAt: null,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("plain DM messages p-tag every recipient except the sender", () => {
  assert.deepEqual(messageMentionPubkeys(channel(), "owner"), ["agent"]);
});

test("DM recipients and explicit mentions are normalized and deduplicated", () => {
  assert.deepEqual(
    messageMentionPubkeys(channel(), "OWNER", ["AGENT", "third"]),
    ["agent", "third"],
  );
});

test("stream messages preserve explicit-mention semantics", () => {
  assert.deepEqual(
    messageMentionPubkeys(
      channel({ channelType: "stream", memberPubkeys: ["owner", "agent"] }),
      "owner",
      [],
    ),
    [],
  );
});

const ready = (data) => ({ data, error: null });
const pending = { data: undefined, error: null };

function member(pubkey, overrides = {}) {
  return { pubkey, role: "member", isAgent: false, ...overrides };
}

function inboxReply(overrides = {}) {
  return inboxReplyRecipientPubkeys({
    channel: channel(),
    senderPubkey: "owner",
    explicitMentions: [],
    members: pending,
    managedAgents: pending,
    relayAgents: pending,
    knownAgentPubkeys: new Set(),
    ...overrides,
  });
}

// owner + agent-a (in the relay directory) + agent-b (an agent only by its
// member-list flag, as for another owner's agent that has not posted) + person.
const groupDm = channel({
  memberPubkeys: ["owner", "agent-a", "agent-b", "person"],
  participantPubkeys: ["owner", "agent-a", "agent-b", "person"],
});
const groupDmRoster = {
  channel: groupDm,
  members: ready([
    member("owner", { role: "owner" }),
    member("agent-a"),
    member("agent-b", { isAgent: true }),
    member("person"),
  ]),
  managedAgents: ready([]),
  relayAgents: ready([{ pubkey: "AGENT-A" }]),
};

test("an Inbox reply in a one-on-one agent DM tags the agent with no @", () => {
  // No roster or directory is needed: the only other participant is addressed.
  assert.deepEqual(inboxReply(), ["agent"]);
});

test("an Inbox reply in a group DM tags the person but no untyped agent", () => {
  assert.deepEqual(inboxReply(groupDmRoster), ["person"]);
});

test("a typed @agent in a group DM Inbox reply is tagged", () => {
  assert.deepEqual(
    inboxReply({ ...groupDmRoster, explicitMentions: ["AGENT-B"] }),
    ["agent-b", "person"],
  );
});

test("a group DM Inbox reply waits for the member list and agent directories", () => {
  for (const notReady of [
    { members: pending },
    { managedAgents: pending },
    { relayAgents: { data: [], error: new Error("relay down") } },
  ]) {
    assert.throws(() => inboxReply({ ...groupDmRoster, ...notReady }), {
      message: DM_THREAD_MEMBERS_LOADING_ERROR,
    });
  }
});

test("a group DM participant missing from the member list is not tagged as a person", () => {
  assert.deepEqual(
    inboxReply({
      ...groupDmRoster,
      members: ready([member("owner"), member("person")]),
    }),
    ["person"],
  );
});

test("an Inbox DM reply refuses a mention of someone outside the DM", () => {
  assert.throws(() => inboxReply({ explicitMentions: ["codex"] }), {
    message: DM_NON_MEMBER_MENTION_ERROR,
  });
});

test("an Inbox reply refuses when the DM or its participants are not loaded", () => {
  for (const missing of [
    { channel: undefined },
    { channel: channel({ memberPubkeys: [], participantPubkeys: ["owner"] }) },
  ]) {
    assert.throws(() => inboxReply(missing), {
      message: "Open the linked channel to reply.",
    });
  }
});

test("an Inbox reply in a stream keeps explicit-mention semantics", () => {
  assert.deepEqual(
    inboxReply({
      channel: channel({ channelType: "stream" }),
      explicitMentions: ["person"],
    }),
    ["person"],
  );
});
