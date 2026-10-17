import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createElement } from "react";
import { JSDOM } from "jsdom";

import { ChannelNavigationProvider } from "../../../shared/context/ChannelNavigationContext.tsx";
import { useChannelLinks } from "./useChannelLinks.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
});
after(() => dom.window.close());

const channels = [
  { id: "c1", name: "general", channelType: "stream", archivedAt: null },
];

// Editing "Welcome to #general" lands the caret after "#general", which opens
// the suggestion list. Typing over it faster than the query debounce used to
// leave the list open on the old query, so the next Enter inserted
// "#general " over the typed text from the old "#" offset instead of
// submitting the edit.
test("a channel query that leaves the text closes before the next key", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const wrapper = ({ children }) =>
    createElement(ChannelNavigationProvider, { channels }, children);
  const { result } = renderHook(() => useChannelLinks(), { wrapper });

  const original = "Welcome to #general";
  await act(async () => {
    result.current.updateChannelQuery(original, original.length);
    await new Promise((resolve) => setTimeout(resolve, 200));
  });
  assert.equal(result.current.isChannelOpen, true);

  const typed = "Edited, not deleted";
  act(() => result.current.updateChannelQuery(typed, typed.length));
  assert.equal(result.current.isChannelOpen, false);
  const enter = { key: "Enter", preventDefault() {} };
  assert.deepEqual(result.current.handleChannelKeyDown(enter), {
    handled: false,
  });
  cleanup();
});
