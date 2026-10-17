/**
 * The share prompt above the composer: a long roster stays short, the quote
 * it would send is visible before the choice, an open channel says who'll
 * read it, and Escape dismisses it.
 */
import assert from "node:assert/strict";
import test from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import {
  ComposerShareLinkPrompt,
  formatShareNames,
} from "./ComposerShareLinkPrompt.tsx";

test("names the first three, then counts the rest", () => {
  assert.equal(formatShareNames(["Ana"]), "Ana");
  assert.equal(formatShareNames(["Ana", "Bo", "Cy"]), "Ana, Bo, Cy");
  assert.equal(
    formatShareNames(["Ana", "Bo", "Cy", "Di"]),
    "Ana, Bo, Cy and 1 other",
  );
  const many = Array.from({ length: 59 }, (_, i) => `M${i}`);
  assert.equal(formatShareNames(many), "M0, M1, M2 and 56 others");
});

test("shows the quote it would send; Escape dismisses", async () => {
  const calls = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const prompt = {
    names: ["Reset Worker"],
    openChannel: "general",
    quote: "> Quoted from **Dana** in DM with Dana, today:\n>\n> the plan",
  };
  try {
    await act(async () =>
      root.render(
        React.createElement(ComposerShareLinkPrompt, {
          onDismiss: () => calls.push("dismiss"),
          onIncludeQuote: () => calls.push("include"),
          onSendLinkOnly: () => calls.push("link"),
          prompt,
        }),
      ),
    );
    const alert = container.querySelector('[role="alert"]');
    assert.match(alert.textContent, /^Reset Worker can't open that link\./);
    assert.match(
      alert.textContent,
      /Everyone who can read #general will see it\.$/,
    );
    assert.equal(
      container.querySelector('[data-testid="share-link-quote"]').textContent,
      "Quoted from **Dana** in DM with Dana, today:\n\nthe plan",
    );
    const [include] = container.querySelectorAll("button");
    include.focus();
    await act(async () =>
      include.dispatchEvent(
        new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }),
      ),
    );
    assert.deepEqual(calls, ["dismiss"]);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
