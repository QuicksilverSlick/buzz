import assert from "node:assert/strict";
import test from "node:test";

import {
  benchBodyBlocks,
  benchCardLayout,
  benchCardMeta,
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
