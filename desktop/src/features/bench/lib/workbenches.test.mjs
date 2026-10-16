import assert from "node:assert/strict";
import test from "node:test";

import {
  cardWorkbench,
  slugWorkbench,
  workbenchCounts,
  workbenchSnapshot,
} from "./workbenches.ts";

function card(id, overrides = {}) {
  return {
    id,
    writer: "someone",
    kind: "pending",
    area: "misc",
    project: null,
    validity: "current",
    answer: null,
    answerers: [],
    severity: null,
    order: 0,
    request: null,
    ...overrides,
  };
}

function snapshot(items) {
  return {
    enabled: true,
    channelId: "bench",
    writerPubkey: "a".repeat(64),
    ownerPubkey: "b".repeat(64),
    lastError: null,
    lastTick: "",
    items,
  };
}

test("a card's project, else its area, else its writer picks the tab", () => {
  assert.equal(
    cardWorkbench(card("o", { writer: "orchestrator", area: "28-day-reset" })),
    "reset",
  );
  assert.equal(cardWorkbench(card("o", { writer: "orchestrator" })), "reset");
  assert.equal(
    cardWorkbench(card("c", { writer: "claude-ai", area: "dreamforge-build" })),
    "dreamforge",
  );
  assert.equal(
    cardWorkbench(card("c", { writer: "upstream-sync" })),
    "dreamforge",
  );
  // The project beats an area and a writer that both say Dreamforge.
  assert.equal(
    cardWorkbench(
      card("p", { writer: "claude-ai", area: "dreamforge", project: "reset" }),
    ),
    "reset",
  );
  // An area that names no workbench falls through to the writer.
  assert.equal(
    cardWorkbench(card("b", { writer: "claude-ai", area: "bench" })),
    "dreamforge",
  );
  assert.equal(cardWorkbench(card("u", { writer: "stranger" })), "other");
});

test("slugs match whole words or a dash-prefix; names go through the d-tag", () => {
  assert.equal(slugWorkbench("28 Day Reset")?.id, "reset");
  assert.equal(slugWorkbench("reset-q4")?.id, "reset");
  assert.equal(slugWorkbench("Dreamforge")?.id, "dreamforge");
  assert.equal(slugWorkbench("buzz")?.id, "dreamforge");
  assert.equal(slugWorkbench("bench"), null);
  assert.equal(slugWorkbench("resetting"), null);
  assert.equal(slugWorkbench("buzzword"), null);
  assert.equal(slugWorkbench(""), null);
  assert.equal(slugWorkbench(null), null);
});

test("counts per tab: current cards and what needs you; tabs filter items", () => {
  const items = [
    card("o/ask", { writer: "orchestrator" }),
    card("o/old", { writer: "orchestrator", validity: "superseded" }),
    card("c/ask", { writer: "claude-ai" }),
    card("c/answered", {
      writer: "claude-ai",
      answer: { option: 0, answeredAt: 1 },
    }),
    card("x/status", { kind: "status", state: "done" }),
  ];
  const counts = workbenchCounts(snapshot(items));
  assert.deepEqual(counts, {
    all: { cards: 4, needsYou: 2 },
    reset: { cards: 1, needsYou: 1 },
    dreamforge: { cards: 2, needsYou: 1 },
    other: { cards: 1, needsYou: 0 },
  });
  assert.deepEqual(
    workbenchSnapshot(snapshot(items), "reset").items.map((c) => c.id),
    ["o/ask", "o/old"],
  );
  assert.equal(workbenchSnapshot(snapshot(items), "all").items.length, 5);
});
