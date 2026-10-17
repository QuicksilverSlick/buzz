/**
 * The Inbox's DM detail never shows the relay's own system JSON (a DM's
 * dm_created), and never hides a person's message, whatever it says.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { renderHook } from "@testing-library/react";

import { useHomeInboxContextMessages } from "./useHomeInboxContextMessages.ts";

const RELAY = "a".repeat(64);
const ANA = "b".repeat(64);
const BO = "c".repeat(64);
const DM_CREATED = JSON.stringify({
  actor: ANA,
  participants: [ANA, BO],
  type: "dm_created",
});

function event(id, pubkey, kind, content, createdAt) {
  return {
    id: id.repeat(64),
    pubkey,
    kind,
    content,
    created_at: createdAt,
    tags: [["h", "dm-1"]],
    sig: "",
  };
}

test("the relay's dm_created is not a message; a person's JSON still is", () => {
  const events = [
    event("1", RELAY.toUpperCase(), 40099, DM_CREATED, 1),
    event("2", ANA, 9, DM_CREATED, 2),
    event("3", ANA, 9, "hi Bo", 3),
  ];
  const selectedItem = { id: events[2].id, item: { pubkey: ANA } };
  const contents = (relaySelfPubkey) =>
    renderHook(() =>
      useHomeInboxContextMessages({
        events,
        reactionEvents: [],
        relaySelfPubkey,
        selectedChannel: null,
        selectedEventId: null,
        selectedItem,
      }),
    ).result.current.map((message) => [message.id[0], message.content]);

  assert.deepEqual(contents(RELAY), [
    ["2", DM_CREATED],
    ["3", "hi Bo"],
  ]);
  // A relay key it can't check hides nothing.
  assert.deepEqual(
    contents(null).map(([id]) => id),
    ["1", "2", "3"],
  );
  // Another key's 40099 is that key's message.
  assert.deepEqual(
    contents(BO).map(([id]) => id),
    ["1", "2", "3"],
  );
});
