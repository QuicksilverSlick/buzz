/**
 * The one-on-one agent chip, through the production hooks wired the way
 * MessageComposer wires them: on by default, dropped once the thread is
 * shared, and never taken from the user's or the switch's pins.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { setKeepMentionedAgentsPinned } from "@/features/messages/lib/autoPinMentionedAgentsPreference";
import { getPersistentAgentAudienceScope } from "@/features/messages/lib/persistentAgentAudience";
import { useAgentAddressLockPicker } from "./useAgentAddressLockPicker.ts";
import { useAutoPinMentionedAgents } from "./useAutoPinMentionedAgents.ts";
import { useThreadAgentAudience } from "./useThreadAgentAudience.ts";

const me = "1".repeat(64);
const person = "2".repeat(64);
const agent = "a".repeat(64);
const ada = { pubkey: agent, displayName: "Agent Ada", isAgent: true };
const label = `@${ada.displayName}`;
const isAgentPubkey = (pubkey) => pubkey === agent;
const oneOnOne = [{ pubkey: agent }, { pubkey: me }];
const shared = [...oneOnOne, { pubkey: person }];
const noop = () => {};
// The draft's p tags come from its mentions; this resolves "@Agent Ada".
const mentions = {
  getDraftMentionRefs: (text) => (text.includes(label) ? [ada] : []),
  getMentionDisplayName: () => ada.displayName,
  insertMention: (_suggestion, cursor) => ({
    replaceFromOffset: cursor,
    replaceToOffset: cursor,
    insertText: `${label} `,
  }),
  isInlineMentionSelection: () => true,
  isMentionOpen: false,
  registerMentionPubkey: () => ada.displayName,
};
let thread = 0;
const roots = [];

afterEach(() =>
  act(() => {
    for (const root of roots.splice(0)) root.unmount();
    setKeepMentionedAgentsPinned(false);
  }),
);

function renderComposer() {
  const scope = getPersistentAgentAudienceScope({
    ownerPubkey: me,
    channelId: "channel-a",
    composerKey: `thread:${thread++}`,
  });
  const view = {
    authored: [],
    picker: null,
    pubkeys: [],
    revoked: [],
    text: "",
  };
  const applyAutocompleteEdit = (edit) => {
    view.text =
      view.text.slice(0, edit.replaceFromOffset) +
      edit.insertText +
      view.text.slice(edit.replaceToOffset);
  };
  const richText = {
    focusEnd: noop,
    getPlainTextAndCursor: () => ({
      text: view.text,
      cursor: view.text.length,
    }),
  };
  function Composer({ participants, rootTags }) {
    const revokeRef = React.useRef(noop);
    const { audience, keepMentionedAgentsPinned } = useThreadAgentAudience({
      currentPubkey: me,
      isAgentPubkey,
      onSharedThreadAgent: (pubkey) => {
        view.revoked.push(pubkey);
        revokeRef.current(pubkey);
      },
      participants,
      rootTags,
      scope,
    });
    const autoPin = useAutoPinMentionedAgents({
      audienceScope: scope,
      enabled: keepMentionedAgentsPinned,
      getDisplayName: mentions.getMentionDisplayName,
      onPulse: noop,
      onTurnOff: () => setKeepMentionedAgentsPinned(false),
      onTurnOn: () => setKeepMentionedAgentsPinned(true),
    });
    const picker = useAgentAddressLockPicker({
      applyAutocompleteEdit,
      audience,
      audienceScope: scope,
      mentions,
      onAddressAgentMention: (suggestion) =>
        autoPin.promoteExplicitlyAddressedAgents({
          pubkeys: [suggestion.pubkey],
        }),
      onAutoPinAgentMention: (suggestion, options) =>
        autoPin.promoteMentionedAgents({
          ...options,
          pubkeys: [suggestion.pubkey],
        }),
      // Drops the "app inserted this prefix" mark; a saved draft keeps it.
      onImplicitPrefixRemoved: (pubkey) => view.authored.push(pubkey),
      onPulseAddressLock: noop,
      richText,
    });
    revokeRef.current = picker.revokeAddressedAgent;
    React.useLayoutEffect(() => {
      picker.restoreAddressedAgentMentions();
    }, [picker.restoreAddressedAgentMentions]);
    view.picker = picker;
    view.pubkeys = audience.pubkeys;
    return null;
  }
  const root = createRoot(document.createElement("div"));
  roots.push(root);
  view.render = (participants, rootTags = []) =>
    act(() =>
      root.render(React.createElement(Composer, { participants, rootTags })),
    );
  view.act = (callback) => act(callback);
  return view;
}

test("an agent-rooted one-on-one thread gets the chip with the switch off", async () => {
  const view = renderComposer();
  await view.render(oneOnOne);
  assert.deepEqual(view.pubkeys, [agent]);
  assert.equal(view.text, `${label} `);
});

test("removing the chip sends with no tag, and it stays removed", async () => {
  const view = renderComposer();
  await view.render(oneOnOne);
  await view.act(() => view.picker.removeAddressedAgent(agent));
  await view.render(oneOnOne);
  assert.deepEqual(view.pubkeys, []);
  assert.equal(view.text, "");
  assert.deepEqual(mentions.getDraftMentionRefs(view.text), []);
});

test("an untouched chip is dropped once another person posts", async () => {
  const view = renderComposer();
  await view.render(oneOnOne);
  await view.render(shared);
  assert.deepEqual(view.revoked, [agent]);
  assert.deepEqual(view.pubkeys, []);
  assert.equal(view.text, "");
});

test("a reply already being written keeps its @mention when the thread turns shared", async () => {
  const view = renderComposer();
  await view.render(oneOnOne);
  view.text += "here is the log, can you check line 40";
  await view.render(shared);
  assert.deepEqual(view.pubkeys, []);
  assert.equal(view.text, `${label} here is the log, can you check line 40`);
  assert.deepEqual(view.authored, [agent]);
});

test("a chip removed and then re-added by hand survives the thread turning shared", async () => {
  const view = renderComposer();
  await view.render(oneOnOne);
  await view.act(() => view.picker.removeAddressedAgent(agent));
  await view.act(() => view.picker.toggleAlwaysAddressAgent(ada));
  assert.deepEqual(view.pubkeys, [agent]);
  await view.render(shared);
  assert.deepEqual(view.revoked, []);
  assert.deepEqual(view.pubkeys, [agent]);
});

test("turning the switch off does not bring back a removed chip", async () => {
  const view = renderComposer();
  await view.render(oneOnOne);
  await view.act(() => view.picker.removeAddressedAgent(agent));
  await view.act(() => setKeepMentionedAgentsPinned(false));
  assert.deepEqual(view.pubkeys, []);
  assert.equal(view.text, "");
});

test("with the switch on, an inline @mention re-pins the agent after a shared drop", async () => {
  await act(() => setKeepMentionedAgentsPinned(true));
  const view = renderComposer();
  await view.render(oneOnOne);
  await view.render(shared);
  assert.deepEqual(view.pubkeys, []);
  await view.act(() => view.picker.selectMentionSuggestion(ada));
  assert.deepEqual(view.pubkeys, [agent]);
});

test("a shared thread keeps an agent the switch pinned from the root's tags", async () => {
  const view = renderComposer();
  const rootTags = [["p", agent]];
  await view.render(oneOnOne, rootTags);
  await act(() => setKeepMentionedAgentsPinned(true));
  await view.render(shared, rootTags);
  assert.deepEqual(view.revoked, []);
  assert.deepEqual(view.pubkeys, [agent]);
});
