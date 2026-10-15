import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const AGENT = "c".repeat(64);
const AUTHOR = "d".repeat(64);
const PLAIN_AGENT = "f".repeat(64);

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    self: dom.window,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const profiles = {
  [AGENT]: {
    displayName: "Agent Ada",
    avatarUrl: "https://example.test/ada.png",
    nip05Handle: null,
    ownerPubkey: null,
    isAgent: true,
  },
  [PLAIN_AGENT]: {
    displayName: "Morgarita",
    avatarUrl: null,
    nip05Handle: null,
    ownerPubkey: null,
    isAgent: true,
  },
};

function message(overrides) {
  return {
    authorLabel: "Author",
    authorPubkey: AUTHOR,
    avatarUrl: null,
    content: "hello",
    createdAt: 1_700_000_000,
    depth: 0,
    fullTimestampLabel: "Nov 14, 2023, 10:13 PM",
    id: "e".repeat(64),
    isSelected: false,
    mentionNames: [],
    tags: [["h", "channel-1"]],
    ...overrides,
  };
}

async function renderRow(inboxMessage) {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const {
    RouterContextProvider,
    createMemoryHistory,
    createRootRoute,
    createRouter,
  } = await import("@tanstack/react-router");
  const { InboxMessageRow } = await import("./InboxMessageRow.tsx");
  const queryClient = new QueryClient({
    defaultOptions: { queries: { enabled: false, retry: false } },
  });
  // Only the router context the Markdown renderer reads; no route resolves.
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  return render(
    React.createElement(
      RouterContextProvider,
      { router },
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(InboxMessageRow, {
          agentPubkeys: new Set([AGENT, PLAIN_AGENT]),
          canReply: false,
          isFocusHighlightVisible: false,
          message: inboxMessage,
          onSelectReplyTarget: () => {},
          profiles,
        }),
      ),
    ),
  );
}

test("an agent mention in an Inbox message renders as an agent chip", async () => {
  const view = await renderRow(
    message({
      content: "@Agent Ada please check",
      mentionNames: ["Agent Ada"],
      mentionPubkeysByName: { "agent ada": AGENT },
      tags: [
        ["h", "channel-1"],
        ["p", AGENT],
      ],
    }),
  );

  const chip = view.container.querySelector("[data-mention-pubkey]");
  assert.equal(chip?.getAttribute("data-mention-kind"), "agent");
});

test("an addressed agent leads the Inbox message with its own avatar", async () => {
  const view = await renderRow(
    message({
      content: "please check",
      tags: [
        ["h", "channel-1"],
        ["p", AGENT],
        ["mention", AGENT, "agent-address"],
      ],
    }),
  );

  const avatar = view.getByTestId("message-agent-address-avatar");
  const chip = avatar.closest("[data-mention]");
  // The avatar replaces the generic robot glyph.
  assert.doesNotMatch(chip?.className ?? "", /inline-chip-icon-agent/);
  // jsdom never loads the image, so the fallback shows after its delay. It
  // must add no initials: chip text is what copy reads back as the label.
  await view.findByTestId("message-agent-address-avatar-fallback");
  assert.equal(chip?.textContent, "Agent Ada");
});

test("an addressed agent with no image keeps the agent glyph and its text", async () => {
  const { act } = await import("@testing-library/react");
  const view = await renderRow(
    message({
      content: "please check",
      tags: [
        ["h", "channel-1"],
        ["p", PLAIN_AGENT],
        ["mention", PLAIN_AGENT, "agent-address"],
      ],
    }),
  );

  // Past the avatar fallback delay (200 ms), when initials would appear.
  await act(() => new Promise((resolve) => setTimeout(resolve, 400)));
  const chip = view.container.querySelector("[data-mention]");
  assert.equal(chip?.textContent, "Morgarita");
  assert.match(chip?.className ?? "", /inline-chip-icon-agent/);
});

test("a workflow post shows the workflow icon in place of an avatar", async () => {
  const view = await renderRow(
    message({ authorLabel: "Workflow", isWorkflowPost: true }),
  );

  assert.ok(view.getByTestId("workflow-avatar"));
  assert.equal(view.getByTestId("message-author").textContent, "Workflow");
});
