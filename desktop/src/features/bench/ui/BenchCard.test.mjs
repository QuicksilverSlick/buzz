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
 * - Plain fields and labels a writer typed show as bold-labelled fields in
 *   the phone's order, the action as the command one tap approves, details
 *   apart (long ones folded); a card without them renders as before.
 * - Open thread shows on every card that has a thread.
 * - A retired post is read-only, from its own text: the card's layout, no
 *   answer buttons, a link to the live card when there is one.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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

/** The bold field labels a card shows, in order. */
function labels(root) {
  return [...root.querySelectorAll("p > span.font-semibold")].map(
    (span) => span.textContent,
  );
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
    { itemId: "dreamforge/ship-it", cardEventId: CARD_ID, option: 0 },
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
    { itemId: "dreamforge/ship-it", cardEventId: CARD_ID, option: null },
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
    { itemId: "dreamforge/ship-it", cardEventId: CARD_ID, option: 0 },
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

test("a receipt's header shows where the decision stands, not the request's urgency", async () => {
  const request = card({
    kind: "pending",
    severity: "critical",
    options: ["Merge it", "Hold"],
    cardEventId: null,
    recorded: 1,
  });
  const receipt = card({
    kind: "status",
    state: "blocked",
    severity: null,
    options: [],
    title: "Held",
    request,
  });
  let view;
  await act(async () => {
    view = render(wrap(benchCard({ card: receipt, onOther: () => {} })));
  });
  const root = view.container.querySelector("[data-testid='bench-card']");
  const header = root.firstElementChild.textContent;
  assert.ok(header.includes("Blocked"), header);
  assert.ok(!header.includes("Your decision"), header);
  assert.ok(!header.includes("Pending"), header);
});

test("a plain card shows its chips, lines, recommendation, action and folded details", async () => {
  const merge =
    "gh pr merge 711 --repo QuicksilverSlick/Metabolic-reset --squash";
  let view;
  await act(async () => {
    view = render(
      wrap(
        benchCard({
          card: scoped({
            options: ["Merge PR", "Hold"],
            recommend: 1,
            plain: {
              risk: "high",
              undo: "hard",
              reach: "3,089 members",
              whatChanges: "The <b>new</b> check goes live.",
              askedBy: "Reset Worker",
              ifYouWait: "Members keep the old check.",
              confidence: "high",
              why: "CI is green.",
              details: "Full notes **here**",
            },
            action: merge,
            backToYou: true,
            previousAnswer: {
              option: 1,
              label: "Hold",
              byName: null,
              answeredAt: 1,
            },
          }),
          onOther: () => {},
        }),
      ),
    );
  });
  const root = view.container.querySelector("[data-testid='bench-card']");
  assert.ok(screen.getByText("Back to you"));
  assert.match(root.textContent, /Earlier: Quicksilver chose Hold · \S/);
  assert.equal(root.querySelector("b"), null, "plain text stays inert");
  // The phone's order, each label bold with its colon, each its own line;
  // risk, undo and reach on one line as the card's text prints them.
  const fields = [
    "What changes: The <b>new</b> check goes live.",
    "Asked by: Reset Worker",
    "Risk: High · Undo: Hard · Reach: 3,089 members",
    "If you wait: Members keep the old check.",
    "Recommended: 1. Merge PR · high confidence · CI is green.",
  ];
  const at = fields.map((text) => root.textContent.indexOf(text));
  assert.ok(
    at.every((i, n) => i !== -1 && (n === 0 || i > at[n - 1])),
    String(at),
  );
  assert.deepEqual(labels(root), [
    "What changes:",
    "Asked by:",
    "Risk:",
    "Undo:",
    "Reach:",
    "If you wait:",
    "Recommended:",
  ]);
  for (const chip of ["High risk", "Hard to undo", "Reach: 3,089 members"]) {
    assert.equal(screen.queryByText(chip), null, chip);
  }
  const action = screen.getByTestId("bench-card-action");
  assert.equal(action.textContent, `If you choose Merge PR: ${merge}`);
  assert.equal(action.querySelector("code").textContent, merge);
  // Details apart, under a rule; ones that fit have no button.
  const details = screen.getByTestId("bench-card-details");
  assert.equal(details.textContent, "DetailsFull notes **here**");
  assert.equal(details.querySelector("button"), null);
});

/**
 * jsdom has no layout: every element measures as `lines` lines tall, and a
 * clamped one shows at most 4 of them. Returns the undo.
 */
function measureLines(lines) {
  const proto = dom.window.HTMLElement.prototype;
  Object.defineProperty(proto, "scrollHeight", {
    configurable: true,
    get: () => lines * 16,
  });
  Object.defineProperty(proto, "clientHeight", {
    configurable: true,
    get() {
      return (
        (this.classList.contains("line-clamp-4") ? Math.min(lines, 4) : lines) *
        16
      );
    },
  });
  return () => {
    delete proto.scrollHeight;
    delete proto.clientHeight;
  };
}

test("long details that fit in four lines offer no Show more", async () => {
  const restore = measureLines(3);
  try {
    await act(async () => {
      render(
        wrap(
          benchCard({
            card: card({
              plain: { details: "a fairly long line. ".repeat(16) },
            }),
          }),
        ),
      );
    });
    const details = screen.getByTestId("bench-card-details");
    assert.equal(details.querySelector("button"), null, "nothing is hidden");
  } finally {
    restore();
  }
});

test("labels a writer typed read as fields; long details fold behind Show more", async (t) => {
  const vector = JSON.parse(
    readFileSync(
      new URL(
        "../../../../src-tauri/src/bench/card_layout_vectors.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const long = "Every check passed on head a30d88a. ".repeat(12);
  t.after(measureLines(9));
  let view;
  await act(async () => {
    view = render(
      wrap(
        benchCard({
          card: card({
            ...vector.drop,
            writer: "orchestrator",
            body: `Stay or move?\nWhat happened: not a label\n${vector.drop.body} ${long}`,
          }),
          projectLabel: "28 Day Reset",
        }),
      ),
    );
  });
  const root = view.container.querySelector("[data-testid='bench-card']");
  // The header names the project once, by name.
  const header = root.firstElementChild.textContent;
  assert.ok(header.endsWith("Pending · 28 Day Reset · orchestrator"), header);
  assert.ok(!header.includes("28-day-reset"), header);
  // The same labels as the phone's text, in order; Details goes apart.
  assert.deepEqual(
    labels(root),
    vector.labels.filter((l) => l !== "Details").map((l) => `${l}:`),
  );
  assert.ok(root.textContent.includes("What happened: not a label"));
  assert.ok(
    root.textContent.includes(
      "Risk: Low · Undo: Easy · Reach: administrators using Production.",
    ),
  );
  const details = screen.getByTestId("bench-card-details");
  const text = details.querySelector("p:nth-of-type(2)");
  assert.ok(text.textContent.startsWith("PR #758 changes 1 file"));
  assert.ok(text.classList.contains("line-clamp-4"), "long details fold");
  const toggle = screen.getByRole("button", { name: "Show more" });
  assert.equal(toggle.getAttribute("aria-expanded"), "false");
  assert.equal(toggle.getAttribute("aria-controls"), text.id);
  await act(async () => {
    fireEvent.click(toggle);
  });
  assert.equal(toggle.getAttribute("aria-expanded"), "true");
  assert.equal(toggle.textContent, "Show less");
  assert.ok(!text.classList.contains("line-clamp-4"));
});

test("a card without plain fields shows none of them", async () => {
  let view;
  await act(async () => {
    view = render(wrap(benchCard({ card: scoped({ recommend: 1 }) })));
  });
  const root = view.container.querySelector("[data-testid='bench-card']");
  assert.equal(screen.queryByTestId("bench-card-details"), null);
  assert.deepEqual(labels(root), []);
  assert.equal(screen.queryByTestId("bench-card-action"), null);
  assert.equal(screen.queryByText("Back to you"), null);
  assert.doesNotMatch(root.textContent, /risk|chose|What changes/);
});

test("Open thread is on question, status and receipt cards with a thread", async () => {
  const thread = () => screen.queryByRole("button", { name: "Open thread" });
  const receipt = card({
    kind: "status",
    options: [],
    state: "done",
    request: scoped({ cardEventId: null }),
  });
  for (const shown of [
    card(),
    card({ kind: "status", options: [], state: "blocked" }),
    receipt,
  ]) {
    let opened = 0;
    await act(async () => {
      render(
        wrap(
          benchCard({
            card: shown,
            onOther: () => {
              opened += 1;
            },
          }),
        ),
      );
    });
    await act(async () => {
      fireEvent.click(thread());
    });
    assert.equal(opened, 1, shown.kind);
    cleanup();
  }
  for (const props of [
    { card: card({ cardEventId: null }), onOther: () => {} },
    { card: { ...receipt, cardEventId: null }, onOther: () => {} },
    { card: card() },
  ]) {
    await act(async () => {
      render(wrap(benchCard(props)));
    });
    assert.equal(thread(), null);
    cleanup();
  }
});

test("a retired post is read-only, from its own text, with a link to the live card", async () => {
  // The router reads `self`; its link is the jump to the live card.
  globalThis.self ??= dom.window;
  const {
    RouterContextProvider,
    createMemoryHistory,
    createRootRoute,
    createRouter,
  } = await import("@tanstack/react-router");
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const vector = JSON.parse(
    readFileSync(
      new URL(
        "../../../../src-tauri/src/bench/card_layout_vectors.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  for (const cardEventId of [CARD_ID, null]) {
    await act(async () => {
      render(
        createElement(
          RouterContextProvider,
          { router },
          wrap(
            benchCard({
              card: card({ cardEventId }),
              onOther: () => {},
              retiredPost: { channelId: CHANNEL, text: vector.closed },
            }),
          ),
        ),
      );
    });
    const root = screen.getByTestId("bench-card-retired");
    assert.equal(screen.queryByTestId("bench-card"), null);
    // The posted wording in the card's layout; the live card's is nowhere.
    assert.ok(root.textContent.includes("Which Production change goes first?"));
    assert.ok(!root.textContent.includes("Ship the release?"));
    assert.deepEqual(
      labels(root),
      vector.labels.filter((l) => l !== "Details").map((l) => `${l}:`),
    );
    assert.ok(screen.getByTestId("bench-card-details"));
    assert.deepEqual(
      [...root.querySelectorAll('ol[aria-label="Options"] li')].map(
        (li) => li.textContent,
      ),
      ["1. Merge it", "2. Hold"],
    );
    // No answer buttons, no Other, no Open thread button, no tap line.
    assert.equal(root.querySelector("fieldset"), null);
    assert.equal(root.querySelector("button"), null);
    assert.ok(!root.textContent.includes("Tap one number"));
    assert.ok(root.textContent.includes("Closed: a tap or reply here"));
    const jump = screen.queryByRole("link", { name: "Go to the newer card" });
    if (cardEventId) {
      assert.equal(
        jump?.getAttribute("href"),
        `buzz://message?channel=${CHANNEL}&id=${CARD_ID}`,
      );
    } else {
      assert.equal(jump, null, "no live card to go to");
    }
    cleanup();
  }
  assert.deepEqual(answerCalls(), []);
});
