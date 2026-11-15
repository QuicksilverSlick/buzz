import assert from "node:assert/strict";
import test from "node:test";

import {
  formatShipAllTime,
  readShipAllCheck,
  runShipAll,
  shipAllGate,
  shipAllPick,
  shipAllRecorded,
  shipAllTaps,
} from "./benchShipAll.ts";
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

test("a button taps the cards both lists name, in the check's order, while the check is fresh", () => {
  const now = Date.parse("2026-11-15T18:30:00.000Z");
  const a = hold(901, { updatedAt: "2026-11-15T18:00:00Z" });
  const b = hold(902, { updatedAt: "2026-11-15T17:00:00Z", sha: "c" });
  const both = [
    [a, true],
    [b, true],
  ];
  const gate = (check, items = [a, b]) =>
    shipAllGate(
      { items: [...items, ...(check ? [check] : [])] },
      "digest",
      now,
    );
  const open = gate(shipAllCheck(both, { at: now - 60_000 }));
  assert.equal(open.enabled, true);
  assert.deepEqual(ids(open.cards), [a.id, b.id], "the steward's order");
  assert.equal(
    gate(shipAllCheck(both, { at: now - 24 * 60_000 })).enabled,
    true,
    "one slow tick does not turn it off",
  );

  const reason = (check) => gate(check).reasons.join(" ");
  assert.match(
    reason(null),
    /check is not on the Bench yet.*own Ship it still works/,
  );
  assert.match(
    reason({ ...shipAllCheck(both), body: "x" }),
    /could not be read\. Each card's own Ship it still works\. .*see #steward/,
  );
  const stale = shipAllCheck(both, { at: now - 26 * 60_000 });
  assert.match(
    reason(stale),
    /last check \(.*\) is too old to trust\. .*tick may be failing/,
  );
  assert.deepEqual(
    ids(gate(stale).cards),
    [b.id, a.id],
    "what it would tap, oldest first",
  );
  assert.equal(
    gate(shipAllCheck(both, { at: now + 3 * 60_000 })).enabled,
    false,
  );

  // Answered by hand since the check: it just drops out.
  const answered = gate(shipAllCheck(both, { at: now - 60_000 }), [
    a,
    { ...b, answer: ANSWER },
  ]);
  assert.deepEqual([answered.enabled, ids(answered.cards)], [true, [a.id]]);
  // PR 902 moved to a new commit: the check has not seen its new card yet.
  const moved = gate(
    shipAllCheck([both[0], [hold(902, { sha: "d" }), true]], {
      at: now - 60_000,
    }),
  );
  assert.deepEqual([moved.enabled, ids(moved.cards)], [true, [a.id]]);
  assert.deepEqual(
    moved.left.map((l) => [l.card.id, l.reason]),
    [[b.id, "New since the steward's check at 6:29 PM."]].map(([id, r]) => [
      id,
      r.replace("6:29 PM", formatShipAllTime(now - 60_000)),
    ]),
  );
  assert.match(moved.recheck, /^It checks again by /);
});

test("a card the check holds is left out with its reason; with none clear the button says each", () => {
  const now = Date.now();
  const a = hold(901);
  const b = hold(902, { sha: "c" });
  const why = (n) =>
    `reset PR ${n} conflicts with main at bbbbbbbbbbbb (2 files): the fix is a new commit.`;
  const one = shipAllGate(
    { items: [a, b, shipAllCheck([[a, true]], { held: [[b, why(902)]] })] },
    "digest",
    now,
  );
  assert.deepEqual([one.enabled, ids(one.cards)], [true, [a.id]]);
  assert.deepEqual(
    one.left.map((l) => l.reason),
    [why(902)],
  );
  const none = shipAllGate(
    {
      items: [
        a,
        b,
        shipAllCheck([], {
          digest: "not clear",
          held: [
            [a, why(901)],
            [b, why(902)],
          ],
          whys: ["why digest: Every card for this button is held this tick."],
        }),
      ],
    },
    "digest",
    now,
  );
  assert.equal(none.enabled, false);
  assert.deepEqual(none.cards, []);
  assert.deepEqual(none.reasons.slice(0, 2), [why(901), why(902)]);
  assert.match(none.reasons[2], /^It checks again by /);
  // Too many to list: the kit names no card and says so.
  const over =
    "40 cards wait, more than one check can list: tap them on their cards until fewer wait.";
  const many = shipAllGate(
    {
      items: [
        a,
        shipAllCheck([], {
          digest: "not clear",
          whys: [`why digest: ${over}`],
        }),
      ],
    },
    "digest",
    now,
  );
  assert.deepEqual(
    many.left.map((l) => l.reason),
    [over],
  );
});

test("the digest button skips read-first cards; Give me details holds them", () => {
  const now = Date.now();
  const a = hold(901);
  const r = hold(902, { readFirst: true, sha: "c" });
  const items = [
    a,
    r,
    shipAllCheck([
      [r, false],
      [a, true],
    ]),
  ];
  assert.deepEqual(ids(shipAllGate({ items }, "digest", now).cards), [a.id]);
  const details = shipAllGate({ items }, "details", now);
  assert.equal(details.enabled, true);
  assert.deepEqual(ids(details.cards), [r.id, a.id]);
  const held = shipAllGate(
    { items: [r, shipAllCheck([[r, false]])] },
    "digest",
    now,
  );
  assert.match(
    held.reasons[0],
    /Every waiting card is read first \(lane:hold, money, sign-in or database\)/,
  );
  const none = shipAllGate({ items: [] }, "details", now);
  assert.match(none.reasons[0], /Nothing for this button waits/);
  // lane:hold added after its card: the title still says tap, the check says details only.
  const late = shipAllGate(
    { items: [a, shipAllCheck([[a, false]])] },
    "digest",
    now,
  );
  assert.equal(late.enabled, false);
  assert.deepEqual(
    late.left.map((l) => [l.readFirst, l.reason]),
    [
      [
        true,
        "It is read first now (lane:hold was added after its card): ship it from Give me details.",
      ],
    ],
  );
});

test("the run taps in order, waits for each Recorded line, stops at a failure", async () => {
  const cards = [hold(1), stack([2, 3]), hold(4, { sha: "d" })];
  const taps = shipAllTaps(cards.map((card) => ({ card })));
  assert.deepEqual(taps[0], {
    itemId: cards[0].id,
    cardEventId: cards[0].cardEventId,
    option: 0,
    title: cards[0].title,
  });
  const log = [];
  let answered = null;
  const deps = (fail) => ({
    answer: async (t) => {
      log.push(`tap ${t.itemId.split("-")[2]}`);
      if (t.itemId === fail) throw new Error("This card changed - look again");
    },
    stillOk: async (t) => t.itemId !== answered,
    // Recorded on the second read of each card.
    isRecorded: async (t) => {
      const read = `read ${t.itemId.split("-")[2]}`;
      log.push(read);
      return log.filter((line) => line === read).length > 1;
    },
    onStep: () => {},
    sleep: async () => {},
  });
  assert.deepEqual(await runShipAll(taps, deps()), {
    recorded: 3,
    error: null,
  });
  assert.deepEqual(
    log.join(),
    "tap 1,read 1,read 1,tap 2,read 2,read 2,tap 4,read 4,read 4",
  );

  log.length = 0;
  const failed = await runShipAll(taps, deps(cards[1].id));
  assert.equal(failed.recorded, 1);
  assert.match(failed.error, /couldn't save: This card changed/);
  assert.deepEqual(log.join(), "tap 1,read 1,read 1,tap 2", "nothing after it");

  const never = { ...deps(), isRecorded: async () => false, timeoutMs: 3_000 };
  const timedOut = await runShipAll(taps, never);
  assert.equal(timedOut.recorded, 0);
  assert.match(
    timedOut.error,
    /its Ship it was sent, but the Bench has not recorded it after 3 s\. It may still record: check its card/,
  );

  // He answered the second card meanwhile: it is never tapped, and nothing after it.
  log.length = 0;
  answered = cards[1].id;
  const stopped = await runShipAll(taps, deps());
  assert.equal(stopped.recorded, 1);
  assert.match(
    stopped.error,
    /: not tapped\. You answered or wrote on it meanwhile/,
  );
  assert.deepEqual(log.join(), "tap 1,read 1,read 1");
});

test("Recorded means the owner's Ship it on the card he was shown", () => {
  const card = hold(1);
  const [tap] = shipAllTaps([{ card }]);
  const shows = (over) =>
    shipAllRecorded({ items: [{ ...card, ...over }] }, tap);
  assert.equal(shows({}), false);
  assert.equal(shows({ recorded: 0, answer: ANSWER }), true);
  assert.equal(
    shows({ recorded: 0, answer: { ...ANSWER, byOwner: false } }),
    false,
  );
  assert.equal(shows({ recorded: 0, cardEventId: "f".repeat(64) }), false);
  assert.equal(shipAllRecorded(null, tap), false);
});
