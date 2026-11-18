import assert from "node:assert/strict";
import test from "node:test";

import { readShipAllCheck, shipAllPick } from "./benchShipAll.ts";
import {
  ANSWER,
  REPLY,
  RESET,
  hold,
  shipAllCheck,
  stack,
  stewardItem,
} from "./benchShipAll.test-support.mjs";

const ids = (cards) => cards.map((c) => c.card.id);
const pr = (card) => card.id.split("-")[2];

test("candidates follow the kit's rules; the rest are left out with why", () => {
  const head = (c) =>
    `${c.repeat(40)} on top of main at commit ${"b".repeat(40)}`;
  const items = [
    hold(901),
    hold(902, { reply: REPLY }),
    hold(903, { cardEventId: null, waitsFor: "slot" }),
    hold(904, { answer: ANSWER }),
    hold(905, { options: ["Merge it", "Hold"] }),
    hold(906, {
      action: `Ship pull request 906 of ${RESET} at commit ${head("f")}`,
    }),
    hold(907, {
      action: `Ship pull request 907 of QuicksilverSlick/buzz at commit ${head("a")}`,
    }),
    hold(908, { writer: "claude" }),
    hold(909, { validity: "superseded" }),
    stack([3, 4]),
    stack([3, 4], { digest: "d".repeat(64) }), // a second card for PR 3
    stewardItem("steward/stop-reset-1", { options: ["Stop this line"] }),
  ];
  const { candidates, left } = shipAllPick({ items });
  assert.deepEqual(ids(candidates), [items[0].id]);
  assert.deepEqual(
    left.map((l) => [pr(l.card), l.reason.split(/[:(]/)[0]]),
    [
      ["902", "You wrote on its card"],
      ["903", "Not on the Bench yet "],
      ["905", "Its card does not name the commits Ship all checks"],
      ["906", "Its card does not name the commits Ship all checks"],
      ["907", "Its card does not name the commits Ship all checks"],
      ["3", "Two open cards for reset pull request 3"],
      ["3", "Two open cards for reset pull request 3"],
    ],
  );
});

test("read first is any title without K1's anchored marker", () => {
  const titles = [
    "reset PR 1 waits for your tap: A",
    "reset PR 1 waits for you, read first: A",
    "reset PR 1 waits for you: a card from before K1",
    "reset PR 1 waits for you, read first: x waits for your tap: y",
    "Fix: reset PR 1 waits for your tap: hostile",
  ];
  const read = titles.map(
    (title) =>
      shipAllPick({ items: [hold(1, { title })] }).candidates[0].readFirst,
  );
  assert.deepEqual(read, [false, true, true, true, true]);
});

test("the check parses line by line; any other line is unreadable", () => {
  const a = hold(901);
  const b = hold(902, { sha: "c" });
  const item = shipAllCheck([[a, true]], {
    held: [[b, "reset PR 902 has red CI."]],
    whys: ["why details: x"],
  });
  const parsed = readShipAllCheck({ items: [item] });
  assert.deepEqual(parsed.cards, [{ id: a.id, digest: true }]);
  assert.deepEqual([...parsed.held], [[b.id, "reset PR 902 has red CI."]]);
  assert.deepEqual(parsed.whys, { digest: [], details: ["x"] });
  const read = (body) => readShipAllCheck({ items: [{ ...item, body }] });
  assert.equal(read(item.body.replace("v1", "v2")), "unreadable");
  assert.equal(read(`${item.body}\nship: all`), "unreadable");
  assert.equal(read(`${item.body}\ncard: ${a.id} details`), "unreadable");
  assert.equal(read(`${item.body}\nwhy digest: late`), "unreadable");
  assert.equal(read(`${item.body}\nheld: ${b.id} late`), "unreadable");
  const twice = shipAllCheck([[a, true]], { held: [[a, "x"]] });
  assert.equal(read(twice.body), "unreadable", "clear and held at once");
  assert.equal(readShipAllCheck({ items: [] }), "missing");
  const foreign = { ...item, writer: "claude" };
  assert.equal(readShipAllCheck({ items: [foreign] }), "missing");
});
