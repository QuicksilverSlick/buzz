import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  benchBodyBlocks,
  benchCardLayout,
  benchCardMeta,
  benchCardPlace,
  benchPostCard,
} from "./benchCardLayout.ts";

const field = (label, text) => ({ kind: "fields", fields: [{ label, text }] });
const text = (t) => ({ kind: "text", text: t });

test("a label opening a line starts a field; lines under it join it until a blank line", () => {
  assert.deepEqual(
    benchBodyBlocks(
      "Stay or move?\nWhat changes: **all** of it\nmore of it\n\nlater\nAsked by:",
    ),
    {
      blocks: [
        text("Stay or move?"),
        field("What changes", "**all** of it\nmore of it"),
        text("later"),
        field("Asked by", ""),
      ],
      details: [],
    },
  );
  assert.deepEqual(benchBodyBlocks("Details: a\nb\nRecommended: Hold"), {
    blocks: [field("Recommended", "Hold")],
    details: ["a\nb"],
  });
  // The orchestrator's sixth label: the trade-off, before the recommendation.
  assert.deepEqual(
    benchBodyBlocks(
      "If you wait: x\nThe choice: speed or safety\nRecommended: Hold",
    ).blocks,
    [
      field("If you wait", "x"),
      field("The choice", "speed or safety"),
      field("Recommended", "Hold"),
    ],
  );
});

test("details keep every paragraph after them until another label", () => {
  // The orchestrator's own format: Details, its evidence, then the date.
  assert.deepEqual(
    benchBodyBlocks(
      "Recommended: Hold - y\n\nDetails: PR changes 2 files.\nChecks: Test passed.\n\nScope: no collisions.\n\n17 Oct 2026 14:05 CT",
    ),
    {
      blocks: [field("Recommended", "Hold - y")],
      details: [
        "PR changes 2 files.\nChecks: Test passed.\n\nScope: no collisions.\n\n17 Oct 2026 14:05 CT",
      ],
    },
  );
  // A value on the next line starts there, with no blank first line.
  assert.deepEqual(benchBodyBlocks("Details:\n- a\n- b"), {
    blocks: [],
    details: ["- a\n- b"],
  });
  // Another label ends them.
  assert.deepEqual(benchBodyBlocks("Details: a\n\nIf you wait: b\n\nc"), {
    blocks: [field("If you wait", "b"), text("c")],
    details: ["a"],
  });
});

test("the risk line splits only at its own labels after '; '", () => {
  assert.deepEqual(
    benchBodyBlocks("Risk: Low; Undo: Easy; Reach: everyone; and more").blocks,
    [
      {
        kind: "fields",
        fields: [
          { label: "Risk", text: "Low" },
          { label: "Undo", text: "Easy" },
          { label: "Reach", text: "everyone; and more" },
        ],
      },
    ],
  );
  assert.deepEqual(benchBodyBlocks("What changes: a; Undo: b").blocks, [
    field("What changes", "a; Undo: b"),
  ]);
});

test("only whitelisted labels at a line's start are labels", () => {
  for (const line of [
    "What happened: merged",
    "Note: What changes: x",
    "the Risk: low",
    "Risky: no",
    "what changes: lowercase",
    "**What changes:** typed bold",
  ]) {
    assert.deepEqual(benchBodyBlocks(line).blocks, [text(line)], line);
  }
});

test("plain fields follow the body in the phone's order, the risk line as fields", () => {
  const layout = benchCardLayout({
    body: "Stay or move?\nDetails: from the body",
    options: ["Stay", "Move"],
    recommend: 2,
    plain: {
      whatChanges: "x",
      askedBy: "Craig",
      risk: "low",
      undo: "impossible",
      reach: "admins",
      ifYouWait: "y",
      confidence: "high",
      details: "from plain",
    },
    action: null,
  });
  assert.deepEqual(
    layout.blocks.map((b) =>
      b.kind === "fields" ? b.fields.map((f) => f.label).join("/") : b.kind,
    ),
    [
      "text",
      "What changes",
      "Asked by",
      "Risk/Undo/Reach",
      "If you wait",
      "Recommended",
    ],
  );
  // The words bench/plain.rs prints for them.
  assert.deepEqual(layout.blocks[3].fields, [
    { label: "Risk", text: "Low" },
    { label: "Undo", text: "Impossible" },
    { label: "Reach", text: "admins" },
  ]);
  assert.equal(layout.details, "from the body\n\nfrom plain");
});

test("the header names a workbench by name, then an area that says more", () => {
  for (const [project, area, place] of [
    [null, "28-day-reset", ["28 Day Reset"]],
    [null, "guest-gate", ["guest-gate"]],
    ["reset", "28-day-reset", ["28 Day Reset"]],
    ["acme", "acme", ["acme"]],
    ["reset", "guest-gate", ["28 Day Reset", "guest-gate"]],
    [null, "dreamforge-build", ["Dreamforge", "dreamforge-build"]],
    [null, "buzz-relay", ["Dreamforge", "buzz-relay"]],
  ]) {
    assert.deepEqual(benchCardPlace({ area, project }), place, area);
  }
  assert.equal(
    benchCardMeta(
      {
        kind: "pending",
        area: "reset-billing",
        project: "reset",
        writer: "orchestrator",
      },
      "28 Day Reset",
    ),
    "Pending · 28 Day Reset · reset-billing · orchestrator",
  );
});

const vector = JSON.parse(
  readFileSync(
    new URL(
      "../../../../src-tauri/src/bench/card_layout_vectors.json",
      import.meta.url,
    ),
    "utf8",
  ),
);
// The live item, reworded since: none of its wording may show.
const LIVE = {
  id: "orchestrator/pr-758",
  writer: "orchestrator",
  kind: "pending",
  title: "Reworded title",
  body: "Reworded body.\nRecommended: Merge",
  area: "28-day-reset",
  options: ["Reworded A", "Reworded B", "Reworded C"],
  deadline: "2026-12-01T00:00:00Z",
  links: [{ label: "Reworded", url: "https://example.com/new" }],
  cardEventId: "1".repeat(64),
  decidedBy: null,
  recommend: 1,
  plain: { whatChanges: "Reworded change", details: "Reworded details" },
  action: "reworded action",
  request: null,
};

test("a replaced post lays out from its own text as the body it was posted from", () => {
  const body = benchBodyBlocks(vector.drop.body);
  for (const text of [vector.text, vector.closed]) {
    const post = benchPostCard(LIVE, text);
    assert.equal(post.meta, "28 Day Reset · orchestrator");
    assert.equal(
      post.card.title,
      "Which Production change goes first? (PR 758)",
    );
    assert.deepEqual(post.card.options, ["Merge it", "Hold"]);
    // The writer's own fields, values and details, read back from the posted
    // text: its bold labels and U+FEFFs are ours.
    assert.deepEqual(benchCardLayout(post.card), {
      blocks: body.blocks,
      details: body.details.join("\n\n"),
      action: null,
    });
    assert.ok(!JSON.stringify(post).includes("Reworded"), "no live wording");
    assert.ok(!JSON.stringify(post).includes("\uFEFF"));
    assert.equal(post.card.id, LIVE.id);
    assert.equal(post.card.cardEventId, LIVE.cardEventId);
  }
  // Closed, it says so in its own words; neither shows a line on how to
  // answer, nor its own thread line (its row opens its thread).
  assert.deepEqual(benchPostCard(LIVE, vector.text).notes, []);
  const closed = benchPostCard(LIVE, vector.closed);
  assert.deepEqual(closed.notes, [
    "Closed: a tap or reply here is no longer read. If this is still asked, answer on the newer card.",
  ]);
  assert.deepEqual(closed.card.links, []);
  assert.equal(closed.card.deadline, null);
});

test("a replaced post's due line, links and other lines; how to answer never shows", () => {
  const post = benchPostCard(
    LIVE,
    [
      "Back to you",
      "🟠 · Dreamforge · reset-worker",
      "**Pick a relay**",
      "",
      "Stay or move?",
      // prose() breaks a list mark and each colon in agent text.
      "\uFEFF1. not an option",
      "due 2026-10-21T17:\uFEFF00:\uFEFF00Z or so",
      "",
      "**Recommended:** 2 (Reset Worker) - cheaper",
      "If you choose Move: run the move",
      "",
      "1. Stay",
      "2. Move",
      "",
      "due 2026-10-21T17:00:00Z",
      "PR 9: https://github.com/block/buzz/pull/9",
      "For: Craig or Quicksilver (owner). Tap one number below to answer.",
      "as of 14:05",
      "Recorded: 2. Move · Craig · 14:07",
      "Or reply in this thread.",
    ].join("\n"),
  );
  assert.equal(post.meta, "Dreamforge · reset-worker");
  assert.equal(post.card.title, "Pick a relay");
  assert.deepEqual(benchCardLayout(post.card).blocks, [
    {
      kind: "text",
      text: "Stay or move?\n1. not an option\ndue 2026-10-21T17:00:00Z or so",
    },
    {
      kind: "fields",
      fields: [
        {
          label: "Recommended",
          text: "2 (Reset Worker) - cheaper\nIf you choose Move: run the move",
        },
      ],
    },
  ]);
  assert.deepEqual(post.card.options, ["Stay", "Move"]);
  assert.equal(post.card.deadline, "2026-10-21T17:00:00Z");
  assert.deepEqual(post.card.links, [
    { label: "PR 9", url: "https://github.com/block/buzz/pull/9" },
  ]);
  assert.deepEqual(post.notes, [
    "For: Craig or Quicksilver (owner).",
    "Recorded: 2. Move · Craig · 14:07",
  ]);
});
