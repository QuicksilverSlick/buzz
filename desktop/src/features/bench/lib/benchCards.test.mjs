import assert from "node:assert/strict";
import test from "node:test";

import {
  BENCH_SAVE_TIMEOUT_MESSAGE,
  INITIAL_BENCH_ANSWER_STATE,
  answerTarget,
  benchAnswerReducer,
  benchCardMode,
  benchDeadlineSeconds,
  benchOptionViews,
  benchRecordedLine,
  benchRowState,
  benchSeverityChip,
  benchStateLabel,
  benchValidityLabel,
  createPollGroup,
  indexBenchCards,
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
    label: "Critical",
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

function click(state, option, recorded) {
  return benchAnswerReducer(state, { type: "click", option, recorded });
}

test("answerTarget: clicking the recorded option undoes it", () => {
  assert.equal(answerTarget(1, null), 1);
  assert.equal(answerTarget(1, 0), 1);
  assert.equal(answerTarget(1, 1), null);
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
    benchAnswerReducer(saving, { type: "recorded", recorded: null }),
    saving,
    "an unchanged snapshot keeps it saving",
  );
  assert.equal(
    benchAnswerReducer(saving, { type: "recorded", recorded: 0 }),
    saving,
    "another option recorded (a newer phone tap) is not this click",
  );
  assert.deepEqual(
    benchAnswerReducer(saving, { type: "recorded", recorded: 1 }),
    { ...INITIAL_BENCH_ANSWER_STATE, attempt: 1 },
  );
});

test("reducer: undo waits for the card to show no answer", () => {
  const saving = click(INITIAL_BENCH_ANSWER_STATE, 0, 0);
  assert.equal(saving.target, null);
  assert.equal(saving.clicked, 0);
  assert.equal(
    benchAnswerReducer(saving, { type: "recorded", recorded: null }).phase,
    "idle",
  );
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
  assert.equal(
    benchAnswerReducer(failed, { type: "recorded", recorded: 1 }).phase,
    "idle",
  );
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
