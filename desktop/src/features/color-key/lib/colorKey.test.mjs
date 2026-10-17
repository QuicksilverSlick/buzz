/**
 * The color key's data: types straight from INBOX_TYPE_META, the sidebar's
 * projects in the sidebar's order with their colors, agents reduced to their
 * public face, and only Bench labels the Bench code really prints.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  benchKindLabel,
  benchRiskChip,
  benchSeverityChip,
  benchValidityLabel,
} from "@/features/bench/lib/benchCards.ts";
import { INBOX_TYPE_META } from "@/features/home/ui/InboxItemPreview.tsx";
import {
  fallbackProjectColor,
  projectColorById,
} from "@/features/projects/lib/projectColors.ts";
import {
  BENCH_KEY_LABELS,
  COLOR_KEY_TYPES,
  NO_AGENT_DESCRIPTION,
  colorKeyAgents,
  colorKeyProjects,
} from "./colorKey.ts";

test("item types are INBOX_TYPE_META's, in its order, each with a meaning", () => {
  assert.deepEqual(
    COLOR_KEY_TYPES.map((row) => row.type),
    Object.keys(INBOX_TYPE_META),
  );
  for (const row of COLOR_KEY_TYPES) {
    assert.equal(row.stripe, INBOX_TYPE_META[row.type].stripe);
    assert.ok(row.meaning.endsWith("."), `${row.type} has a one-line meaning`);
  }
});

const ME = "a".repeat(64);
const project = (dtag, name, extra = {}) => ({
  id: `p:${dtag}`,
  dtag,
  name,
  owner: "b".repeat(64),
  createdAt: 1,
  projectAddress: `30090:${"b".repeat(64)}:${dtag}`,
  color: null,
  ...extra,
});

test("projects: the sidebar's set and order, each with its color", () => {
  const projects = [
    project("zeta", "Zeta", { color: "pink", createdAt: 3 }),
    project("hidden", "Hidden", { createdAt: 4 }),
    project("alpha", "Alpha", { createdAt: 2 }),
    project("mine", "Mine", { owner: ME, createdAt: 1 }),
  ];
  const added = new Set(
    ["zeta", "alpha"].map((dtag) => `30090:${"b".repeat(64)}:${dtag}`),
  );
  const byName = colorKeyProjects({
    addedProjectAddresses: added,
    currentPubkey: ME,
    filter: "added",
    projects,
    sort: "name",
  });
  assert.deepEqual(
    byName.map((row) => row.name),
    ["Alpha", "Zeta"],
  );
  assert.equal(byName[1].color, projectColorById("pink"));
  assert.equal(byName[0].color, fallbackProjectColor("alpha"));
  assert.equal(byName[0].address, projects[2].projectAddress);

  const newestFirst = colorKeyProjects({
    addedProjectAddresses: added,
    currentPubkey: ME,
    filter: "added",
    projects,
    sort: "created",
  });
  assert.deepEqual(
    newestFirst.map((row) => row.name),
    ["Zeta", "Alpha"],
  );
  const owned = colorKeyProjects({
    addedProjectAddresses: added,
    currentPubkey: ME,
    filter: "owned",
    projects,
    sort: "name",
  });
  assert.deepEqual(
    owned.map((row) => row.name),
    ["Mine"],
  );
});

test("agents: name, avatar and About only; a missing About says so", () => {
  const agent = (pubkey, name) => ({
    pubkey,
    name,
    avatarUrl: null,
    systemPrompt: "SECRET PROMPT",
    envVars: { TOKEN: "SECRET" },
    acpCommand: "secret-acp",
    agentArgs: ["--secret"],
    status: "running",
  });
  const abouts = new Map([
    ["1", "  Builds the app.  "],
    ["2", "   "],
    ["3", null],
  ]);
  const rows = colorKeyAgents(
    [agent("1", "Reset Worker"), agent("2", "Fixer"), agent("3", "Scout")],
    (pubkey) => abouts.get(pubkey),
  );
  assert.deepEqual(
    rows.map((row) => row.job),
    ["Builds the app.", NO_AGENT_DESCRIPTION, NO_AGENT_DESCRIPTION],
  );
  assert.equal(NO_AGENT_DESCRIPTION, "No description yet");
  for (const row of rows) {
    assert.deepEqual(Object.keys(row).sort(), [
      "avatarUrl",
      "job",
      "name",
      "pubkey",
    ]);
    assert.ok(!JSON.stringify(row).includes("SECRET"));
  }
});

test("Bench labels are the ones the Bench prints, in the look it prints them", () => {
  const chips = new Map(
    BENCH_KEY_LABELS.flatMap((row) => row.chips).map((chip) => [
      chip.label,
      chip.variant,
    ]),
  );
  for (const severity of ["critical", "attention", "info", "good"]) {
    const { label, variant } = benchSeverityChip(severity);
    assert.equal(chips.get(label), variant, label);
  }
  for (const risk of ["high", "medium", "low"]) {
    const { label, variant } = benchRiskChip(risk);
    assert.equal(chips.get(label), variant, label);
  }
  for (const kind of ["pending", "decision", "status"]) {
    assert.equal(chips.get(benchKindLabel(kind)), null, kind);
  }
  for (const validity of ["suspected-stale", "superseded"]) {
    assert.equal(chips.get(benchValidityLabel(validity)), "outline");
  }
  // Every other label's words are in the Bench's own code.
  const source = [
    "../../bench/lib/benchCards.ts",
    "../../bench/ui/BenchCard.tsx",
  ]
    .map((path) => readFileSync(new URL(path, import.meta.url), "utf8"))
    .join("\n");
  for (const label of chips.keys()) {
    for (const part of label.split(/[…·]/)) {
      if (part.trim()) assert.ok(source.includes(part.trim()), part);
    }
  }
});
