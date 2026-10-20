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

const clients = [];

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  // A Bench card row polls and caches; nothing may outlive its test.
  for (const client of clients.splice(0)) client.clear();
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

async function renderRow(
  inboxMessage,
  { benchCards, canReply = false, channelId } = {},
) {
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
  const { TooltipProvider } = await import("@/shared/ui/tooltip.tsx");
  const queryClient = new QueryClient({
    defaultOptions: { queries: { enabled: false, retry: false } },
  });
  clients.push(queryClient);
  if (benchCards) queryClient.setQueryData(["bench-cards"], benchCards);
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
        // The action bar a replyable row shows uses tooltips.
        React.createElement(
          TooltipProvider,
          null,
          React.createElement(InboxMessageRow, {
            agentPubkeys: new Set([AGENT, PLAIN_AGENT]),
            canReply,
            channelId,
            isFocusHighlightVisible: false,
            message: inboxMessage,
            onSelectReplyTarget: () => {},
            profiles,
          }),
        ),
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

const BENCH = "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10";
const WRITER = "a".repeat(64);
const CARD_EVENT = "e".repeat(64);
const OLD_TEXT =
  "🔴 · 28-day-reset · orchestrator\n**Ship it?**\n\nWhat changes: the welcome email\nAsked by: Craig";

function benchCards() {
  return {
    enabled: true,
    channelId: BENCH,
    writerPubkey: WRITER,
    ownerPubkey: AUTHOR,
    lastError: null,
    lastTick: "2026-10-17T10:00:00Z",
    items: [
      {
        id: "orchestrator/ship",
        writer: "orchestrator",
        kind: "pending",
        title: "Ship it?",
        body: "What changes: the welcome email\nAsked by: Craig",
        area: "28-day-reset",
        severity: "critical",
        options: ["Ship", "Hold"],
        state: null,
        decidedBy: null,
        deadline: null,
        links: [],
        order: 0,
        validity: "current",
        cardEventId: CARD_EVENT,
        answer: null,
        recorded: null,
        updatedAt: "2026-10-17T10:00:00Z",
        channelId: null,
        project: null,
        scoped: false,
        needs: "you",
        answerers: [],
        ownerName: null,
        recommend: null,
        recommender: null,
        reply: null,
        request: null,
      },
    ],
  };
}

test("a live Bench card's post shows as the card, not the text it was posted with", async () => {
  const card = message({
    authorPubkey: WRITER,
    content: OLD_TEXT,
    id: CARD_EVENT,
    kind: 9,
    signerPubkey: WRITER,
    tags: [["h", BENCH]],
  });
  const view = await renderRow(card, {
    benchCards: benchCards(),
    channelId: BENCH,
  });
  const shown = view.getByTestId("bench-card");
  assert.deepEqual(
    [...shown.querySelectorAll("p > span.font-semibold")].map(
      (span) => span.textContent,
    ),
    ["What changes:", "Asked by:"],
  );
  assert.ok(!view.container.textContent.includes("28-day-reset"));
  view.unmount();

  // Anyone else's message with the card's id, or the card's text, is chat.
  const forged = await renderRow(
    { ...card, signerPubkey: AUTHOR, authorPubkey: AUTHOR },
    { benchCards: benchCards(), channelId: BENCH },
  );
  assert.equal(forged.queryByTestId("bench-card"), null);
  assert.ok(forged.container.textContent.includes("28-day-reset"));
});

test("a retired card's post names the live card read-only, without its keycap pills", async () => {
  const keycap = (emoji) => ({ emoji, count: 1, users: [] });
  const retired = message({
    authorPubkey: WRITER,
    content: OLD_TEXT,
    id: "9".repeat(64),
    kind: 9,
    reactions: [keycap("1️⃣"), keycap("2️⃣"), keycap("👍")],
    signerPubkey: WRITER,
    tags: [
      ["h", BENCH],
      ["client", "bench:card:orchestrator/ship@0a1b2c"],
    ],
  });
  const view = await renderRow(retired, {
    benchCards: benchCards(),
    canReply: true,
    channelId: BENCH,
  });
  const shown = view.getByTestId("bench-card-retired");
  assert.equal(shown.getAttribute("data-bench-item"), "orchestrator/ship");
  assert.ok(shown.textContent.startsWith("Replaced by a newer card"));
  assert.ok(!view.container.textContent.includes("28-day-reset"));
  // Kept history: no second live card, so nothing to answer or reply to.
  assert.equal(view.queryByTestId("bench-card"), null);
  assert.equal(
    view.container.querySelector('fieldset[aria-label="Answer"]'),
    null,
  );
  assert.equal(view.queryByText("Other..."), null);
  assert.equal(view.queryByText("Open thread"), null);
  const pills = view.getByTestId("message-reactions").textContent;
  assert.ok(pills.includes("👍"));
  assert.ok(!pills.includes("1️⃣") && !pills.includes("2️⃣"), pills);
  view.unmount();

  const live = await renderRow(
    { ...retired, id: CARD_EVENT },
    { benchCards: benchCards(), canReply: true, channelId: BENCH },
  );
  assert.ok(live.container.querySelector('fieldset[aria-label="Answer"]'));
  assert.ok(live.getByText("Open thread"));
});
