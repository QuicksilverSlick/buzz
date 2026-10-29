import assert from "node:assert/strict";
import test from "node:test";

import {
  benchLastTickSeconds,
  benchNeedsYouCount,
  benchSections,
  benchSeverityRank,
} from "./benchScreen.ts";

function card(id, overrides = {}) {
  return {
    id,
    writer: "dreamforge",
    kind: "pending",
    title: id,
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
    cardEventId: "e".repeat(64),
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

function snapshot(items) {
  return {
    enabled: true,
    channelId: "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10",
    writerPubkey: "a".repeat(64),
    ownerPubkey: "b".repeat(64),
    lastError: null,
    lastTick: "2026-10-12T10:00:00Z",
    items,
  };
}

const ids = (cards) => cards.map((c) => c.id);

test("sections: each item lands in exactly one section, like the board", () => {
  const sections = benchSections(
    snapshot([
      card("w/open"),
      card("w/answered", { answer: { option: 1, answeredAt: 50 } }),
      card("w/decided", { kind: "decision", options: [], decidedBy: "claude" }),
      card("w/status", { kind: "status", options: [], state: "done" }),
      card("w/stale", { validity: "suspected-stale" }),
      card("w/gone", { kind: "status", validity: "superseded" }),
    ]),
  );
  assert.deepEqual(ids(sections.needsYou), ["w/open"]);
  assert.deepEqual(ids(sections.answered), ["w/answered"]);
  assert.deepEqual(ids(sections.decided), ["w/decided"]);
  assert.deepEqual(ids(sections.status), ["w/status"]);
  assert.equal(sections.retiring, 2, "stale and superseded are folded");
});

test("sections: one tick's needs you cards sort by severity, then order, then id", () => {
  const sections = benchSections(
    snapshot([
      card("w/none", { severity: null }),
      card("w/info", { severity: "info" }),
      card("w/attention-late", { severity: "attention", order: 2 }),
      card("w/critical", { severity: "critical", order: 9 }),
      card("w/attention-b", { severity: "attention", order: 1 }),
      card("w/attention-a", { severity: "attention", order: 1 }),
      card("w/good", { severity: "good" }),
    ]),
  );
  assert.deepEqual(ids(sections.needsYou), [
    "w/critical",
    "w/attention-a",
    "w/attention-b",
    "w/attention-late",
    "w/info",
    "w/good",
    "w/none",
  ]);
});

test("sections: one tick's answered cards are oldest answer first; decided and status by area", () => {
  const sections = benchSections(
    snapshot([
      card("w/later", { answer: { option: 0, answeredAt: 200 } }),
      card("w/earlier", { answer: { option: 1, answeredAt: 100 } }),
      card("w/d-ops", { kind: "decision", area: "ops", options: [] }),
      card("w/d-app-2", { kind: "decision", area: "app", order: 2 }),
      card("w/d-app-1", { kind: "decision", area: "app", order: 1 }),
      card("w/s-web", { kind: "status", area: "web", options: [] }),
      card("w/s-api", { kind: "status", area: "api", options: [] }),
    ]),
  );
  assert.deepEqual(ids(sections.answered), ["w/earlier", "w/later"]);
  assert.deepEqual(ids(sections.decided), [
    "w/d-app-1",
    "w/d-app-2",
    "w/d-ops",
  ]);
  assert.deepEqual(ids(sections.status), ["w/s-api", "w/s-web"]);
});

test("sections: every section is newest created first; a receipt by its question's", () => {
  const at = (hour) => `2026-10-12T${hour}:00:00Z`;
  const sections = benchSections(
    snapshot([
      card("w/old-critical", { severity: "critical", updatedAt: at("08") }),
      card("w/new-info", { severity: "info", updatedAt: at("11") }),
      card("w/garbage", { severity: "critical", updatedAt: "soon" }),
      card("w/asked-first", {
        updatedAt: at("08"),
        answer: { option: 0, answeredAt: 200 },
      }),
      card("w/asked-last", {
        updatedAt: at("11"),
        answer: { option: 0, answeredAt: 100 },
      }),
      card("w/d-app-old", {
        kind: "decision",
        area: "app",
        updatedAt: at("08"),
      }),
      card("w/d-ops-new", {
        kind: "decision",
        area: "ops",
        updatedAt: at("11"),
      }),
      // Its outcome is the newest drop; its card is the oldest question's.
      card("w/receipt", {
        kind: "status",
        options: [],
        state: "done",
        updatedAt: at("12"),
        request: card("w/receipt", { updatedAt: at("07") }),
      }),
    ]),
  );
  assert.deepEqual(ids(sections.needsYou), [
    "w/new-info",
    "w/old-critical",
    "w/garbage",
  ]);
  assert.deepEqual(ids(sections.answered), ["w/asked-last", "w/asked-first"]);
  assert.deepEqual(ids(sections.decided), [
    "w/d-ops-new",
    "w/d-app-old",
    "w/receipt",
  ]);
});

test("sections: no snapshot is four empty sections", () => {
  for (const value of [null, undefined]) {
    const sections = benchSections(value);
    assert.deepEqual(
      [sections.needsYou, sections.answered, sections.decided, sections.status],
      [[], [], [], []],
    );
    assert.equal(sections.retiring, 0);
  }
});

test("severity rank: unknown and missing severities sort last", () => {
  assert.deepEqual(
    ["critical", "attention", "info", "good", null, "toString"].map(
      benchSeverityRank,
    ),
    [0, 1, 2, 3, 4, 4],
  );
});

test("badge: null while disabled, else the Needs you count", () => {
  assert.equal(benchNeedsYouCount({ enabled: false }), null);
  assert.equal(benchNeedsYouCount(undefined), null);
  assert.equal(benchNeedsYouCount(snapshot([])), 0);
  assert.equal(
    benchNeedsYouCount(
      snapshot([
        card("w/a"),
        card("w/b", { severity: "critical" }),
        card("w/answered", { answer: { option: 0, answeredAt: 1 } }),
        card("w/stale", { validity: "superseded" }),
        card("w/status", { kind: "status", options: [] }),
      ]),
    ),
    2,
  );
});

test("last tick: RFC 3339 to unix seconds; empty or garbage is null", () => {
  assert.equal(benchLastTickSeconds("2026-10-12T10:00:00Z"), 1_791_799_200);
  assert.equal(benchLastTickSeconds(""), null);
  assert.equal(benchLastTickSeconds("not a time"), null);
});

test("sections: the owner sees cards for others apart; receipts are decided", () => {
  const craig = [
    { pk: "d".repeat(64), name: "Craig", role: "decider", agent: false },
  ];
  const items = [
    card("w/mine"),
    card("w/craig", { scoped: true, answerers: craig }),
    card("w/craig-answered", {
      scoped: true,
      answerers: craig,
      answer: {
        option: 0,
        answeredAt: 5,
        by: "d".repeat(64),
        byName: "Craig",
        byOwner: false,
        source: "tap",
      },
    }),
    card("w/receipt", {
      kind: "status",
      options: [],
      request: card("w/receipt"),
    }),
  ];
  for (const viewer of [undefined, "B".repeat(64)]) {
    const s = benchSections(snapshot(items), viewer);
    assert.deepEqual(ids(s.needsYou), ["w/mine"]);
    assert.deepEqual(ids(s.waitingOnOthers), ["w/craig"]);
    assert.deepEqual(ids(s.answered), ["w/craig-answered"]);
    assert.deepEqual(ids(s.decided), ["w/receipt"]);
    assert.deepEqual(ids(s.status), []);
  }
  const other = benchSections(snapshot(items), "c".repeat(64));
  assert.deepEqual(ids(other.needsYou), ["w/craig", "w/mine"]);
  assert.deepEqual(other.waitingOnOthers, []);
  assert.equal(benchNeedsYouCount(snapshot(items)), 1);
});

test("sections: blocked and inactive status items are held, receipts too", () => {
  const sections = benchSections(
    snapshot([
      card("w/blocked", { kind: "status", options: [], state: "blocked" }),
      card("w/paused", { kind: "status", options: [], state: "inactive" }),
      card("w/moving", { kind: "status", options: [], state: "in-progress" }),
      // A Hold answer's close-out: "PR #689 on hold".
      card("w/held-receipt", {
        kind: "status",
        options: [],
        state: "blocked",
        request: card("w/held-receipt"),
      }),
      card("w/done-receipt", {
        kind: "status",
        options: [],
        state: "done",
        request: card("w/done-receipt"),
      }),
    ]),
  );
  assert.deepEqual(ids(sections.held), [
    "w/blocked",
    "w/held-receipt",
    "w/paused",
  ]);
  assert.deepEqual(ids(sections.status), ["w/moving"]);
  assert.deepEqual(ids(sections.decided), ["w/done-receipt"]);
});
