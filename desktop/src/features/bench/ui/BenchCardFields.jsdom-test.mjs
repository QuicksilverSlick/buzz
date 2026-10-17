/**
 * One card, three readers: the text the Bench posts (pinned by the Rust
 * test from the same vector) through the desktop's markdown, as the message
 * view shows a card copy; the Bench screen's layout of the same body; and
 * the Inbox row, which shows the title and the first field, never the
 * header line.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterContextProvider,
  createMemoryHistory,
  createRootRoute,
  createRouter,
} from "@tanstack/react-router";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { benchCardLayout } from "../lib/benchCardLayout.ts";
import { Markdown } from "@/shared/ui/markdown";
import { BenchCardSnippet } from "./BenchCardFields.tsx";

const vector = JSON.parse(
  readFileSync(
    new URL(
      "../../../../src-tauri/src/bench/card_layout_vectors.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

const card = {
  id: "orchestrator/pr-758",
  writer: "orchestrator",
  kind: "pending",
  ...vector.drop,
  project: null,
  plain: null,
  recommend: null,
};

async function mount(element) {
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { enabled: false } },
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () =>
    root.render(
      React.createElement(
        RouterContextProvider,
        { router },
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          element,
        ),
      ),
    ),
  );
  return {
    container,
    async unmount() {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

const visible = (node) => node.textContent.replaceAll("﻿", "");

test("the posted text renders its labels bold, apart, with no markdown showing", async () => {
  const view = await mount(
    React.createElement(Markdown, { content: vector.text, interactive: false }),
  );
  try {
    const bold = [...view.container.querySelectorAll("strong")].map(visible);
    const risk = ["Risk", "Undo", "Reach"];
    assert.deepEqual(bold, [
      "Which Production change goes first? (PR 758)",
      ...vector.labels.map((l) => `${l}:`),
    ]);
    // Each field its own paragraph; the risk line's three share one.
    const paragraphs = [...view.container.querySelectorAll("p")].map(visible);
    for (const label of vector.labels.filter((l) => !risk.includes(l))) {
      assert.equal(
        paragraphs.filter((p) => p.startsWith(`${label}:`)).length,
        1,
        label,
      );
    }
    assert.ok(
      paragraphs.includes(
        "Risk: Low · Undo: Easy · Reach: administrators using Production.",
      ),
    );
    const text = visible(view.container);
    assert.ok(!text.includes("**What") && !text.includes(":**"), text);
    assert.ok(!text.includes("\\"), "no escape characters");
    // The writer's own stars stay literal, not emphasis.
    assert.ok(text.includes("*Copy Script*") && text.includes("**notes**"));
    assert.equal(view.container.querySelector("em"), null);
  } finally {
    await view.unmount();
  }
});

test("the Bench screen lays out the same body with the same labels in order", () => {
  const layout = benchCardLayout(card);
  const labels = layout.blocks.flatMap((block) =>
    block.kind === "fields" ? block.fields.map((f) => f.label) : [],
  );
  assert.deepEqual(
    labels,
    vector.labels.filter((l) => l !== "Details"),
  );
  assert.equal(layout.details, "PR #758 changes 1 file; see **notes**.");
});

test("an Inbox row for a card shows its title, then its first field", async () => {
  const view = await mount(React.createElement(BenchCardSnippet, { card }));
  try {
    const [title, field] = view.container.querySelectorAll("p");
    assert.equal(title.textContent, card.title);
    assert.ok(title.classList.contains("font-semibold"));
    assert.equal(
      field.textContent,
      "What changes: Admins get a *Copy Script* button.",
    );
    assert.equal(
      field.querySelector("span.font-semibold").textContent,
      "What changes:",
    );
    assert.ok(!view.container.textContent.includes("orchestrator"));
  } finally {
    await view.unmount();
  }
  // No field: the first paragraph instead.
  const plain = await mount(
    React.createElement(BenchCardSnippet, {
      card: { ...card, body: "Stay or move?\n\nMore later." },
    }),
  );
  try {
    assert.equal(
      plain.container.querySelectorAll("p")[1].textContent,
      "Stay or move?",
    );
  } finally {
    await plain.unmount();
  }
});
