import assert from "node:assert/strict";
import test from "node:test";

import { fallbackProjectColor } from "@/features/projects/lib/projectColors.ts";
import {
  classifyInboxItem,
  inboxCardPost,
  inboxProjects,
  inboxRepoDtag,
} from "./inboxClass.ts";

const WRITER = "a".repeat(64);
const OWNER = "b".repeat(64);
const STRANGER = "c".repeat(64);
const BENCH = "bench-channel";
const RESET_CHANNEL = "support-shadow";
const CARD_EVENT = "e".repeat(64);
const REPO = `30617:${OWNER}:buzz`;

const PROJECTS = [
  {
    id: `30621:${OWNER}:28-day-reset`,
    owner: OWNER,
    dtag: "28-day-reset",
    name: "28 Day Reset",
    color: "lime",
    legacy: false,
    projectChannelId: "reset-home",
    relatedChannelIds: [RESET_CHANNEL],
    repositories: [],
    repositoryAddresses: [],
  },
  {
    id: `30621:${OWNER}:garden`,
    owner: OWNER,
    dtag: "garden",
    name: "Garden club",
    legacy: false,
    projectChannelId: "garden-home",
    relatedChannelIds: [],
    repositories: [
      {
        id: "r",
        name: "r",
        channelId: "garden-repo",
        owner: OWNER,
        repoAddress: `30617:${OWNER}:garden-app`,
      },
    ],
    repositoryAddresses: [`30617:${OWNER}:garden-app`],
  },
];

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
const NONE = inboxProjects([], OWNER);

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

test("only a well-formed client marker makes a post the card's (shared parser)", () => {
  const bench = snapshot([card("claude-ai/ship")]);
  const post = (tags) =>
    inboxItem(event({ pubkey: WRITER, channelId: BENCH, tags }));
  assert.equal(
    inboxCardPost(post([marker("claude-ai/ship")]), bench)?.id,
    "claude-ai/ship",
  );
  for (const tags of [
    [["client", "bench:card:claude-ai/ship"]],
    [["t", "bench:card:claude-ai/ship@0a1b2c"]],
    [["client", "bench:card:claude-ai/other@0a1b2c"]],
  ]) {
    assert.equal(inboxCardPost(post(tags), bench), null, JSON.stringify(tags));
  }
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

test("only the card's own post shows as the card; a reply in its thread shows as itself", () => {
  const bench = snapshot([card("claude-ai/ship", { cardEventId: CARD_EVENT })]);
  const post = inboxItem(
    event({
      pubkey: WRITER,
      channelId: BENCH,
      tags: [marker("claude-ai/ship")],
    }),
  );
  const reply = inboxItem(
    event({ channelId: BENCH, tags: [["e", CARD_EVENT, "", "root"]] }),
    { conversationId: CARD_EVENT },
  );
  const cardOf = (item) => inboxCardPost(item, bench);
  assert.equal(cardOf(post)?.id, "claude-ai/ship");
  assert.equal(cardOf(reply), null);
  // A member's reply that forges the card's marker is still the reply: the
  // row is the card's thread, but its text is the member's.
  const forged = inboxItem(
    event({
      pubkey: STRANGER,
      channelId: BENCH,
      tags: [["e", CARD_EVENT, "", "root"], marker("claude-ai/ship")],
    }),
    { conversationId: CARD_EVENT },
  );
  assert.equal(
    classifyInboxItem(forged, bench, NONE).benchCard?.id,
    "claude-ai/ship",
  );
  assert.equal(cardOf(forged), null);
  // Nor is the writer's marked post outside #bench and the card's channel.
  const elsewhere = inboxItem(
    event({
      pubkey: WRITER,
      channelId: "other",
      tags: [marker("claude-ai/ship")],
    }),
  );
  assert.equal(cardOf(elsewhere), null);
  // Another card's marker on the shown event is not this card's post.
  const other = inboxItem(
    event({
      pubkey: WRITER,
      channelId: BENCH,
      tags: [marker("claude-ai/other")],
    }),
    { conversationId: CARD_EVENT },
  );
  assert.equal(cardOf(other), null);
  assert.equal(inboxCardPost(post, null), null);
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
  const lookup = inboxProjects(PROJECTS, OWNER);
  const chip = (channelId) =>
    classifyInboxItem(inboxItem(event({ channelId })), null, lookup).workbench;
  assert.equal(chip(RESET_CHANNEL), "28 Day Reset");
  assert.equal(chip("reset-home"), "28 Day Reset");
  assert.equal(chip("garden-repo"), "Garden club");
  assert.equal(chip("general"), null);
});

test("a chip carries its project's color; a row with no project has none", () => {
  const lookup = inboxProjects(PROJECTS, OWNER);
  const color = (item, bench = null) =>
    classifyInboxItem(inboxItem(item), bench, lookup).color?.id ?? null;
  const garden = fallbackProjectColor("garden").id;

  assert.equal(color(event({ channelId: RESET_CHANNEL })), "lime");
  assert.equal(color(event({ channelId: "garden-repo" })), garden);
  assert.equal(color(event()), null);

  // A Bench card takes its workbench's project color; Dreamforge, with no
  // project here, the fixed color of its first slug.
  const post = (id) =>
    event({ pubkey: WRITER, channelId: BENCH, tags: [marker(id)] });
  const cards = snapshot([
    card("orchestrator/a", { writer: "orchestrator", area: "28-day-reset" }),
    card("claude-ai/b"),
  ]);
  assert.equal(color(post("orchestrator/a"), cards), "lime");
  assert.equal(
    color(post("claude-ai/b"), cards),
    fallbackProjectColor("dreamforge").id,
  );

  // A git row: the project listing its repository, else the repo's d-tag.
  const review = event({
    kind: 1618,
    tags: [["a", `30617:${OWNER}:garden-app`]],
  });
  const reviewClass = classifyInboxItem(inboxItem(review), null, lookup);
  assert.equal(reviewClass.workbench, "Garden club");
  assert.equal(reviewClass.color?.id, garden);
  const notes = event({ kind: 1621, tags: [["a", `30617:${OWNER}:notes`]] });
  assert.equal(color(notes), fallbackProjectColor("notes").id);
});

test("someone else's project can't label or color the viewer's rows", () => {
  const repo = `30617:${OWNER}:payments-api`;
  const mine = {
    ...PROJECTS[1],
    id: `30621:${OWNER}:payments`,
    dtag: "payments",
    name: "Payments",
    color: "sky",
    createdAt: 100,
    projectChannelId: "payments-home",
    repositories: [{ id: "p", name: "p", owner: OWNER, repoAddress: repo }],
    repositoryAddresses: [repo],
  };
  const theirs = {
    ...mine,
    id: `30621:${STRANGER}:approve`,
    owner: STRANGER,
    dtag: "approve",
    name: "Approve now - Russell",
    color: "red",
    createdAt: 200,
  };
  const review = event({ kind: 1618, tags: [["a", repo]] });
  const chipOf = (projects, item = review) =>
    classifyInboxItem(inboxItem(item), null, inboxProjects(projects, OWNER));
  for (const projects of [
    [theirs, mine],
    [mine, theirs],
  ]) {
    assert.equal(chipOf(projects).workbench, "Payments");
    assert.equal(chipOf(projects).color?.id, "sky");
    const home = event({ channelId: "payments-home" });
    assert.equal(chipOf(projects, home).workbench, "Payments");
  }
  // A repository its owner doesn't own or maintain is never theirs to claim.
  assert.equal(chipOf([theirs]).workbench, "payments-api");
  // A recolor republishes (newer createdAt) and moves no other chip.
  const second = { ...mine, id: `30621:${OWNER}:billing`, name: "Billing" };
  for (const createdAt of [50, 500]) {
    assert.equal(
      chipOf([{ ...second, createdAt, color: "teal" }, mine]).workbench,
      "Billing",
    );
  }
});

test("an agent-owned Reset project colors the Reset chip for its owner", () => {
  const AGENT = "d".repeat(64);
  const agentReset = {
    ...PROJECTS[0],
    id: `30621:${AGENT}:28-day-reset`,
    owner: AGENT,
    color: "orange",
  };
  const home = event({ channelId: "reset-home" });
  const chipOf = (agents) =>
    classifyInboxItem(
      inboxItem(home),
      null,
      inboxProjects([agentReset], WRITER, agents),
    ).color?.id;
  assert.equal(chipOf(new Set([AGENT])), "orange");
  assert.equal(chipOf(undefined), fallbackProjectColor("28-day-reset").id);
});
