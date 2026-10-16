import assert from "node:assert/strict";
import test from "node:test";

import {
  channelProjects,
  classifyInboxItem,
  inboxRepoDtag,
} from "./inboxClass.ts";

const WRITER = "a".repeat(64);
const OWNER = "b".repeat(64);
const STRANGER = "c".repeat(64);
const BENCH = "bench-channel";
const RESET_CHANNEL = "support-shadow";
const CARD_EVENT = "e".repeat(64);
const REPO = `30617:${OWNER}:buzz`;

function card(id, overrides = {}) {
  return {
    id,
    writer: "claude-ai",
    kind: "decision",
    title: id,
    body: "",
    area: "dreamforge-build",
    severity: null,
    options: [],
    state: null,
    validity: "current",
    cardEventId: null,
    answer: null,
    channelId: null,
    project: null,
    answerers: [],
    request: null,
    ...overrides,
  };
}

function snapshot(items) {
  return {
    enabled: true,
    channelId: BENCH,
    writerPubkey: WRITER,
    ownerPubkey: OWNER,
    lastError: null,
    lastTick: "",
    items,
  };
}

function event(overrides = {}) {
  return {
    id: "1".repeat(64),
    kind: 9,
    pubkey: STRANGER,
    content: "hello",
    createdAt: 1,
    channelId: "general",
    channelName: "general",
    tags: [],
    category: "activity",
    ...overrides,
  };
}

function inboxItem(primary, overrides = {}) {
  return {
    conversationId: primary.id,
    id: primary.id,
    item: primary,
    groupItems: [primary],
    isActionRequired: false,
    ...overrides,
  };
}

const marker = (id) => ["client", `bench:card:${id}@0a1b2c`];
const NONE = new Map();

test("a signed Bench post is Bench, with its card's project", () => {
  const post = event({
    pubkey: WRITER,
    channelId: BENCH,
    tags: [marker("claude-ai/ship"), ["p", OWNER]],
    category: "mention",
  });
  const result = classifyInboxItem(
    inboxItem(post),
    snapshot([card("claude-ai/ship")]),
    NONE,
  );
  assert.equal(result.type, "bench");
  assert.equal(result.workbench, "Dreamforge");
  assert.equal(result.benchCard?.id, "claude-ai/ship");
});

test("the same marker from any other key, or outside the card's channels, is not Bench", () => {
  const bench = snapshot([card("claude-ai/ship")]);
  const forged = event({
    channelId: BENCH,
    tags: [marker("claude-ai/ship")],
    category: "mention",
  });
  assert.equal(
    classifyInboxItem(inboxItem(forged), bench, NONE).type,
    "mention",
  );
  const elsewhere = event({
    pubkey: WRITER,
    channelId: "general",
    tags: [marker("claude-ai/ship")],
  });
  const result = classifyInboxItem(inboxItem(elsewhere), bench, NONE);
  assert.equal(result.type, "thread");
  assert.equal(result.benchCard, null);
});

test("a retired card's post is still Bench; a card in its own channel too", () => {
  const post = event({
    pubkey: WRITER,
    channelId: RESET_CHANNEL,
    tags: [marker("orchestrator/old")],
  });
  const result = classifyInboxItem(
    inboxItem(post),
    snapshot([
      card("orchestrator/old", {
        writer: "orchestrator",
        area: "28-day-reset",
        validity: "superseded",
        channelId: RESET_CHANNEL,
      }),
    ]),
    NONE,
  );
  assert.equal(result.type, "bench");
  assert.equal(result.workbench, "28 Day Reset");
});

test("a pending, current, unanswered card needs you; answered it is Bench", () => {
  const post = event({
    pubkey: WRITER,
    channelId: BENCH,
    tags: [marker("orchestrator/ask")],
  });
  const ask = card("orchestrator/ask", {
    writer: "orchestrator",
    kind: "pending",
  });
  assert.equal(
    classifyInboxItem(inboxItem(post), snapshot([ask]), NONE).type,
    "needs-you",
  );
  const answered = { ...ask, answer: { option: 0, answeredAt: 1 } };
  assert.equal(
    classifyInboxItem(inboxItem(post), snapshot([answered]), NONE).type,
    "bench",
  );
});

test("a reply in a card's thread is that card's, by the card's own event id", () => {
  const reply = event({
    channelId: BENCH,
    tags: [["e", CARD_EVENT, "", "root"]],
  });
  const result = classifyInboxItem(
    inboxItem(reply, { conversationId: CARD_EVENT }),
    snapshot([card("claude-ai/ship", { cardEventId: CARD_EVENT })]),
    NONE,
  );
  assert.equal(result.type, "bench");
  assert.equal(result.workbench, "Dreamforge");
});

test("precedence: needs action beats everything; then Code, DM, Mention, Thread", () => {
  const dm = event({ channelType: "dm", category: "mention" });
  assert.equal(
    classifyInboxItem(inboxItem(dm, { isActionRequired: true }), null, NONE)
      .type,
    "needs-you",
  );
  const review = event({
    kind: 1618,
    tags: [["a", REPO]],
    category: "mention",
  });
  assert.equal(classifyInboxItem(inboxItem(review), null, NONE).type, "code");
  assert.equal(classifyInboxItem(inboxItem(dm), null, NONE).type, "dm");
  assert.equal(
    classifyInboxItem(inboxItem(event({ category: "mention" })), null, NONE)
      .type,
    "mention",
  );
  assert.equal(
    classifyInboxItem(inboxItem(event()), null, NONE).type,
    "thread",
  );
});

test("a git item's chip is its repository's project", () => {
  const review = event({ kind: 1618, tags: [["a", REPO]] });
  assert.equal(inboxRepoDtag(inboxItem(review)), "buzz");
  assert.equal(
    classifyInboxItem(inboxItem(review), null, NONE).workbench,
    "Dreamforge",
  );
  const other = event({ kind: 1621, tags: [["a", `30617:${OWNER}:notes`]] });
  assert.equal(
    classifyInboxItem(inboxItem(other), null, NONE).workbench,
    "notes",
  );
});

test("a channel bound to a project shows that project's chip", () => {
  const lookup = channelProjects([
    {
      dtag: "28-day-reset",
      name: "28 Day Reset",
      projectChannelId: "reset-home",
      relatedChannelIds: [RESET_CHANNEL],
      repositories: [],
    },
    {
      dtag: "garden",
      name: "Garden club",
      projectChannelId: "garden-home",
      relatedChannelIds: [],
      repositories: [{ id: "r", name: "r", channelId: "garden-repo" }],
    },
  ]);
  const chip = (channelId) =>
    classifyInboxItem(inboxItem(event({ channelId })), null, lookup).workbench;
  assert.equal(chip(RESET_CHANNEL), "28 Day Reset");
  assert.equal(chip("reset-home"), "28 Day Reset");
  assert.equal(chip("garden-repo"), "Garden club");
  assert.equal(chip("general"), null);
});
