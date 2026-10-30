/**
 * The one-on-one agent chip, through the production hook: on by default,
 * dropped once the thread is shared, and never taken from the switch's pins.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { setKeepMentionedAgentsPinned } from "@/features/messages/lib/autoPinMentionedAgentsPreference";
import { getPersistentAgentAudienceScope } from "@/features/messages/lib/persistentAgentAudience";
import { useThreadAgentAudience } from "./useThreadAgentAudience.ts";

const me = "1".repeat(64);
const person = "2".repeat(64);
const agent = "a".repeat(64);
const isAgentPubkey = (pubkey) => pubkey === agent;
let thread = 0;
const roots = [];

afterEach(() =>
  act(() => {
    for (const root of roots.splice(0)) root.unmount();
    setKeepMentionedAgentsPinned(false);
  }),
);

async function renderThread() {
  const scope = getPersistentAgentAudienceScope({
    ownerPubkey: me,
    channelId: "channel-a",
    composerKey: `thread:${thread++}`,
  });
  const result = { pubkeys: [], shared: [] };
  function Harness({ participants, rootTags = [] }) {
    const { audience } = useThreadAgentAudience({
      currentPubkey: me,
      isAgentPubkey,
      onSharedThreadAgent: (pubkey) => result.shared.push(pubkey),
      participants,
      replyToPubkeys: [agent],
      rootTags,
      scope,
    });
    result.pubkeys = audience.pubkeys;
    return null;
  }
  const root = createRoot(document.createElement("div"));
  roots.push(root);
  result.render = (props) =>
    act(() => root.render(React.createElement(Harness, props)));
  return result;
}

test("an agent-rooted one-on-one thread gets the chip with the switch off", async () => {
  const view = await renderThread();
  await view.render({ participants: [{ pubkey: agent }, { pubkey: me }] });
  assert.deepEqual(view.pubkeys, [agent]);
});

test("the chip is dropped once another person posts", async () => {
  const view = await renderThread();
  await view.render({ participants: [{ pubkey: agent }] });
  await view.render({ participants: [{ pubkey: agent }, { pubkey: person }] });
  assert.deepEqual(view.shared, [agent]);
});

test("a shared thread keeps an agent the switch pinned from the root's tags", async () => {
  await act(() => setKeepMentionedAgentsPinned(true));
  const view = await renderThread();
  const rootTags = [["p", agent]];
  await view.render({ participants: [{ pubkey: agent }], rootTags });
  await view.render({
    participants: [{ pubkey: agent }, { pubkey: person }],
    rootTags,
  });
  assert.deepEqual(view.shared, []);
  assert.deepEqual(view.pubkeys, [agent]);
});
