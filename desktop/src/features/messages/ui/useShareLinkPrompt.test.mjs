import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const OWNER = "a".repeat(64);
const AGENT = "b".repeat(64);
const LINK = `buzz://message?channel=private-1&id=${"e".repeat(64)}`;

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      if (command === "get_event") {
        return JSON.stringify({
          content: "secret plan",
          created_at: 1_760_000_000,
          id: "e".repeat(64),
          kind: 9,
          pubkey: OWNER,
          tags: [["h", "private-1"]],
        });
      }
      throw new Error(`unmocked Tauri command: ${command}`);
    },
  };
});

const clients = [];
afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  for (const client of clients.splice(0)) client.clear();
});

after(() => dom.window.close());

async function renderPrompt(resubmit) {
  const React = await import("react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { renderHook } = await import("@testing-library/react");
  const { ChannelNavigationProvider } = await import(
    "../../../shared/context/ChannelNavigationContext.tsx"
  );
  const { channelMembersQueryKey } = await import(
    "../../channels/rosterFreshness.ts"
  );
  const { useShareLinkPrompt } = await import("./useShareLinkPrompt.ts");

  const queryClient = new QueryClient();
  clients.push(queryClient);
  queryClient.setQueryData(channelMembersQueryKey("private-1"), [
    { pubkey: OWNER },
  ]);
  const channels = [
    {
      channelType: "stream",
      id: "private-1",
      isMember: true,
      name: "planning",
      visibility: "private",
    },
  ];
  const wrapper = ({ children }) =>
    React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(ChannelNavigationProvider, { channels }, children),
    );
  return renderHook(
    () =>
      useShareLinkPrompt({
        channelId: "here",
        getComposerRevision: () => 1,
        mentions: {
          getMentionDisplayName: (pubkey) =>
            pubkey === AGENT ? "Reset Worker" : null,
          hasResolvedMembers: true,
          memberPubkeys: new Set([OWNER, AGENT]),
        },
        ownerPubkey: OWNER,
        resubmit,
      }),
    { wrapper },
  );
}

test("holds the send and names who can't open the link", async () => {
  const { act } = await import("@testing-library/react");
  const resubmits = [];
  const { result } = await renderPrompt((quote) => resubmits.push(quote));

  let held = false;
  await act(async () => {
    held = await result.current.check(`look ${LINK}`);
  });
  assert.equal(held, true);
  assert.deepEqual(result.current.names, ["Reset Worker"]);
  assert.deepEqual(resubmits, []);
});

test("Send link only resends the draft unchanged", async () => {
  const { act } = await import("@testing-library/react");
  const resubmits = [];
  const { result } = await renderPrompt((quote) => resubmits.push(quote));
  await act(async () => {
    await result.current.check(LINK);
  });
  act(() => result.current.onSendLinkOnly());
  assert.deepEqual(resubmits, [null]);
  assert.equal(result.current.names, null);
});

test("Include quote resends with the labeled quote", async () => {
  const { act } = await import("@testing-library/react");
  const resubmits = [];
  const { result } = await renderPrompt((quote) => resubmits.push(quote));
  await act(async () => {
    await result.current.check(LINK);
  });
  act(() => result.current.onIncludeQuote());
  assert.equal(resubmits.length, 1);
  assert.match(resubmits[0], /^> Quoted from \*\*.+\*\* in #planning, /);
  assert.match(resubmits[0], /\n> secret plan$/);
});

test("no prompt for a link with nothing to share", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await renderPrompt(() => {});
  let held = true;
  await act(async () => {
    held = await result.current.check("buzz://message?channel=here&id=1");
  });
  assert.equal(held, false);
  assert.equal(result.current.names, null);
});
