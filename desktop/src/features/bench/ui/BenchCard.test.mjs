/**
 * Production-seam pins for the Bench card in #bench: the real `BenchCard`
 * and `useBenchCardForMessage` mounted against a mocked Tauri IPC boundary.
 *
 * - Agent-written text stays inert: markdown and HTML in the title, body and
 *   options render as literal text, and only https links become anchors.
 * - A click sends `bench_answer` with the 0-based option, shows "Saving..."
 *   until the snapshot's `recorded` moves, then "Recorded: n. option"; a
 *   click on the recorded option sends `option: null` (undo).
 * - A command error shows inline; read-only cards send nothing.
 * - The row hook matches only the live card signed by the writer key.
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
let createElement;
let QueryClient;
let QueryClientProvider;
let TooltipProvider;
let BenchCard;
let useBenchCardForMessage;

const CHANNEL = "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10";
const WRITER = "a".repeat(64);
const OWNER = "b".repeat(64);
const CARD_ID = "e".repeat(64);

function card(overrides = {}) {
  return {
    id: "dreamforge/ship-it",
    writer: "dreamforge",
    kind: "pending",
    title: "Ship the release?",
    body: "Two options.",
    area: "release",
    severity: "critical",
    options: ["Ship it", "Hold"],
    state: null,
    decidedBy: null,
    deadline: null,
    links: [],
    order: 0,
    validity: "current",
    cardEventId: CARD_ID,
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

function wrap(element) {
  const client = new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0 },
      queries: { gcTime: 0, retry: false },
    },
  });
  clients.push(client);
  return createElement(
    QueryClientProvider,
    { client },
    createElement(TooltipProvider, null, element),
  );
}

function benchCard(props) {
  return createElement(BenchCard, {
    ownerPubkey: OWNER,
    viewerPubkey: OWNER,
    ...props,
  });
}

function answerCalls() {
  return ipcCalls.filter((c) => c.cmd === "bench_answer").map((c) => c.args);
}

function optionButton(name) {
  return screen.getByRole("button", { name: new RegExp(`^${name}`) });
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
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

  ({ act, cleanup, fireEvent, render, screen, waitFor } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip"));
  ({ BenchCard } = await import("./BenchCard.tsx"));
  ({ useBenchCardForMessage } = await import("../useBenchCards.ts"));
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

test("agent text renders inert: no markdown, no HTML, no unsafe links", async () => {
  const hostile = card({
    title: "**Bold** <b>x</b> # heading",
    body: "[click](https://evil.example) ![](https://evil.example/p.png)\n@alice #general <img src=x onerror=alert(1)> `code` ~~strike~~ \\(x\\)",
    options: ["<i>Ship</i> *now*", "| a | b |"],
    links: [
      { label: "PR 42", url: "https://github.com/block/buzz/pull/42" },
      { label: "Evil", url: "javascript:alert(1)" },
      { label: "Plain", url: "http://github.com/x" },
    ],
  });
  let view;
  await act(async () => {
    view = render(wrap(benchCard({ card: hostile })));
  });
  const root = view.container.querySelector("[data-testid='bench-card']");
  assert.ok(root, "the card renders");
  for (const tag of ["img", "b", "i", "strong", "em", "code", "del", "h1"]) {
    assert.equal(root.querySelector(tag), null, `no <${tag}> from agent text`);
  }
  assert.ok(root.textContent.includes("**Bold** <b>x</b> # heading"));
  assert.ok(root.textContent.includes("<img src=x onerror=alert(1)>"));
  assert.ok(root.textContent.includes("[click](https://evil.example)"));
  assert.ok(root.textContent.includes("<i>Ship</i> *now*"));
  const anchors = [...root.querySelectorAll("a")];
  assert.deepEqual(
    anchors.map((a) => [a.textContent, a.getAttribute("href")]),
    [["PR 42", "https://github.com/block/buzz/pull/42"]],
  );
  assert.ok(
    screen.getByText("Your decision"),
    "severity chip has a readable label",
  );
});

test("a click answers, shows Saving... until recorded, then the Recorded line", async () => {
  ipcHandlers.set("bench_answer", () => Promise.resolve(null));
  let view;
  await act(async () => {
    view = render(wrap(benchCard({ card: card() })));
  });

  await act(async () => {
    fireEvent.click(optionButton("1. Ship it"));
  });
  assert.deepEqual(answerCalls(), [
    { itemId: "dreamforge/ship-it", option: 0 },
  ]);
  assert.ok(screen.getByText("Saving..."));
  assert.equal(optionButton("1. Ship it").getAttribute("aria-busy"), "true");
  assert.equal(optionButton("2. Hold").disabled, true, "others wait");

  // A second click while saving sends nothing.
  await act(async () => {
    fireEvent.click(optionButton("1. Ship it"));
  });
  assert.equal(answerCalls().length, 1);

  const answeredAt = Math.floor(Date.now() / 1000);
  await act(async () => {
    view.rerender(
      wrap(
        benchCard({
          card: card({ recorded: 0, answer: { option: 0, answeredAt } }),
        }),
      ),
    );
  });
  assert.equal(screen.queryByText("Saving..."), null);
  const recorded = screen.getByTestId("bench-card-recorded");
  assert.match(recorded.textContent, /^Recorded: 1\. Ship it · \S/);
  assert.equal(optionButton("1. Ship it").getAttribute("aria-pressed"), "true");
  assert.equal(optionButton("2. Hold").getAttribute("aria-pressed"), "false");
});

test("clicking the recorded option undoes it with option: null", async () => {
  ipcHandlers.set("bench_answer", () => Promise.resolve(null));
  const answered = card({ recorded: 1, answer: { option: 1, answeredAt: 1 } });
  let view;
  await act(async () => {
    view = render(wrap(benchCard({ card: answered })));
  });
  await act(async () => {
    fireEvent.click(optionButton("2. Hold"));
  });
  assert.deepEqual(answerCalls(), [
    { itemId: "dreamforge/ship-it", option: null },
  ]);
  assert.ok(screen.getByText("Saving..."));
  await act(async () => {
    view.rerender(wrap(benchCard({ card: card() })));
  });
  assert.equal(screen.queryByText("Saving..."), null);
  assert.equal(screen.queryByTestId("bench-card-recorded"), null);
});

test("a command error shows inline and re-enables the options", async () => {
  ipcHandlers.set("bench_answer", () =>
    Promise.reject("owner identity changed; refusing to answer"),
  );
  await act(async () => {
    render(wrap(benchCard({ card: card() })));
  });
  await act(async () => {
    fireEvent.click(optionButton("2. Hold"));
  });
  assert.equal(screen.queryByText("Saving..."), null);
  assert.ok(
    screen.getByText(
      "Couldn't save: owner identity changed; refusing to answer",
    ),
  );
  assert.equal(optionButton("1. Ship it").disabled, false);
});

test("read-only cards send nothing; decision cards have no options", async () => {
  await act(async () => {
    render(wrap(benchCard({ card: card(), viewerPubkey: "c".repeat(64) })));
  });
  assert.equal(optionButton("1. Ship it").disabled, true);
  assert.ok(screen.getByText("Only the Bench owner can answer this card."));
  await act(async () => {
    fireEvent.click(optionButton("1. Ship it"));
  });
  assert.equal(answerCalls().length, 0);
  cleanup();

  await act(async () => {
    render(
      wrap(
        benchCard({
          card: card({ kind: "decision", options: [], decidedBy: "russell" }),
        }),
      ),
    );
  });
  assert.equal(screen.queryAllByRole("button").length, 0);
  assert.ok(screen.getByText("Decided by russell"));
});

function RowProbe({ message, onMatch }) {
  onMatch(useBenchCardForMessage(message));
  return null;
}

test("row hook: only the writer-signed live card in #bench is a match", async () => {
  const snapshot = {
    enabled: true,
    channelId: CHANNEL,
    writerPubkey: WRITER,
    ownerPubkey: OWNER,
    lastError: null,
    lastTick: "2026-10-12T10:00:00Z",
    items: [card()],
  };
  ipcHandlers.set("bench_cards", () => Promise.resolve(snapshot));
  const rows = {
    card: { channelId: CHANNEL, id: CARD_ID, kind: 9, signerPubkey: WRITER },
    forged: { channelId: CHANNEL, id: CARD_ID, kind: 9, signerPubkey: OWNER },
    elsewhere: {
      channelId: "1d2c3b4a-0000-4000-8000-000000000000",
      id: CARD_ID,
      kind: 9,
      signerPubkey: WRITER,
    },
  };
  const seen = {};
  await act(async () => {
    render(
      wrap(
        createElement(
          "div",
          null,
          ...Object.entries(rows).map(([name, message]) =>
            createElement(RowProbe, {
              key: name,
              message,
              onMatch: (match) => {
                seen[name] = match;
              },
            }),
          ),
        ),
      ),
    );
  });
  await waitFor(() => assert.equal(seen.card?.card.id, "dreamforge/ship-it"));
  assert.equal(seen.card?.ownerPubkey, OWNER);
  assert.equal(seen.forged, null, "another signer is plain chat");
  assert.equal(seen.elsewhere, null, "another channel is plain chat");
  assert.equal(
    ipcCalls.filter((c) => c.cmd === "bench_cards").length,
    1,
    "rows share one snapshot fetch",
  );
});

const CRAIG = "d".repeat(64);

function scoped(overrides = {}) {
  return card({
    channelId: "105f15cc-1d5a-431a-9826-9402fba1a12b",
    project: "reset",
    scoped: true,
    answerers: [{ pk: CRAIG, name: "Craig", role: "decider", agent: false }],
    ownerName: "Quicksilver",
    recommend: 2,
    recommender: "Reset Worker",
    ...overrides,
  });
}

test("a scoped card shows who it is for, the recommendation and Other", async () => {
  let others = 0;
  await act(async () => {
    render(
      wrap(
        benchCard({
          card: scoped(),
          onOther: () => {
            others += 1;
          },
        }),
      ),
    );
  });
  assert.ok(screen.getByText("For Craig · Decider"));
  const recommended = optionButton("2. Hold");
  assert.equal(recommended.getAttribute("aria-label"), "2. Hold, recommended");
  assert.ok(recommended.textContent.includes("Recommended"));
  assert.equal(
    optionButton("1. Ship it").getAttribute("aria-label"),
    "1. Ship it",
  );
  await act(async () => {
    fireEvent.click(
      screen.getByRole("button", {
        name: "Other: reply in this card's thread",
      }),
    );
  });
  assert.equal(others, 1);
  assert.equal(answerCalls().length, 0, "Other answers nothing itself");
});

test("Other is absent on a legacy card and without a thread to open", async () => {
  const other = () => screen.queryByRole("button", { name: /^Other: reply/ });
  for (const props of [
    { card: card(), onOther: () => {} },
    { card: scoped() },
    { card: scoped({ cardEventId: null }), onOther: () => {} },
  ]) {
    await act(async () => {
      render(wrap(benchCard(props)));
    });
    assert.equal(other(), null);
    cleanup();
  }
});

test("Craig's recorded answer: the owner overrides it, never undoes it", async () => {
  ipcHandlers.set("bench_answer", () => Promise.resolve(null));
  const craigs = scoped({
    recorded: 0,
    answer: {
      option: 0,
      answeredAt: 1,
      by: CRAIG,
      byName: "Craig",
      byOwner: false,
      source: "tap",
    },
  });
  let view;
  await act(async () => {
    view = render(wrap(benchCard({ card: craigs })));
  });
  assert.ok(screen.getByText("Answer as owner (overrides)"));
  assert.match(
    screen.getByTestId("bench-card-recorded").textContent,
    /^Recorded: 1\. Ship it · Craig · \S/,
  );
  await act(async () => {
    fireEvent.click(optionButton("1. Ship it"));
  });
  assert.deepEqual(answerCalls(), [
    { itemId: "dreamforge/ship-it", option: 0 },
  ]);
  // Craig's 1 still shows: not yet the owner's answer.
  assert.ok(screen.getByText("Saving..."));
  await act(async () => {
    view.rerender(
      wrap(
        benchCard({
          card: scoped({
            recorded: 0,
            answer: {
              option: 0,
              answeredAt: 2,
              by: OWNER,
              byName: null,
              byOwner: true,
              source: "tap",
            },
          }),
        }),
      ),
    );
  });
  assert.equal(screen.queryByText("Saving..."), null);
  assert.equal(screen.queryByText("Answer as owner (overrides)"), null);
  assert.match(
    screen.getByTestId("bench-card-recorded").textContent,
    /^Recorded: 1\. Ship it · Quicksilver · \S/,
  );
});

test("a receipt shows the request, the answer and the outcome as inert text", async () => {
  const request = scoped({
    title: "Send the <b>new</b> email?",
    options: ["Send the new email", "Keep the old one"],
    cardEventId: null,
    recorded: 0,
    answer: {
      option: 0,
      answeredAt: 1,
      by: CRAIG,
      byName: "Craig",
      byOwner: false,
      source: "tap",
    },
  });
  const receipt = card({
    kind: "decision",
    severity: null,
    options: [],
    decidedBy: "claude",
    title: "Sent the **new** email",
    body: "Live since 14:20. <img src=x onerror=alert(1)>",
    links: [{ label: "PR 9", url: "https://github.com/x/y/pull/9" }],
    channelId: request.channelId,
    request,
  });
  let view;
  await act(async () => {
    view = render(wrap(benchCard({ card: receipt, onOther: () => {} })));
  });
  const root = view.container.querySelector("[data-testid='bench-card']");
  assert.equal(root.querySelector("img"), null);
  assert.equal(root.querySelector("b"), null);
  assert.ok(root.textContent.includes("Send the <b>new</b> email?"));
  const chosen = optionButton("1. Send the new email");
  assert.equal(chosen.disabled, true);
  assert.equal(chosen.getAttribute("aria-pressed"), "true");
  assert.equal(optionButton("2. Keep the old one").disabled, true);
  assert.match(
    screen.getByTestId("bench-card-answered").textContent,
    /^Answered: 1\. Send the new email · Craig · \S/,
  );
  const outcome = screen.getByTestId("bench-card-outcome");
  assert.match(outcome.textContent, /^Outcome · \S/);
  assert.ok(outcome.textContent.includes("Sent the **new** email"));
  assert.ok(outcome.textContent.includes("<img src=x onerror=alert(1)>"));
  assert.ok(outcome.textContent.includes("Decided by claude"));
  assert.deepEqual(
    [...outcome.querySelectorAll("a")].map((a) => a.getAttribute("href")),
    ["https://github.com/x/y/pull/9"],
  );
  assert.equal(screen.queryByRole("button", { name: /^Other: reply/ }), null);
});
