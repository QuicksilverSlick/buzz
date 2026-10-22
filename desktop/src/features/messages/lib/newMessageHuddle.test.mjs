import assert from "node:assert/strict";
import test from "node:test";

import { startNewMessageHuddle } from "./newMessageHuddle.ts";

const SELF = "1".repeat(64);
const ALICE = "2".repeat(64);
const AGENT = "a".repeat(64);

const directMessage = {
  id: "dm-1",
  name: "",
  channelType: "dm",
  participantPubkeys: [SELF, ALICE, AGENT],
  participants: ["Me", "Alice Smith", "Helper Bot"],
};

test("opens the DM, shows it, then starts the huddle there", async () => {
  const calls = [];

  const started = await startNewMessageHuddle({
    currentPubkey: SELF,
    recipients: [
      { pubkey: ALICE, isAgent: false },
      { pubkey: AGENT.toUpperCase(), isAgent: true },
    ],
    openDirectMessage: async () => {
      calls.push(["open"]);
      return directMessage;
    },
    showDirectMessage: async (channel) => {
      calls.push(["show", channel.id]);
      return true;
    },
    startHuddle: async (parentChannelId, memberPubkeys, channelName) => {
      calls.push(["start", parentChannelId, memberPubkeys, channelName]);
    },
  });

  assert.equal(started, true);
  assert.deepEqual(calls, [
    ["open"],
    ["show", "dm-1"],
    ["start", "dm-1", [AGENT], "Me <> Alice <> Helper huddle"],
  ]);
});

test("starts nothing when the DM cannot be opened", async () => {
  let shown = false;
  let huddleStarted = false;

  const started = await startNewMessageHuddle({
    currentPubkey: SELF,
    recipients: [{ pubkey: ALICE, isAgent: false }],
    openDirectMessage: async () => null,
    showDirectMessage: async () => {
      shown = true;
    },
    startHuddle: async () => {
      huddleStarted = true;
    },
  });

  assert.equal(started, false);
  assert.equal(shown, false);
  assert.equal(huddleStarted, false);
});

test("does not start the huddle after the user has left the screen", async () => {
  let huddleStarted = false;

  const started = await startNewMessageHuddle({
    currentPubkey: SELF,
    recipients: [{ pubkey: ALICE, isAgent: false }],
    openDirectMessage: async () => directMessage,
    showDirectMessage: async () => false,
    startHuddle: async () => {
      huddleStarted = true;
    },
  });

  assert.equal(started, false);
  assert.equal(huddleStarted, false);
});
