/**
 * The Inbox hover preview, mounted for real around a row shaped like the
 * Inbox's own: it opens 700 ms after the pointer settles on the row, at once
 * on keyboard focus, never over the avatar, closes on
 * Escape and click, and holds no buttons or links.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

let act;
let cleanup;
let fireEvent;
let render;
let screen;
let createElement;
let QueryClient;
let QueryClientProvider;
let TooltipProvider;
let RouterContextProvider;
let createMemoryHistory;
let createRootRoute;
let createRouter;
let InboxItemPreview;
let INBOX_PREVIEW_OPEN_DELAY_MS;

const sleep = (ms) =>
  act(() => new Promise((resolve) => setTimeout(resolve, ms)));

function feedEvent(id) {
  return {
    id,
    kind: 9,
    pubkey: "c".repeat(64),
    content: "Hello **world**",
    createdAt: 1,
    channelId: "general",
    channelName: "general",
    tags: [],
    category: "mention",
  };
}

const ITEM = {
  conversationId: "1".repeat(64),
  id: "1".repeat(64),
  item: feedEvent("1".repeat(64)),
  groupItems: [feedEvent("1".repeat(64)), feedEvent("2".repeat(64))],
  isActionRequired: false,
  mentionNames: [],
  preview: "Hello **world**",
  senderLabel: "Ana",
  subject: "Mention",
  timestampLabel: "2:05 PM",
};

const MENTION = { type: "mention", workbench: "Dreamforge", benchCard: null };
const LABEL = { text: "Mentioned in", channelLabel: "general" };

function row({ inboxClass = MENTION } = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { gcTime: 0, retry: false } },
  });
  // Only the router context the markdown renderer reads; no route resolves.
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  return createElement(
    RouterContextProvider,
    { router },
    createElement(
      QueryClientProvider,
      { client },
      createElement(
        TooltipProvider,
        null,
        createElement(
          InboxItemPreview,
          {
            channelDescription: "Where the team talks",
            inboxClass,
            item: ITEM,
            label: LABEL,
          },
          createElement(
            "div",
            null,
            createElement("button", { type: "button" }, "Open inbox item"),
            createElement(
              "span",
              { "data-inbox-profile-trigger": "true", "data-testid": "avatar" },
              "A",
            ),
            createElement("p", { "data-testid": "row-text" }, "Hello world"),
            createElement(
              "div",
              { "data-inbox-preview-ignore": "", "data-testid": "actions" },
              createElement("button", { type: "button" }, "Mark as read"),
            ),
          ),
        ),
      ),
    ),
  );
}

const preview = () => screen.queryByTestId("inbox-item-preview");

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
  dom.window.ResizeObserver = globalThis.ResizeObserver;

  ({ act, cleanup, fireEvent, render, screen } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip"));
  ({
    RouterContextProvider,
    createMemoryHistory,
    createRootRoute,
    createRouter,
  } = await import("@tanstack/react-router"));
  ({ InboxItemPreview, INBOX_PREVIEW_OPEN_DELAY_MS } = await import(
    "./InboxItemPreview.tsx"
  ));
});

afterEach(() => cleanup?.());
after(() => dom.window.close());

test("opens after the hover delay; shows type, project, channel and message", async () => {
  assert.equal(INBOX_PREVIEW_OPEN_DELAY_MS, 700);
  await act(async () => {
    render(row());
  });
  await act(async () => {
    fireEvent.mouseOver(screen.getByTestId("row-text"));
  });
  await sleep(350);
  assert.equal(preview(), null, "not before the delay");
  await sleep(450);
  const shown = preview();
  assert.ok(shown, "open after 700 ms");
  const text = shown.textContent;
  for (const part of [
    "Mention",
    "Dreamforge",
    "#general",
    "2:05 PM",
    "Hello world",
    "Where the team talks",
    "2 messages",
  ]) {
    assert.ok(text.includes(part), part);
  }
  // Markdown renders, raw text does not leak its asterisks.
  assert.equal(text.includes("**"), false);
  assert.equal(shown.querySelectorAll("button, a").length, 0);

  await act(async () => {
    fireEvent.keyDown(dom.window.document, { key: "Escape" });
  });
  assert.equal(preview(), null, "Escape closes it");
  // Dismissed until the pointer leaves the row.
  await act(async () => {
    fireEvent.mouseOver(screen.getByTestId("row-text"));
  });
  await sleep(800);
  assert.equal(preview(), null);
});

test("never over the avatar or the action bar, and a click closes it", async () => {
  await act(async () => {
    render(row());
  });
  for (const id of ["avatar", "actions"]) {
    await act(async () => {
      fireEvent.mouseOver(screen.getByTestId(id));
    });
    await sleep(800);
    assert.equal(preview(), null, id);
  }
  await act(async () => {
    fireEvent.mouseOver(screen.getByTestId("row-text"));
  });
  await sleep(800);
  assert.ok(preview());
  await act(async () => {
    fireEvent.pointerDown(screen.getByTestId("row-text"));
  });
  assert.equal(preview(), null, "a click closes it");
});

test("keyboard focus opens it at once; a Bench card shows its options inertly", async () => {
  const bench = {
    type: "needs-you",
    workbench: "28 Day Reset",
    benchCard: {
      id: "orchestrator/ask",
      writer: "orchestrator",
      kind: "pending",
      title: "Send the week 2 email?",
      body: "Line one\nLine two\nLine three\nLine four",
      area: "28-day-reset",
      severity: "attention",
      options: ["Send", "Hold", "Rewrite", "Skip", "Fifth"],
      state: null,
      deadline: null,
      validity: "current",
      cardEventId: "e".repeat(64),
      answer: null,
      recorded: 1,
      recommend: 1,
      plain: { whatChanges: "Guests get the email", risk: "high" },
    },
  };
  await act(async () => {
    render(row({ inboxClass: bench }));
  });
  // Tab from elsewhere: the focus is keyboard focus (:focus-visible).
  await act(async () => {
    fireEvent.keyDown(dom.window.document.body, { key: "Tab" });
    screen.getByRole("button", { name: "Open inbox item" }).focus();
  });
  const shown = preview();
  assert.ok(shown, "opens without waiting");
  const text = shown.textContent;
  for (const part of [
    "Needs you",
    "28 Day Reset",
    "Send the week 2 email?",
    "Attention",
    "High risk",
    "What changes: Guests get the email",
    "1. Send · Recommended",
    "2. Hold · Answered",
    "4. Skip",
  ]) {
    assert.ok(text.includes(part), part);
  }
  assert.equal(text.includes("Line four"), false, "first 3 body lines");
  assert.equal(text.includes("Fifth"), false, "up to 4 options");
  assert.equal(shown.querySelectorAll("button, a").length, 0);
  // Focus stays on the row: the preview takes none.
  assert.equal(
    dom.window.document.activeElement?.textContent,
    "Open inbox item",
  );
});
