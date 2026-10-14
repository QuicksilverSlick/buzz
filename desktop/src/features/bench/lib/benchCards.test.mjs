import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  BENCH_SAVE_TIMEOUT_MESSAGE,
  INITIAL_BENCH_ANSWER_STATE,
  answerTarget,
  benchAnswerReducer,
  benchCardMode,
  benchDeadlineSeconds,
  benchEligible,
  benchForLabel,
  benchOptionViews,
  benchReceipt,
  benchRecordedLine,
  benchRowState,
  benchSeverityChip,
  benchStateLabel,
  benchValidityLabel,
  createPollGroup,
  indexBenchCards,
  isBenchAnswerKeycap,
  isBenchCandidate,
  safeBenchLinks,
} from "./benchCards.ts";

const CHANNEL = "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10";
const OTHER_CHANNEL = "1d2c3b4a-0000-4000-8000-000000000000";
const WRITER = "a".repeat(64);
const OWNER = "b".repeat(64);
const STRANGER = "c".repeat(64);
const CARD_ID = "e".repeat(64);

function card(overrides = {}) {
  return {
    id: "dreamforge/ship-it",
    writer: "dreamforge",
    kind: "pending",
    title: "Ship the release?",
    body: "Two options.",
    area: "release",
    severity: "attention",
    options: ["Ship it", "Hold"],
    state: null,
    decidedBy: null,
    deadline: null,
    links: [],
    order: 0,
    validity: "current",
    cardEventId: CARD_ID,
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

function snapshot(items = [card()], overrides = {}) {
  return {
    enabled: true,
    channelId: CHANNEL,
    writerPubkey: WRITER,
    ownerPubkey: OWNER,
    lastError: null,
    lastTick: "2026-10-12T10:00:00Z",
    items,
    ...overrides,
  };
}

function row(overrides = {}) {
  return {
    channelId: CHANNEL,
    id: CARD_ID,
    kind: 9,
    signerPubkey: WRITER,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// The card selection guard
// ---------------------------------------------------------------------------

test("guard: a live card in #bench signed by the writer is a Bench card", () => {
  const cards = snapshot();
  const state = benchRowState(cards, row());
  assert.equal(state.inBenchChannel, true);
  assert.equal(state.match?.card, cards.items[0]);
  assert.equal(state.match?.ownerPubkey, OWNER);
});

test("guard: the signer comparison ignores hex case", () => {
  const state = benchRowState(
    snapshot(),
    row({ signerPubkey: WRITER.toUpperCase() }),
  );
  assert.notEqual(state.match, null);
});

test("guard: any other signer renders as an ordinary message", () => {
  for (const signerPubkey of [OWNER, STRANGER, undefined, ""]) {
    const state = benchRowState(snapshot(), row({ signerPubkey }));
    assert.equal(state.match, null, `signer ${signerPubkey}`);
    assert.equal(state.inBenchChannel, true);
  }
});

test("guard: the same event id in another channel is not a Bench card", () => {
  const state = benchRowState(snapshot(), row({ channelId: OTHER_CHANNEL }));
  assert.deepEqual(state, { inBenchChannel: false, match: null });
  assert.equal(benchRowState(snapshot(), row({ channelId: null })).match, null);
});

test("guard: a message id that is no live card renders as before", () => {
  const state = benchRowState(snapshot(), row({ id: "f".repeat(64) }));
  assert.equal(state.match, null);
  assert.equal(state.inBenchChannel, true, "still polls inside #bench");
});

test("guard: an item without a live card claims no message", () => {
  const cards = snapshot([card({ cardEventId: null })]);
  assert.equal(benchRowState(cards, row()).match, null);
});

test("guard: a disabled, missing or not-yet-ticked Bench matches nothing", () => {
  for (const cards of [
    { enabled: false },
    undefined,
    null,
    snapshot(undefined, { channelId: "" }),
  ]) {
    assert.deepEqual(
      benchRowState(cards, row({ channelId: cards?.channelId ?? CHANNEL })),
      { inBenchChannel: false, match: null },
    );
  }
});

test("guard: an empty writer key matches no signer", () => {
  const cards = snapshot(undefined, { writerPubkey: "" });
  assert.equal(benchRowState(cards, row({ signerPubkey: "" })).match, null);
});

test("guard: other kinds and optimistic sends never become cards", () => {
  assert.equal(benchRowState(snapshot(), row({ kind: 40008 })).match, null);
  assert.equal(benchRowState(snapshot(), row({ pending: true })).match, null);
  assert.notEqual(
    benchRowState(snapshot(), row({ kind: undefined })).match,
    null,
  );
  assert.equal(isBenchCandidate(row({ kind: 7 })), false);
  assert.equal(isBenchCandidate(row()), true);
});

test("index: live cards by event id, cached per snapshot", () => {
  const cards = snapshot([
    card(),
    card({ id: "w/two", cardEventId: "1".repeat(64) }),
    card({ id: "w/none", cardEventId: null }),
  ]);
  const index = indexBenchCards(cards);
  assert.deepEqual([...index.keys()], [CARD_ID, "1".repeat(64)]);
  assert.equal(indexBenchCards(cards), index, "same snapshot, same Map");
  assert.equal(indexBenchCards(null).size, 0);
});

test("index: an event id claimed by two items maps to neither", () => {
  const cards = snapshot([card(), card({ id: "w/dupe" })]);
  assert.equal(indexBenchCards(cards).has(CARD_ID), false);
  assert.equal(benchRowState(cards, row()).match, null);
});

// ---------------------------------------------------------------------------
// Interactivity
// ---------------------------------------------------------------------------

test("mode: a current pending card answers for the owner", () => {
  assert.deepEqual(benchCardMode(card(), OWNER, OWNER), {
    interactive: true,
    readOnlyReason: null,
  });
  assert.equal(
    benchCardMode(card(), OWNER, OWNER.toUpperCase()).interactive,
    true,
  );
  assert.equal(
    benchCardMode(card(), OWNER, undefined).interactive,
    true,
    "an unknown viewer is left to bench_answer's own owner check",
  );
});

test("mode: read-only for another viewer, no owner, stale or superseded", () => {
  const cases = [
    [card(), OWNER, STRANGER, /Only the Bench owner/],
    [card(), null, OWNER, /no owner/],
    [card({ validity: "superseded" }), OWNER, OWNER, /Superseded/],
    [card({ validity: "suspected-stale" }), OWNER, OWNER, /May be stale/],
    [
      card({ cardEventId: null, answer: { option: 0, answeredAt: 1 } }),
      OWNER,
      OWNER,
      /no longer live/,
    ],
    [card({ cardEventId: null }), OWNER, OWNER, /Waiting for the Bench/],
  ];
  for (const [c, owner, viewer, reason] of cases) {
    const mode = benchCardMode(c, owner, viewer);
    assert.equal(mode.interactive, false);
    assert.match(mode.readOnlyReason ?? "", reason);
  }
});

test("mode: decision and status cards are never interactive", () => {
  for (const kind of ["decision", "status"]) {
    assert.deepEqual(benchCardMode(card({ kind }), OWNER, OWNER), {
      interactive: false,
      readOnlyReason: null,
    });
  }
});

// ---------------------------------------------------------------------------
// Chips and labels
// ---------------------------------------------------------------------------

test("severity: each level has a readable label and a theme badge variant", () => {
  assert.deepEqual(benchSeverityChip("critical"), {
    label: "Your decision",
    variant: "destructive",
  });
  assert.deepEqual(benchSeverityChip("attention"), {
    label: "Attention",
    variant: "warning",
  });
  assert.deepEqual(benchSeverityChip("info"), {
    label: "Info",
    variant: "info",
  });
  assert.deepEqual(benchSeverityChip("good"), {
    label: "Good",
    variant: "success",
  });
});

test("severity: none, unknown and prototype keys give no chip", () => {
  for (const value of [
    null,
    undefined,
    "",
    "urgent",
    "toString",
    "__proto__",
  ]) {
    assert.equal(benchSeverityChip(value), null, String(value));
  }
});

test("labels: status states and validity", () => {
  assert.equal(benchStateLabel("in-progress"), "In progress");
  assert.equal(benchStateLabel("not-started"), "Not started");
  assert.equal(benchStateLabel(null), null);
  assert.equal(benchStateLabel("constructor"), null);
  assert.equal(benchValidityLabel("current"), null);
  assert.equal(benchValidityLabel("superseded"), "Superseded");
  assert.equal(benchValidityLabel("suspected-stale"), "May be stale");
});

// ---------------------------------------------------------------------------
// Link filter
// ---------------------------------------------------------------------------

test("links: https and buzz:// survive with their scheme", () => {
  const message = `buzz://message?channel=${CHANNEL}&id=${CARD_ID}`;
  assert.deepEqual(
    safeBenchLinks([
      { label: "PR 42", url: "https://github.com/block/buzz/pull/42" },
      { label: "Card", url: message },
    ]),
    [
      {
        label: "PR 42",
        url: "https://github.com/block/buzz/pull/42",
        scheme: "https",
      },
      { label: "Card", url: message, scheme: "buzz" },
    ],
  );
});

test("links: every other scheme, credentials and odd bytes are dropped", () => {
  const dropped = [
    "javascript:alert(1)",
    "JavaScript:alert(1)",
    "data:text/html,<b>x</b>",
    "http://github.com/x",
    "file:///C:/Windows/win.ini",
    "vbscript:msgbox",
    "https://user:pass@github.com/x",
    "https://user@github.com/x",
    "https:github.com/x",
    "https:///nohost",
    "https://github.com/a b",
    "https://github.com/\tx",
    "https://github.com\\@evil.example/x",
    "https://gith\u00fcb.com/x",
    " https://github.com/x",
    "buzz:message?channel=x",
    "//github.com/x",
    "",
  ];
  for (const url of dropped) {
    assert.deepEqual(safeBenchLinks([{ label: "x", url }]), [], url);
  }
});

test("links: blank labels and malformed entries are dropped", () => {
  assert.deepEqual(
    safeBenchLinks([
      { label: "  ", url: "https://github.com/x" },
      { label: "ok", url: 42 },
      null,
      { url: "https://github.com/x" },
    ]),
    [],
  );
  assert.deepEqual(safeBenchLinks(undefined), []);
});

// ---------------------------------------------------------------------------
// Pending / recorded reducer
// ---------------------------------------------------------------------------

function click(state, option, recorded, byOwner = true) {
  return benchAnswerReducer(state, {
    type: "click",
    option,
    recorded,
    byOwner,
  });
}

function record(state, recorded, byOwner = true) {
  return benchAnswerReducer(state, { type: "recorded", recorded, byOwner });
}

test("answerTarget: clicking the recorded option undoes it", () => {
  assert.equal(answerTarget(1, null, true), 1);
  assert.equal(answerTarget(1, 0, true), 1);
  assert.equal(answerTarget(1, 1, true), null);
  assert.equal(
    answerTarget(1, 1, false),
    1,
    "someone else's answer is overridden, not undone",
  );
});

test("reducer: a click saves until the snapshot records the target", () => {
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 1, null);
  assert.deepEqual(saving, {
    attempt: 1,
    phase: "saving",
    clicked: 1,
    target: 1,
    error: null,
  });
  assert.equal(
    record(saving, null),
    saving,
    "an unchanged snapshot keeps it saving",
  );
  assert.equal(
    record(saving, 0),
    saving,
    "another option recorded (a newer phone tap) is not this click",
  );
  assert.deepEqual(record(saving, 1), {
    ...INITIAL_BENCH_ANSWER_STATE,
    attempt: 1,
  });
});

test("reducer: undo waits for the card to show no answer", () => {
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 0, 0);
  assert.equal(saving.target, null);
  assert.equal(saving.clicked, 0);
  assert.equal(record(saving, null).phase, "idle");
});

test("reducer: clicks while saving are ignored", () => {
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 1, null);
  assert.equal(click(saving, 0, null), saving);
});

test("reducer: a click right after the record lands starts a new attempt", () => {
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 1, null);
  // The stored state still says saving, but the snapshot already shows 1.
  const next = click(saving, 1, 1);
  assert.equal(next.attempt, 2);
  assert.equal(next.target, null, "the second click undoes the first");
});

test("reducer: command errors and the 15 s timeout fail only their attempt", () => {
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 1, null);
  assert.equal(
    benchAnswerReducer(saving, { type: "timedOut", attempt: 0 }),
    saving,
    "a stale timer is ignored",
  );
  const timedOut = benchAnswerReducer(saving, { type: "timedOut", attempt: 1 });
  assert.equal(timedOut.phase, "failed");
  assert.equal(timedOut.error, BENCH_SAVE_TIMEOUT_MESSAGE);

  const failed = benchAnswerReducer(saving, {
    type: "commandFailed",
    attempt: 1,
    message: "owner identity changed; refusing to answer",
  });
  assert.equal(failed.phase, "failed");
  assert.equal(
    failed.error,
    "Couldn't save: owner identity changed; refusing to answer",
  );
  assert.equal(
    benchAnswerReducer(failed, { type: "timedOut", attempt: 1 }),
    failed,
    "a failed attempt does not time out again",
  );
});

test("reducer: a late record clears the error; a retry starts fresh", () => {
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 1, null);
  const failed = benchAnswerReducer(saving, { type: "timedOut", attempt: 1 });
  assert.equal(record(failed, 1).phase, "idle");
  const retry = click(failed, 0, null);
  assert.equal(retry.phase, "saving");
  assert.equal(retry.attempt, 2);
  assert.equal(retry.error, null);
});

// ---------------------------------------------------------------------------
// Option buttons and the Recorded line
// ---------------------------------------------------------------------------

test("options: the recorded option is selected; the clicked one is pending", () => {
  const recorded = card({ recorded: 0, answer: { option: 0, answeredAt: 1 } });
  const idle = benchOptionViews(recorded, INITIAL_BENCH_ANSWER_STATE, true);
  assert.deepEqual(
    idle.map((o) => [o.number, o.text, o.selected, o.pending, o.disabled]),
    [
      ["1", "Ship it", true, false, false],
      ["2", "Hold", false, false, false],
    ],
  );
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 1, 0);
  const busy = benchOptionViews(recorded, saving, true);
  assert.deepEqual(
    busy.map((o) => [o.selected, o.pending, o.disabled]),
    [
      [true, false, true],
      [false, true, false],
    ],
  );
});

test("options: read-only cards disable every button", () => {
  const views = benchOptionViews(card(), INITIAL_BENCH_ANSWER_STATE, false);
  assert.ok(views.every((o) => o.disabled));
});

test("recorded line: option number, text and the answer time", () => {
  const formatTime = (s) => `t${s}`;
  assert.equal(benchRecordedLine(card(), formatTime), null);
  assert.equal(
    benchRecordedLine(
      card({ recorded: 1, answer: { option: 1, answeredAt: 99 } }),
      formatTime,
    ),
    "Recorded: 2. Hold \u00b7 t99",
  );
  assert.equal(
    benchRecordedLine(
      card({ recorded: 1, answer: { option: 0, answeredAt: 99 } }),
      formatTime,
    ),
    "Recorded: 2. Hold",
    "no time when the answer has already moved on",
  );
  assert.equal(benchRecordedLine(card({ recorded: 5 }), formatTime), null);
});

test("retired card: the Bench's answer still shows, read-only", () => {
  // state.json 24 h after the answer: the card is retired, `recorded` is
  // forgotten and `answer` stays.
  const retired = card({
    cardEventId: null,
    answer: { option: 1, answeredAt: 99 },
    recorded: null,
  });
  assert.equal(
    benchRecordedLine(retired, (s) => `t${s}`),
    "Recorded: 2. Hold · t99",
  );
  const mode = benchCardMode(retired, OWNER, OWNER);
  assert.equal(mode.interactive, false);
  assert.deepEqual(
    benchOptionViews(retired, INITIAL_BENCH_ANSWER_STATE, mode.interactive).map(
      (o) => [o.selected, o.disabled],
    ),
    [
      [false, true],
      [true, true],
    ],
  );
  // A live card shows only what its text records, not an answer the Bench
  // has yet to edit in.
  const live = card({ answer: { option: 1, answeredAt: 99 }, recorded: null });
  assert.equal(
    benchRecordedLine(live, (s) => `t${s}`),
    null,
  );
  assert.ok(
    benchOptionViews(live, INITIAL_BENCH_ANSWER_STATE, true).every(
      (o) => !o.selected,
    ),
  );
});

test("keycaps: only the card's own answer keycaps, as the Bench reads them", () => {
  const two = card();
  assert.equal(isBenchAnswerKeycap(two, "1️⃣"), true);
  assert.equal(isBenchAnswerKeycap(two, "2️⃣"), true);
  assert.equal(isBenchAnswerKeycap(two, "3️⃣"), false, "no option 3");
  assert.equal(isBenchAnswerKeycap(two, "1⃣"), false, "not the seeded form");
  assert.equal(isBenchAnswerKeycap(two, "\u{1F44D}"), false);
  const five = card({ options: ["a", "b", "c", "d", "e"] });
  assert.equal(isBenchAnswerKeycap(five, "4️⃣"), true);
  assert.equal(isBenchAnswerKeycap(five, "5️⃣"), false, "stops at 4");
  assert.equal(
    isBenchAnswerKeycap(card({ kind: "status", options: [] }), "1️⃣"),
    false,
  );
  const receipt = card({ kind: "decision", options: [], request: two });
  assert.equal(isBenchAnswerKeycap(receipt, "2️⃣"), true, "its question's");
  assert.equal(isBenchAnswerKeycap(receipt, "3️⃣"), false);
});

test("deadline: RFC 3339 to unix seconds, null when unparseable", () => {
  assert.equal(benchDeadlineSeconds("2026-10-14T15:00:00Z"), 1_791_990_000);
  assert.equal(benchDeadlineSeconds("soon"), null);
  assert.equal(benchDeadlineSeconds(null), null);
});

// ---------------------------------------------------------------------------
// Shared poll timer
// ---------------------------------------------------------------------------

function fakeTimers() {
  const timers = new Map();
  let next = 1;
  return {
    timers,
    api: {
      setInterval: (callback, ms) => {
        const id = next++;
        timers.set(id, { callback, ms });
        return id;
      },
      clearInterval: (id) => timers.delete(id),
    },
    tick() {
      for (const { callback } of [...timers.values()]) callback();
    },
  };
}

test("poll group: one timer for many subscribers, one fetch per tick", () => {
  const fake = fakeTimers();
  const group = createPollGroup(3_000, () => true, fake.api);
  const calls = [];
  const releaseA = group.retain(() => calls.push("a"));
  const releaseB = group.retain(() => calls.push("b"));
  assert.equal(fake.timers.size, 1);
  assert.equal([...fake.timers.values()][0].ms, 3_000);
  fake.tick();
  assert.deepEqual(calls, ["a"]);
  releaseA();
  fake.tick();
  assert.deepEqual(calls, ["a", "b"], "the next subscriber takes over");
  releaseB();
  assert.equal(fake.timers.size, 0, "the last release stops the timer");
  assert.equal(group.size, 0);
});

test("poll group: no fetch while the window is hidden", () => {
  const fake = fakeTimers();
  let visible = false;
  const group = createPollGroup(3_000, () => visible, fake.api);
  let calls = 0;
  const release = group.retain(() => calls++);
  fake.tick();
  assert.equal(calls, 0);
  visible = true;
  fake.tick();
  assert.equal(calls, 1);
  release();
});

// ---------------------------------------------------------------------------
// Scoped cards: project channels, who may answer, receipts
// ---------------------------------------------------------------------------

const PROJECT = "105f15cc-1d5a-431a-9826-9402fba1a12b";
const CRAIG = "d".repeat(64);
const PROJECT_CARD = "9".repeat(64);

function scoped(overrides = {}) {
  return card({
    id: "reset-worker/welcome",
    channelId: PROJECT,
    project: "reset",
    scoped: true,
    cardEventId: PROJECT_CARD,
    answerers: [{ pk: CRAIG, name: "Craig", role: "decider", agent: false }],
    ownerName: "Quicksilver",
    ...overrides,
  });
}

function answer(overrides = {}) {
  return {
    option: 1,
    answeredAt: 99,
    by: OWNER,
    byName: null,
    byOwner: true,
    source: "tap",
    ...overrides,
  };
}

test("guard: a project card matches in its own channel only", () => {
  const cards = snapshot([card(), scoped()]);
  const inProject = benchRowState(
    cards,
    row({ channelId: PROJECT, id: PROJECT_CARD }),
  );
  assert.equal(inProject.inBenchChannel, true, "polls in the card's channel");
  assert.equal(inProject.match?.card.id, "reset-worker/welcome");
  assert.equal(
    benchRowState(cards, row({ id: PROJECT_CARD })).match,
    null,
    "the same id in #bench is not this card",
  );
  assert.equal(
    benchRowState(cards, row({ channelId: PROJECT })).match,
    null,
    "a #bench card's id in the project channel is not a card",
  );
  assert.deepEqual(
    benchRowState(cards, row({ channelId: OTHER_CHANNEL, id: PROJECT_CARD })),
    { inBenchChannel: false, match: null },
  );
  assert.equal(
    benchRowState(
      cards,
      row({ channelId: PROJECT, id: PROJECT_CARD, signerPubkey: CRAIG }),
    ).match,
    null,
    "only the writer key signs cards",
  );
});

test("eligible: the shared answer vectors agree with item.rs", () => {
  const vectors = JSON.parse(
    readFileSync(
      new URL(
        "../../../../src-tauri/src/bench/answer_vectors.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const keys = new Map();
  const key = (who) => {
    if (!keys.has(who)) keys.set(who, String(keys.size).padStart(64, "a"));
    return keys.get(who);
  };
  const ownerKeys = vectors.ownerSet.map(key);
  let checked = 0;
  for (const c of vectors.cases) {
    const target = {
      needs: c.needs,
      answerers: c.to.map((t) => ({
        pk: key(t.who),
        name: t.who,
        role: t.role,
        agent: t.agent,
      })),
    };
    for (const [who, want] of Object.entries(c.eligible)) {
      assert.equal(
        benchEligible(target, key(who).toUpperCase(), ownerKeys),
        want,
        `${c.name}: ${who}`,
      );
      checked += 1;
    }
  }
  assert.ok(checked > 20, `checked ${checked}`);
});

test("mode: only the owner answers; others are told who can", () => {
  assert.equal(benchCardMode(scoped(), OWNER, OWNER).interactive, true);
  assert.deepEqual(benchCardMode(scoped(), OWNER, CRAIG), {
    interactive: false,
    readOnlyReason: "Answer from your phone.",
  });
  assert.equal(
    benchCardMode(scoped(), OWNER, STRANGER).readOnlyReason,
    "For Craig. Only Craig or Quicksilver can answer.",
  );
  const two = scoped({
    ownerName: null,
    answerers: [
      { pk: CRAIG, name: "Craig", role: "decider", agent: false },
      { pk: "f".repeat(64), name: "Dana", role: "owner", agent: false },
    ],
  });
  assert.equal(
    benchCardMode(two, OWNER, STRANGER).readOnlyReason,
    "For Craig or Dana. Only Craig, Dana or the owner can answer.",
  );
  // An agent on a needs-you card is not eligible here either.
  const agent = scoped({
    answerers: [{ pk: CRAIG, name: "Bot", role: "decider", agent: true }],
  });
  assert.match(benchCardMode(agent, OWNER, CRAIG).readOnlyReason, /^For Bot/);
});

test("labels: For names each answerer and role; owner-only has none", () => {
  assert.equal(benchForLabel(scoped()), "For Craig · Decider");
  assert.equal(benchForLabel(card()), null);
  assert.equal(benchSeverityChip("critical").label, "Your decision");
});

test("options: the recommended option is marked", () => {
  const views = benchOptionViews(
    scoped({ recommend: 2 }),
    INITIAL_BENCH_ANSWER_STATE,
    true,
  );
  assert.deepEqual(
    views.map((o) => o.recommended),
    [false, true],
  );
  assert.ok(
    benchOptionViews(card(), INITIAL_BENCH_ANSWER_STATE, true).every(
      (o) => !o.recommended,
    ),
  );
});

test("reducer: overriding another answer waits for the owner's own", () => {
  // Craig's 2 is recorded; the owner clicks 2.
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 1, 1, false);
  assert.equal(saving.target, 1, "an override, not an undo");
  assert.equal(record(saving, 1, false), saving, "still Craig's answer");
  assert.equal(record(saving, 1, true).phase, "idle");
  // The owner clears their own answer and Craig's older one stands.
  const clearing = click(INITIAL_BENCH_ANSWER_STATE, 0, 0, true);
  assert.equal(clearing.target, null);
  assert.equal(record(clearing, 0, true), clearing);
  assert.equal(record(clearing, 1, false).phase, "idle");
});

test("recorded line: a scoped card names who answered", () => {
  const formatTime = (s) => `t${s}`;
  assert.equal(
    benchRecordedLine(
      scoped({
        recorded: 1,
        answer: answer({ by: CRAIG, byName: "Craig", byOwner: false }),
      }),
      formatTime,
    ),
    "Recorded: 2. Hold · Craig · t99",
  );
  assert.equal(
    benchRecordedLine(scoped({ recorded: 1, answer: answer() }), formatTime),
    "Recorded: 2. Hold · Quicksilver · t99",
  );
  assert.equal(
    benchRecordedLine(
      scoped({ recorded: 1, answer: answer(), ownerName: null }),
      formatTime,
    ),
    "Recorded: 2. Hold · the owner · t99",
  );
  assert.equal(
    benchRecordedLine(card({ recorded: 1, answer: answer() }), formatTime),
    "Recorded: 2. Hold · t99",
    "a legacy card names no one",
  );
});

test("receipt: the request, the answer or Other reply, and the outcome", () => {
  const formatTime = (s) => `t${s}`;
  const outcome = (request, overrides = {}) =>
    card({
      kind: "decision",
      options: [],
      title: "Sent it",
      updatedAt: "2026-10-12T10:00:00Z",
      request,
      ...overrides,
    });
  assert.equal(benchReceipt(card(), formatTime), null);
  const answered = scoped({
    cardEventId: null,
    answer: answer({ option: 0, by: CRAIG, byName: "Craig", byOwner: false }),
  });
  const r = benchReceipt(outcome(answered), formatTime);
  assert.equal(r.request, answered);
  assert.equal(r.answeredLine, "Answered: 1. Ship it · Craig · t99");
  assert.equal(r.outcomeLine, "Outcome · t1791799200");
  const replied = scoped({
    reply: {
      eventId: "1".repeat(64),
      by: OWNER,
      byName: null,
      role: null,
      at: 50,
      hhmm: "14:00",
    },
  });
  assert.equal(
    benchReceipt(outcome(replied), formatTime).answeredLine,
    "Answered: Other, in this thread · Quicksilver · t50",
  );
  const status = outcome(card(), { kind: "status", state: "done" });
  const none = benchReceipt(status, formatTime);
  assert.equal(none.answeredLine, "Answered: no answer recorded");
  assert.equal(none.outcomeLine, "Outcome · Done · t1791799200");
  assert.equal(
    benchReceipt(outcome(card({ answer: answer() })), formatTime).answeredLine,
    "Answered: 2. Hold · t99",
    "a legacy request names no one",
  );
});
