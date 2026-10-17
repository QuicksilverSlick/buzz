import assert from "node:assert/strict";
import test from "node:test";

import {
  benchBodyBlocks,
  benchCardLayout,
  benchCardPlace,
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

test("plain fields follow the body in the phone's order, chips in the risk line's place", () => {
  const layout = benchCardLayout({
    body: "Stay or move?\nDetails: from the body",
    options: ["Stay", "Move"],
    recommend: 2,
    plain: {
      whatChanges: "x",
      askedBy: "Craig",
      risk: "low",
      ifYouWait: "y",
      confidence: "high",
      details: "from plain",
    },
    action: null,
  });
  assert.deepEqual(
    layout.blocks.map((b) =>
      b.kind === "fields" ? b.fields[0].label : b.kind,
    ),
    ["text", "What changes", "Asked by", "chips", "If you wait", "Recommended"],
  );
  assert.equal(layout.details, "from the body\n\nfrom plain");
});

test("the header names a workbench by name, else the slug", () => {
  assert.equal(
    benchCardPlace({ area: "28-day-reset", project: null }),
    "28 Day Reset",
  );
  assert.equal(
    benchCardPlace({ area: "guest-gate", project: "reset" }),
    "28 Day Reset",
  );
  assert.equal(
    benchCardPlace({ area: "dreamforge-build", project: null }),
    "Dreamforge",
  );
  assert.equal(
    benchCardPlace({ area: "guest-gate", project: null }),
    "guest-gate",
  );
});
