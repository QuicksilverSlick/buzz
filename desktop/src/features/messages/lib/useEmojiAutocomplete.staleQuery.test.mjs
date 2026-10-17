import assert from "node:assert/strict";
import { after, test } from "node:test";
import { JSDOM } from "jsdom";

import { useEmojiAutocomplete } from "./useEmojiAutocomplete.ts";

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

// Custom matches resolve without emoji-mart, so the list opens in node.
const customEmoji = [{ shortcode: "smilecat", url: "https://cdn.test/s.png" }];

// Same defect as the #channel list: typing past ":smi" faster than the query
// debounce left the list open, so Enter replaced the typed text with an emoji.
test("an emoji list the user typed past closes before it can claim Enter", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { result } = renderHook(() => useEmojiAutocomplete(customEmoji));
  const original = "hi :smi";
  await act(async () => {
    result.current.updateEmojiQuery(original, original.length);
    await new Promise((resolve) => setTimeout(resolve, 200));
  });
  assert.equal(result.current.isEmojiAutocompleteOpen, true);

  const typed = "hi :smi there";
  act(() => result.current.updateEmojiQuery(typed, typed.length));
  let open;
  act(() => {
    open = result.current.revalidateOpen();
  });
  assert.equal(open, false);
  assert.equal(result.current.isEmojiAutocompleteOpen, false);
  cleanup();
});
