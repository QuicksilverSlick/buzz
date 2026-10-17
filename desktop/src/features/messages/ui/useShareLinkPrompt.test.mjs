import assert from "node:assert/strict";
import { after, afterEach, before, mock, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const OWNER = "a".repeat(64);
const AGENT = "b".repeat(64);
const MESSAGE_ID = "e".repeat(64);
const LINK = `buzz://message?channel=private-1&id=${MESSAGE_ID}`;

const ORIGINAL = {
  content: "secret plan",
  created_at: 1_760_000_000,
  id: MESSAGE_ID,
  kind: 9,
  pubkey: OWNER,
  tags: [["h", "private-1"]],
};

/** What the relay holds; tests change it between lookups. */
let relay;
function resetRelay() {
  relay = { aux: [], gate: null, lookups: 0 };
}
resetRelay();

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
        relay.lookups += 1;
        await relay.gate;
        return JSON.stringify(ORIGINAL);
      }
      if (command === "get_relay_self") return null;
      throw new Error(`unmocked Tauri command: ${command}`);
    },
  };
});

const clients = [];
afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  for (const client of clients.splice(0)) client.clear();
  mock.restoreAll();
  resetRelay();
});

after(() => dom.window.close());

async function renderPrompt(resubmit, overrides = {}) {
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
  const { relayClient } = await import("../../../shared/api/relayClient.ts");
  const { useShareLinkPrompt } = await import("./useShareLinkPrompt.ts");

  mock.method(relayClient, "fetchAuxEventsByReference", async () => relay.aux);
  mock.method(
    relayClient,
    "fetchAuxDeletionEventsForAuxEvents",
    async () => [],
  );
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
    ({ channelId, draftKey }) =>
      useShareLinkPrompt({
        channelId,
        draftKey,
        getComposerRevision: () => 1,
        mentions: {
          extractMentionPersonas: () => [],
          extractMentionPubkeys: () => [],
          getMentionDisplayName: (pubkey) =>
            pubkey === AGENT ? "Reset Worker" : null,
          hasResolvedMembers: true,
          memberPubkeys: new Set([OWNER, AGENT]),
          ...overrides.mentions,
        },
        ownerPubkey: OWNER,
        resubmit,
      }),
    { initialProps: { channelId: "here", draftKey: "here" }, wrapper },
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
  assert.deepEqual(result.current.prompt?.names, ["Reset Worker"]);
  assert.deepEqual(resubmits, []);
});

test("an agent the send mentions or always addresses counts, not just members", async () => {
  const { act } = await import("@testing-library/react");
  const members = { memberPubkeys: new Set([OWNER]) };
  for (const [overrides, addressed] of [
    [{ ...members, extractMentionPubkeys: () => [AGENT.toUpperCase()] }, []],
    [members, [AGENT]],
  ]) {
    const { result } = await renderPrompt(() => {}, { mentions: overrides });
    await act(async () => {
      await result.current.check(`@Reset Worker summarize ${LINK}`, addressed);
    });
    assert.deepEqual(result.current.prompt?.names, ["Reset Worker"]);
  }
  const alone = await renderPrompt(() => {}, { mentions: members });
  await act(async () => {
    assert.equal(await alone.result.current.check(LINK), false);
  });
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
  assert.equal(result.current.prompt, null);
});

test("Include quote resends with the labeled quote it showed", async () => {
  const { act } = await import("@testing-library/react");
  const resubmits = [];
  const { result } = await renderPrompt((quote) => resubmits.push(quote));
  await act(async () => {
    await result.current.check(LINK);
  });
  const shown = result.current.prompt?.quote;
  act(() => result.current.onIncludeQuote());
  assert.equal(resubmits.length, 1);
  assert.equal(resubmits[0], shown);
  assert.match(resubmits[0], /^> Quoted from \*\*.+\*\* in #planning, /);
  assert.match(resubmits[0], /\n> secret plan$/);
});

test("the quote is the message as edited now, and a deleted one is never quoted", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await renderPrompt(() => {});
  await act(async () => {
    await result.current.check(LINK);
  });
  assert.match(result.current.prompt?.quote ?? "", /secret plan/);

  // Edited after the first lookup: a fresh fetch, never the session cache.
  relay.aux = [
    {
      content: "(removed)",
      created_at: ORIGINAL.created_at + 60,
      id: "1".repeat(64),
      kind: 40003,
      pubkey: OWNER,
      tags: [
        ["e", MESSAGE_ID],
        ["h", "private-1"],
      ],
    },
  ];
  await act(async () => {
    await result.current.check(LINK);
  });
  assert.equal(relay.lookups, 2);
  assert.match(result.current.prompt?.quote ?? "", /\n> \(removed\)$/);
  assert.doesNotMatch(result.current.prompt?.quote ?? "", /secret/);

  // Someone else's "edit" doesn't count.
  relay.aux = [{ ...relay.aux[0], pubkey: AGENT }];
  await act(async () => {
    await result.current.check(LINK);
  });
  assert.match(result.current.prompt?.quote ?? "", /secret plan/);

  relay.aux = [
    {
      content: "",
      created_at: ORIGINAL.created_at + 90,
      id: "2".repeat(64),
      kind: 5,
      pubkey: OWNER,
      tags: [["e", MESSAGE_ID]],
    },
  ];
  let held = true;
  await act(async () => {
    held = await result.current.check(LINK);
  });
  assert.equal(held, false);
  assert.equal(result.current.prompt, null);
});

test("switching channel while the lookup runs offers nothing and sends nothing", async () => {
  const { act } = await import("@testing-library/react");
  const resubmits = [];
  const { rerender, result } = await renderPrompt((quote) =>
    resubmits.push(quote),
  );
  let release;
  relay.gate = new Promise((resolve) => {
    release = resolve;
  });
  let pending;
  act(() => {
    pending = result.current.check(LINK);
  });
  rerender({ channelId: "other", draftKey: "other" });
  let held = false;
  await act(async () => {
    release();
    held = await pending;
  });
  assert.equal(held, true, "the old channel's send stays unsent");
  assert.equal(result.current.prompt, null);
  act(() => result.current.onIncludeQuote());
  assert.deepEqual(resubmits, []);
});

test("a prompt from another draft can't be answered here", async () => {
  const { act } = await import("@testing-library/react");
  const resubmits = [];
  const { rerender, result } = await renderPrompt((quote) =>
    resubmits.push(quote),
  );
  await act(async () => {
    await result.current.check(LINK);
  });
  const { onIncludeQuote } = result.current;
  rerender({ channelId: "here", draftKey: "here:thread" });
  act(() => onIncludeQuote());
  assert.deepEqual(resubmits, []);
});

test("an ambiguous mention skips the prompt instead of failing the send", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await renderPrompt(() => {}, {
    mentions: {
      extractMentionPubkeys: () => {
        throw new Error("The mention @Claude is ambiguous.");
      },
    },
  });
  for (const draft of ["@Claude hi", `@Claude see ${LINK}`]) {
    let held = true;
    await act(async () => {
      held = await result.current.check(draft);
    });
    assert.equal(held, false, "the send goes on to the mention flow's toast");
    assert.equal(result.current.prompt, null);
  }
});

test("no prompt for a link with nothing to share", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await renderPrompt(() => {});
  let held = true;
  await act(async () => {
    held = await result.current.check("buzz://message?channel=here&id=1");
  });
  assert.equal(held, false);
  assert.equal(result.current.prompt, null);
});
