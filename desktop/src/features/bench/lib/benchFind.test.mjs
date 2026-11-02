import assert from "node:assert/strict";
import test from "node:test";

import { benchCardTier, benchNeedsYou, findBenchCards } from "./benchFind.ts";

const HOLD_792 = "steward/hold-reset-792-b59153c6ba09-5dd0d86acaa3";
const HOLD_94 = "steward/hold-buzz-94-287009e1603b-49fbb657c0aa";
const HOLD_12 = "steward/hold-talos-12-0a1b2c3d4e5f-6a7b8c9d0e1f";
const LAUNCH = "steward/buzz-launch-needed-287009e1603b";

// Only what findBenchCards and the Bench search read.
const card = (id, more = {}) => ({
  id,
  kind: "status",
  title: "",
  body: "",
  area: "buzz",
  options: [],
  links: [],
  answerers: [],
  validity: "current",
  updatedAt: "2026-10-30T10:00:00Z",
  answer: null,
  request: null,
  ...more,
});
const hold = (id, pr, repo, more = {}) =>
  card(id, {
    kind: "pending",
    title: `${id.split("-")[1]} PR ${pr} waits for you: Fix bug`,
    options: ["Ship it", "Send it back", "Keep holding"],
    links: [
      {
        label: `Pull request ${pr}`,
        url: `https://github.com/o/${repo}/pull/${pr}`,
      },
    ],
    ...more,
  });

const snapshot = {
  items: [
    card(LAUNCH, {
      title: "buzz: Launch check needed for 287009e1603b",
      body: "buzz:pr-94 merges only after a person launches it.",
      updatedAt: "2026-10-30T12:00:00Z",
    }),
    hold(HOLD_94, "94", "buzz", { body: "commit 792d3edd, 7928 tests" }),
    hold(HOLD_792.replace("b591", "0000"), "792", "Metabolic-reset", {
      validity: "superseded",
    }),
    card("steward/buzz-train", {
      kind: "decision",
      body: "Pull requests (2):\n- 94 Fix x (green)\n- 76 Gate (red)",
      updatedAt: "2026-10-30T11:00:00Z",
    }),
    card("orchestrator/pr-719", {
      title: "PR #719 merged",
      area: "28-day-reset",
    }),
    hold(HOLD_12, "12", "dreamforge-talos"),
    card("steward/buzz-not-live-pr-5", { title: "See PR 12 first" }),
    hold(HOLD_792, "792", "Metabolic-reset", { area: "28-day-reset" }),
  ],
};
const ids = (query) => findBenchCards(snapshot, query).map((c) => c.id);

test("a PR number finds its card, never a digit decoy or a stale copy", () => {
  for (const q of ["#792", "PR 792", "pr792", "pr-792", "792"]) {
    assert.deepEqual(ids(q), [HOLD_792], q);
  }
  assert.deepEqual(ids("pr719"), ["orchestrator/pr-719"]);
});

test("a repo narrows PR numbers that repeat across repos", () => {
  // The hold that needs you, then the newer launch card, then the receipt.
  assert.deepEqual(ids("buzz#94"), [HOLD_94, LAUNCH, "steward/buzz-train"]);
  assert.deepEqual(ids("reset#94"), []);
  assert.deepEqual(ids("talos#12"), [HOLD_12]);
  // A space between is words, which still name the talos card.
  assert.deepEqual(ids("talos 12"), [HOLD_12]);
  assert.deepEqual(ids("#12"), [HOLD_12, "steward/buzz-not-live-pr-5"]);
});

test("words find cards by type, title and repo; holds are red", () => {
  assert.deepEqual(ids("launch check"), [LAUNCH]);
  assert.deepEqual(ids("receipt"), ["steward/buzz-train"]);
  assert.deepEqual(ids("metabolic-reset"), [HOLD_792]);
  assert.deepEqual(findBenchCards(null, "#792"), []);
  // Words never match a hold's options or body: every hold has "Send it back".
  for (const q of ["send", "it", "tests", "--"])
    assert.deepEqual(ids(q), [], q);
  const [hold94, launch] = findBenchCards(snapshot, "buzz#94");
  assert.equal(benchCardTier(hold94), "red");
  assert.equal(benchCardTier(launch), null);
  assert.equal(benchNeedsYou(hold94), true);
  const craig = { pk: "c".repeat(64), name: "Craig" };
  assert.equal(benchNeedsYou({ ...hold94, answerers: [craig] }), false);
});
