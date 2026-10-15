/**
 * Production-seam pins for the Bench screen and its sidebar entry: the real
 * `BenchScreen` and `BenchSidebarEntry` mounted against a mocked Tauri IPC
 * boundary.
 *
 * - The screen shows the board's sections in its order, plus Held next to
 *   Decided, each with its own empty copy, the last update time and a
 *   visible lastError line.
 * - Search narrows every section; Open thread opens the card's thread.
 * - Answering from the screen sends `bench_answer` and the card moves to
 *   "Answered, waiting for action" once the snapshot records it.
 * - Only the owner's identity gets live option buttons.
 * - The sidebar entry exists only while the Bench is enabled; its badge is
 *   the "Needs you" count.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const clients = [];
const ipcCalls = [];
const ipcHandlers = new Map();

let act;
let cleanup;
let fireEvent;
let render;
let screen;
let waitFor;
let within;
let createElement;
let QueryClient;
let QueryClientProvider;
let TooltipProvider;
let SidebarProvider;
let SidebarMenu;
let BenchScreen;
let BenchSidebarEntry;
let RouterContextProvider;
let createMemoryHistory;
let createRootRoute;
let createRouter;
let router;

const OWNER = "b".repeat(64);
const STRANGER = "c".repeat(64);

/** A distinct card event id per item id, so rows keyed by it never clash. */
function eventIdOf(id) {
  return [...id]
    .map((c) => c.charCodeAt(0).toString(16))
    .join("")
    .padEnd(64, "e")
    .slice(0, 64);
}

function card(id, overrides = {}) {
  return {
    id,
    writer: "dreamforge",
    kind: "pending",
    title: `Title of ${id}`,
    body: "",
    area: "release",
    severity: "attention",
    options: ["Ship it", "Hold"],
    state: null,
    decidedBy: null,
    deadline: null,
    links: [],
    order: 0,
    validity: "current",
    cardEventId: eventIdOf(id),
    answer: null,
    recorded: null,
    updatedAt: "2026-10-12T10:00:00Z",
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
    ...overrides,
  };
}

function snapshot(items, overrides = {}) {
  return {
    enabled: true,
    channelId: "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10",
    writerPubkey: "a".repeat(64),
    ownerPubkey: OWNER,
    lastError: null,
    lastTick: new Date().toISOString(),
    items,
    ...overrides,
  };
}

function serveSnapshot(value) {
  ipcHandlers.set("bench_cards", () => Promise.resolve(value));
}

function serveIdentity(pubkey) {
  ipcHandlers.set("get_identity", () =>
    Promise.resolve({ pubkey, display_name: "me", lost: false, locked: false }),
  );
}

function wrap(element) {
  const client = new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0 },
      queries: { gcTime: 0, retry: false },
    },
  });
  clients.push(client);
  // Only the router context useAppNavigation reads; no route resolves.
  router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/bench"] }),
  });
  return createElement(
    RouterContextProvider,
    { router },
    createElement(
      QueryClientProvider,
      { client },
      createElement(TooltipProvider, null, element),
    ),
  );
}

function cardIds(container) {
  return [...container.querySelectorAll("[data-testid='bench-card']")].map(
    (node) => node.getAttribute("data-bench-item"),
  );
}

function answerCalls() {
  return ipcCalls.filter((c) => c.cmd === "bench_answer").map((c) => c.args);
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    self: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (key === "window" || key === "document" || key === "globalThis")
      continue;
    const value = dom.window[key];
    if (
      typeof value === "function" &&
      /^(HTML|SVG)|Element$|Event$|EventTarget$|^Node|^Document|Observer$/.test(
        key,
      )
    ) {
      globalThis[key] = value;
    }
  }
  globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      ipcCalls.push({ cmd, args });
      const handler = ipcHandlers.get(cmd);
      if (handler) return handler(args);
      return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
    },
    transformCallback: () => Math.random(),
  };

  ({ act, cleanup, fireEvent, render, screen, waitFor, within } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip"));
  ({ SidebarMenu, SidebarProvider } = await import("@/shared/ui/sidebar"));
  ({
    RouterContextProvider,
    createMemoryHistory,
    createRootRoute,
    createRouter,
  } = await import("@tanstack/react-router"));
  ({ BenchScreen } = await import("./BenchScreen.tsx"));
  ({ BenchSidebarEntry } = await import("./BenchSidebarEntry.tsx"));
});

afterEach(() => {
  cleanup?.();
  for (const client of clients.splice(0)) {
    client.cancelQueries();
    client.clear();
  }
  ipcHandlers.clear();
  ipcCalls.length = 0;
});

after(() => dom.window.close());

test("screen: board-order sections, counts, empty copy and the error line", async () => {
  serveIdentity(OWNER);
  serveSnapshot(
    snapshot(
      [
        card("w/later", { severity: "info" }),
        card("w/first", { severity: "critical" }),
        card("w/answered", {
          answer: { option: 1, answeredAt: 1 },
          recorded: 1,
        }),
        card("w/status", { kind: "status", options: [], state: "blocked" }),
        card("w/stale", { validity: "superseded" }),
      ],
      { lastError: "relay timed out" },
    ),
  );
  let view;
  await act(async () => {
    view = render(wrap(createElement(BenchScreen)));
  });
  await waitFor(() => screen.getByTestId("bench-section-needsYou"));

  assert.deepEqual(
    [...view.container.querySelectorAll("section")].map((s) =>
      s.getAttribute("aria-label"),
    ),
    [
      "Needs you",
      "Waiting on others",
      "Answered, waiting for action",
      "Decided",
      "Held",
      "Where things stand",
    ],
  );
  const needsYou = screen.getByTestId("bench-section-needsYou");
  assert.deepEqual(cardIds(needsYou), ["w/first", "w/later"]);
  assert.ok(within(needsYou).getByRole("heading", { name: "Needs you 2" }));
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-answered")), [
    "w/answered",
  ]);
  // Blocked is on hold: next to Decided, not under Where things stand.
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-held")), [
    "w/status",
  ]);
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-status")), []);
  const decided = screen.getByTestId("bench-section-decided");
  assert.deepEqual(cardIds(decided), []);
  assert.ok(within(decided).getByText("No decisions yet."));
  assert.equal(
    screen.getByTestId("bench-last-error").textContent,
    "The Bench's last update failed: relay timed out",
  );
  assert.match(
    screen.getByTestId("bench-last-update").textContent,
    /^Last update: \d{1,2}:\d{2}\s[AP]M$/,
  );
  assert.ok(
    screen.getByText(
      "1 stale or superseded item is being retired and isn't shown.",
    ),
  );
});

test("screen: an answer goes through bench_answer and the card moves to Answered", async () => {
  serveIdentity(OWNER);
  const open = card("w/ship");
  serveSnapshot(snapshot([open]));
  ipcHandlers.set("bench_answer", ({ itemId, option }) => {
    // The Bench's tick records the keycap; the next snapshot shows it.
    serveSnapshot(
      snapshot([
        { ...open, recorded: option, answer: { option, answeredAt: 1 } },
      ]),
    );
    assert.equal(itemId, "w/ship");
    return Promise.resolve(null);
  });
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  const needsYou = await waitFor(() =>
    screen.getByTestId("bench-section-needsYou"),
  );
  await waitFor(() => assert.deepEqual(cardIds(needsYou), ["w/ship"]));

  await act(async () => {
    fireEvent.click(
      within(needsYou).getByRole("button", { name: /^2\. Hold/ }),
    );
  });
  assert.deepEqual(answerCalls(), [
    { itemId: "w/ship", cardEventId: eventIdOf("w/ship"), option: 1 },
  ]);

  const answered = screen.getByTestId("bench-section-answered");
  await waitFor(() => assert.deepEqual(cardIds(answered), ["w/ship"]));
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-needsYou")), []);
  assert.ok(screen.getByText("Nothing needs you right now."));
  assert.match(
    within(answered).getByTestId("bench-card-recorded").textContent,
    /^Recorded: 2\. Hold/,
  );
});

test("screen: a replaced card is a new card; nothing from the one seen carries over", async () => {
  serveIdentity(OWNER);
  const seen = card("w/ship");
  serveSnapshot(snapshot([seen]));
  ipcHandlers.set("bench_answer", () => {
    // Re-dropped between the look and the click: the Bench refuses the card
    // the owner saw, and the next snapshot has its replacement.
    serveSnapshot(
      snapshot([
        {
          ...seen,
          cardEventId: "f".repeat(64),
          options: ["Ship it now", "Hold"],
        },
      ]),
    );
    return Promise.reject(new Error("This card changed - look again"));
  });
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  const needsYou = await waitFor(() =>
    screen.getByTestId("bench-section-needsYou"),
  );
  await waitFor(() => assert.deepEqual(cardIds(needsYou), ["w/ship"]));

  await act(async () => {
    fireEvent.click(
      within(needsYou).getByRole("button", { name: /^1\. Ship it$/ }),
    );
  });
  assert.deepEqual(answerCalls(), [
    { itemId: "w/ship", cardEventId: eventIdOf("w/ship"), option: 0 },
  ]);
  await waitFor(() =>
    within(needsYou).getByRole("button", { name: /^1\. Ship it now/ }),
  );
  // Text, not the node: a jsdom node in a failure message never prints.
  assert.equal(screen.queryByText(/This card changed/)?.textContent, undefined);
  assert.equal(screen.queryByText("Saving...")?.textContent, undefined);
});

test("screen: only the owner's identity gets live buttons", async () => {
  serveIdentity(STRANGER);
  serveSnapshot(snapshot([card("w/ship")]));
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  await waitFor(() =>
    assert.ok(screen.getByText("Only the Bench owner can answer this card.")),
  );
  const button = screen.getByRole("button", { name: /^1\. Ship it/ });
  assert.equal(button.disabled, true);
  await act(async () => {
    fireEvent.click(button);
  });
  assert.equal(answerCalls().length, 0);
});

test("screen: disabled, unreadable and not-yet-ticked Benches say so", async () => {
  serveIdentity(OWNER);
  serveSnapshot({ enabled: false });
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  await waitFor(() =>
    assert.equal(
      screen.getByTestId("bench-disabled").textContent,
      "The Bench isn't enabled on this computer.",
    ),
  );
  assert.equal(screen.queryByTestId("bench-section-needsYou"), null);
  cleanup();

  ipcHandlers.set("bench_cards", () => Promise.reject("no home directory"));
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  await waitFor(() =>
    assert.ok(screen.getByText("Couldn't read the Bench: no home directory")),
  );
  cleanup();

  serveSnapshot(snapshot([], { channelId: "", lastTick: "" }));
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  await waitFor(() =>
    assert.equal(
      screen.getByTestId("bench-last-update").textContent,
      "Waiting for the Bench's first update.",
    ),
  );
  for (const copy of [
    "Nothing needs you right now.",
    "No card is waiting on anyone else.",
    "Nothing you answered is waiting on an agent.",
    "No decisions yet.",
    "Nothing is on hold.",
    "No status updates yet.",
  ]) {
    assert.ok(screen.getByText(copy), copy);
  }
});

function sidebarEntry(onSelect) {
  return wrap(
    createElement(
      SidebarProvider,
      null,
      createElement(
        SidebarMenu,
        null,
        createElement(BenchSidebarEntry, { isActive: false, onSelect }),
      ),
    ),
  );
}

test("sidebar: hidden while disabled; the badge counts Needs you", async () => {
  serveSnapshot({ enabled: false });
  await act(async () => {
    render(sidebarEntry(() => {}));
  });
  await waitFor(() => assert.ok(ipcCalls.some((c) => c.cmd === "bench_cards")));
  // Let the disabled snapshot land before asserting nothing rendered.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  assert.equal(screen.queryByTestId("open-bench-view"), null);
  cleanup();

  serveSnapshot(
    snapshot([
      card("w/a"),
      card("w/b", { severity: "critical" }),
      card("w/answered", { answer: { option: 0, answeredAt: 1 } }),
      card("w/stale", { validity: "suspected-stale" }),
    ]),
  );
  let selected = 0;
  await act(async () => {
    render(
      sidebarEntry(() => {
        selected += 1;
      }),
    );
  });
  const button = await waitFor(() => screen.getByTestId("open-bench-view"));
  assert.equal(screen.getByTestId("sidebar-bench-count").textContent, "2");
  await act(async () => {
    fireEvent.click(button);
  });
  assert.equal(selected, 1);
  cleanup();

  serveSnapshot(snapshot([card("w/done", { kind: "status", options: [] })]));
  await act(async () => {
    render(sidebarEntry(() => {}));
  });
  await waitFor(() => screen.getByTestId("open-bench-view"));
  assert.equal(
    screen.queryByTestId("sidebar-bench-count"),
    null,
    "no badge when nothing needs the owner",
  );
});

const CRAIG = "d".repeat(64);
const PROJECT_CHANNEL = "105f15cc-1d5a-431a-9826-9402fba1a12b";

function scoped(id, overrides = {}) {
  return card(id, {
    channelId: PROJECT_CHANNEL,
    project: "reset",
    scoped: true,
    answerers: [{ pk: CRAIG, name: "Craig", role: "decider", agent: false }],
    ownerName: "Quicksilver",
    cardEventId: "9".repeat(64),
    ...overrides,
  });
}

test("screen: sections depend on who is viewing; receipts are decided", async () => {
  const items = [
    card("w/mine"),
    scoped("w/craig"),
    card("w/closed", {
      kind: "status",
      options: [],
      state: "done",
      request: scoped("w/closed", { cardEventId: null }),
    }),
  ];
  serveIdentity(OWNER);
  serveSnapshot(snapshot(items));
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  await waitFor(() =>
    assert.deepEqual(cardIds(screen.getByTestId("bench-section-needsYou")), [
      "w/mine",
    ]),
  );
  const waiting = screen.getByTestId("bench-section-waitingOnOthers");
  assert.deepEqual(cardIds(waiting), ["w/craig"]);
  assert.ok(within(waiting).getByText("For Craig · Decider"));
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-decided")), [
    "w/closed",
  ]);
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-status")), []);

  // Other opens the card's thread in its own channel.
  await act(async () => {
    fireEvent.click(
      within(waiting).getByRole("button", {
        name: "Other: reply in this card's thread",
      }),
    );
  });
  await waitFor(() =>
    assert.equal(
      router.history.location.pathname,
      `/channels/${PROJECT_CHANNEL}`,
    ),
  );
  // This bare router JSON-quotes search values; the app's does not.
  assert.match(
    decodeURIComponent(router.history.location.search),
    /^\?thread="?9{64}"?$/,
  );
  cleanup();

  // Anyone else sees every open card under Needs you, read-only.
  serveIdentity(STRANGER);
  serveSnapshot(snapshot(items));
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  const needsYou = await waitFor(() => {
    const section = screen.getByTestId("bench-section-needsYou");
    assert.deepEqual(cardIds(section), ["w/craig", "w/mine"]);
    return section;
  });
  assert.deepEqual(
    cardIds(screen.getByTestId("bench-section-waitingOnOthers")),
    [],
  );
  assert.ok(
    within(needsYou).getByText(
      "For Craig. Only Craig or Quicksilver can answer.",
    ),
  );
});

test("sidebar: the badge leaves out cards waiting on others", async () => {
  serveSnapshot(snapshot([card("w/mine"), scoped("w/craig")]));
  await act(async () => {
    render(sidebarEntry(() => {}));
  });
  await waitFor(() =>
    assert.equal(screen.getByTestId("sidebar-bench-count").textContent, "1"),
  );
});

test("screen: search narrows every section; Open thread opens the card's thread", async () => {
  serveIdentity(OWNER);
  serveSnapshot(
    snapshot([
      card("w/merge", { title: "Merge PR 689?" }),
      card("w/held", {
        kind: "status",
        options: [],
        state: "blocked",
        title: "#689 on hold",
        cardEventId: "7".repeat(64),
      }),
      card("w/other", { title: "Rotate the keys" }),
      card("w/posting", { kind: "status", options: [], cardEventId: null }),
    ]),
  );
  await act(async () => {
    render(wrap(createElement(BenchScreen)));
  });
  const search = await waitFor(() =>
    screen.getByRole("searchbox", { name: "Search the Bench" }),
  );
  await act(async () => {
    fireEvent.change(search, { target: { value: "#689" } });
  });
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-needsYou")), [
    "w/merge",
  ]);
  const held = screen.getByTestId("bench-section-held");
  assert.deepEqual(cardIds(held), ["w/held"]);
  assert.deepEqual(cardIds(screen.getByTestId("bench-section-status")), []);
  assert.ok(
    within(screen.getByTestId("bench-section-decided")).getByText(
      "No matching cards.",
    ),
  );

  // A status card's thread is in #bench: the snapshot's channel.
  await act(async () => {
    fireEvent.click(within(held).getByRole("button", { name: "Open thread" }));
  });
  await waitFor(() =>
    assert.equal(
      router.history.location.pathname,
      "/channels/8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10",
    ),
  );
  assert.match(
    decodeURIComponent(router.history.location.search),
    /^\?thread="?7{64}"?$/,
  );

  await act(async () => {
    fireEvent.change(search, { target: { value: "" } });
  });
  const status = screen.getByTestId("bench-section-status");
  assert.deepEqual(cardIds(status), ["w/posting"]);
  assert.equal(
    within(status).queryByRole("button", { name: "Open thread" }),
    null,
    "no thread before the card is posted",
  );
});
