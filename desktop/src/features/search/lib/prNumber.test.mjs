import assert from "node:assert/strict";
import test from "node:test";

import { parsePrQuery, prMentionScore } from "./prNumber.ts";

const list = (s) => s.split(" | ");

test("parsePrQuery: every PR form, with its repo; nothing else", () => {
  for (const q of list("#792 | PR 792 | pr792 | pr-792 | PR #792 | pull/792")) {
    assert.deepEqual(
      parsePrQuery(q),
      { number: "792", repo: null, explicit: true },
      q,
    );
  }
  assert.deepEqual(parsePrQuery(" 792 "), {
    number: "792",
    repo: null,
    explicit: false,
  });
  assert.equal(parsePrQuery("buzz#792")?.repo, "buzz");
  assert.equal(parsePrQuery("reset pr 792")?.repo, "reset");
  assert.equal(parsePrQuery("Metabolic-reset#792")?.repo, "reset");
  assert.deepEqual(parsePrQuery("talos/12"), {
    number: "12",
    repo: "talos",
    explicit: true,
  });
  const none =
    "792d3edd | 7928x | pr | # | buzz | buzz2 | 0.5.792 | 2026-10-14 | dec 14 | 12345678 | release 12 | reset 14 | buzz 2 | talos 12";
  for (const q of list(none)) assert.equal(parsePrQuery(q), null, q);
});

test("prMentionScore: PR forms 2, a whole number 1, never inside another", () => {
  const named = [
    "reset PR 792 waits for you",
    "see #792.",
    "PR #792 | pr-792 | orchestrator/pr-792",
    "https://github.com/QuicksilverSlick/Metabolic-reset/pull/792",
    "Ship pull request 792 of QuicksilverSlick/Metabolic-reset",
    "buzz:pr-792 merges only after a person launches it",
  ];
  for (const text of named.flatMap(list))
    assert.equal(prMentionScore(text, "792"), 2, text);
  for (const text of list("took 792 ms | hold-reset-792-b59153c6ba09")) {
    assert.equal(prMentionScore(text, "792"), 1, text);
  }
  const never = [
    "7928/7929 passed | 1792 | #1792 | #7928 | pr 7920",
    "commit 792d3edd | a792f | #792d3e | b59153c6ba09792e",
    "4b1e8f0c2d792a6e9f3b5c7d1a0e2f4b6c8d0e1f",
    "0.5.792 | v0.5.792 | 0.5.792x | 3,792 members | 792.5",
  ];
  for (const text of never.flatMap(list))
    assert.equal(prMentionScore(text, "792"), 0, text);
});
