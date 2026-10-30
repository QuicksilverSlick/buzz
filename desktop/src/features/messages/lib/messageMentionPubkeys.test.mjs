import assert from "node:assert/strict";
import test from "node:test";

import {
  directMessageRecipientPubkeys,
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

const isAgent = (pubkey) => pubkey.startsWith("agent");

test("an Inbox reply in a one-on-one agent DM tags the agent with no @", () => {
  assert.deepEqual(
    directMessageRecipientPubkeys(channel(), "owner", [], isAgent),
    ["agent"],
  );
});

test("an Inbox reply in a group DM tags the people but no untyped agent", () => {
  const agentsOnlyDm = channel({
    memberPubkeys: ["owner", "agent-a", "agent-b"],
    participantPubkeys: ["owner", "agent-a", "agent-b"],
  });
  assert.deepEqual(
    directMessageRecipientPubkeys(agentsOnlyDm, "owner", [], isAgent),
    [],
  );
  const groupDm = channel({
    memberPubkeys: ["owner", "agent-a", "agent-b", "person"],
    participantPubkeys: ["owner", "agent-a", "agent-b", "person"],
  });
  assert.deepEqual(
    directMessageRecipientPubkeys(groupDm, "owner", [], isAgent),
    ["person"],
  );
  assert.deepEqual(
    directMessageRecipientPubkeys(groupDm, "owner", ["AGENT-B"], isAgent),
    ["agent-b", "person"],
  );
});

test("an Inbox reply refuses when the DM's participants are not loaded", () => {
  assert.equal(
    directMessageRecipientPubkeys(
      channel({ memberPubkeys: [], participantPubkeys: ["owner"] }),
      "owner",
      [],
      isAgent,
    ),
    null,
  );
});

test("an Inbox reply in a stream keeps explicit-mention semantics", () => {
  assert.deepEqual(
    directMessageRecipientPubkeys(
      channel({ channelType: "stream" }),
      "owner",
      ["person"],
      isAgent,
    ),
    ["person"],
  );
});
