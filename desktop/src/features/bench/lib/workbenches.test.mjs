import assert from "node:assert/strict";
import test from "node:test";

import { readFileSync } from "node:fs";

import { fallbackProjectColor } from "@/features/projects/lib/projectColors.ts";
import { benchCardPlace } from "./benchCardLayout.ts";
import {
  BUILT_IN_WORKBENCHES,
  INLINE_PROJECT_TABS,
  adoptBenchProjects,
  benchTabs,
  cardWorkbench,
  otherReason,
  slugWorkbench,
  splitBenchTabs,
  workbenchColor,
  workbenchCounts,
  workbenchSnapshot,
  workbenches,
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

const VIEWER = "a".repeat(64);
const STRANGER = "b".repeat(64);

function project(dtag, name, color, overrides = {}) {
  return {
    color,
    dtag,
    id: `30621:${VIEWER}:${dtag}`,
    legacy: false,
    name,
    owner: VIEWER,
    ...overrides,
  };
}

test("a workbench tab takes its project's color, announced projects first", () => {
  const [reset, dreamforge] = BUILT_IN_WORKBENCHES;
  const projects = [
    project("reset-notes", "reset-notes", undefined, { legacy: true }),
    project("garden", "Garden", "sky"),
    project("program", "28 Day Reset", "lime"),
  ];
  assert.equal(workbenchColor(reset, projects, VIEWER).id, "lime");
  assert.equal(
    workbenchColor(reset, projects.slice(0, 2), VIEWER),
    fallbackProjectColor("reset-notes"),
  );
  // Dreamforge with no project of its own: the fixed color of its slug.
  assert.equal(
    workbenchColor(dreamforge, projects, VIEWER),
    fallbackProjectColor("dreamforge"),
  );
  assert.equal(
    workbenchColor(dreamforge, [project("buzz", "buzz", "indigo")], VIEWER).id,
    "indigo",
  );
});

test("a newer or someone else's matching project never recolors a workbench", () => {
  const [reset] = BUILT_IN_WORKBENCHES;
  const program = project("28-day-reset", "28 Day Reset", "lime");
  const cohort = project("reset-q4-cohort", "Reset Q4 cohort", "sky");
  const theirs = project("28-day-reset", "28 Day Reset", "red", {
    id: `30621:${STRANGER}:28-day-reset`,
    owner: STRANGER,
  });
  // useProjectsQuery lists newest first; order must not matter.
  for (const projects of [
    [cohort, theirs, program],
    [theirs, program, cohort],
  ]) {
    assert.equal(workbenchColor(reset, projects, VIEWER).id, "lime");
  }
  assert.equal(
    workbenchColor(reset, [theirs], VIEWER),
    fallbackProjectColor("28-day-reset"),
  );
  assert.equal(
    workbenchColor(reset, [program], undefined),
    fallbackProjectColor("28-day-reset"),
  );
});

test("the viewer's agent's project colors a workbench the viewer has none for", () => {
  const [reset] = BUILT_IN_WORKBENCHES;
  const AGENT = "c".repeat(64);
  const agents = new Set([AGENT]);
  const agentReset = project("28-day-reset", "28 Day Reset", "orange", {
    id: `30621:${AGENT}:28-day-reset`,
    owner: AGENT.toUpperCase(),
  });
  const theirs = project("28-day-reset", "28 Day Reset", "red", {
    id: `30621:${STRANGER}:28-day-reset`,
    owner: STRANGER,
  });
  // The picker on the agent's project home recolors the tab and the chip.
  for (const projects of [
    [agentReset, theirs],
    [theirs, agentReset],
  ]) {
    assert.equal(workbenchColor(reset, projects, VIEWER, agents).id, "orange");
  }
  // Still never a stranger's, and the viewer's own comes first.
  assert.equal(
    workbenchColor(reset, [theirs], VIEWER, agents),
    fallbackProjectColor("28-day-reset"),
  );
  const own = project("reset", "Reset", "lime");
  assert.equal(
    workbenchColor(reset, [agentReset, own], VIEWER, agents).id,
    "lime",
  );
});

// The kit's seed and where its cards land (tests/projects.rs pins the
// same cards' headers on the Bench).
const VECTORS = JSON.parse(
  readFileSync(
    new URL(
      "../../../../src-tauri/src/bench/projects_vectors.json",
      import.meta.url,
    ),
    "utf8",
  ),
);
const SEEDED = { enabled: true, projects: VECTORS.registry.projects };

test("the kit's seed: Talos gets its own tab beside Dreamforge, old areas follow", (t) => {
  t.after(() => adoptBenchProjects(null));
  adoptBenchProjects(SEEDED);
  assert.deepEqual(
    benchTabs().map((tab) => [tab.id, tab.label, tab.parent]),
    [
      ["all", "All", undefined],
      ["dreamforge", "Dreamforge", undefined],
      ["talos", "Talos", "dreamforge"],
      ["reset", "28 Day Reset", undefined],
      ["other", "Other", undefined],
    ],
  );
  for (const [project, area, writer, tab, header] of VECTORS.cards) {
    assert.equal(cardWorkbench({ project, area, writer }), tab, area);
    assert.deepEqual(benchCardPlace({ project, area }), header, area);
  }
  // The built-in writers stay with their slug; a new project has none.
  assert.deepEqual(workbenches()[0].writers, BUILT_IN_WORKBENCHES[1].writers);
  assert.deepEqual(workbenches()[1].writers, []);
  // Unchanged, the same tabs; none, the built-in ones.
  const tabs = workbenches();
  adoptBenchProjects({ ...SEEDED, items: [] });
  assert.equal(workbenches(), tabs);
  adoptBenchProjects({ ...SEEDED, projects: null });
  assert.equal(workbenches(), BUILT_IN_WORKBENCHES);
  assert.equal(
    cardWorkbench({ project: null, area: "dreamforge-talos", writer: "x" }),
    "dreamforge",
  );
});

test("Other says why its cards are there, each name once, largest first", () => {
  const other = (area, project = null, validity = "current") => ({
    area,
    project,
    validity,
  });
  assert.equal(otherReason([other("gone", null, "stale")]), null);
  assert.match(
    otherReason([other("factory")]),
    /^1 card names no project: factory \(1\)\. /,
  );
  assert.match(
    otherReason([other("your-desktop"), other("fathom"), other("x", "fathom")]),
    /^3 cards name no project: fathom \(2\), your-desktop \(1\)\. /,
  );
});

test("past six project tabs the rest wait in More, those needing the owner first", () => {
  const ids = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
  const tabs = [
    { id: "all", label: "All" },
    ...ids.map((id) => ({ id, label: id })),
    { id: "other", label: "Other" },
  ];
  const counts = { g: { needsYou: 0 }, h: { needsYou: 0 }, i: { needsYou: 2 } };
  const shown = (open) => {
    const { inline, more } = splitBenchTabs(tabs, open, counts);
    return [inline.map((t) => t.id).join(""), more.map((t) => t.id).join("")];
  };
  assert.equal(INLINE_PROJECT_TABS, 6);
  assert.deepEqual(shown("all"), ["allabcdefother", "igh"]);
  // The open tab shows beside the six; an open tab among them adds none.
  assert.deepEqual(shown("h"), ["allabcdefhother", "ig"]);
  assert.deepEqual(shown("a"), ["allabcdefother", "igh"]);
  // Six or fewer: no More.
  assert.equal(splitBenchTabs(tabs.slice(0, 7), "all", null).more.length, 0);
});

test("a tab with no project of its own never borrows another tab's color", (t) => {
  t.after(() => adoptBenchProjects(null));
  adoptBenchProjects(SEEDED);
  const [dreamforge, talos, reset] = workbenches();
  // Talos's own slug color is lime, which Reset already shows.
  assert.equal(fallbackProjectColor("talos").id, "lime");
  const projects = [project("28-day-reset", "28 Day Reset", "lime")];
  assert.equal(workbenchColor(reset, projects, VIEWER).id, "lime");
  assert.equal(
    workbenchColor(dreamforge, projects, VIEWER),
    fallbackProjectColor("dreamforge"),
  );
  assert.equal(workbenchColor(talos, projects, VIEWER).id, "sky");
  // Without the clash, its own slug's color.
  assert.equal(workbenchColor(talos, [], VIEWER).id, "lime");
});
