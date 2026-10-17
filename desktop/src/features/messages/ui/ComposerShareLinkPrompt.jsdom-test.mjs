/**
 * The share prompt above the composer: a long roster stays short, the quote
 * it would send is visible before the choice, an open channel says who'll
 * read it, and Escape dismisses it.
 */
import assert from "node:assert/strict";
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
  // The quote renders through Markdown, which reads router and query context.
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { enabled: false } },
  });
  try {
    await act(async () =>
      root.render(
        React.createElement(
          RouterContextProvider,
          { router },
          React.createElement(
            QueryClientProvider,
            { client },
            React.createElement(ComposerShareLinkPrompt, {
              onDismiss: () => calls.push("dismiss"),
              onIncludeQuote: () => calls.push("include"),
              onSendLinkOnly: () => calls.push("link"),
              prompt,
            }),
          ),
        ),
      ),
    );
    const alert = container.querySelector('[role="alert"]');
    assert.match(alert.textContent, /^Reset Worker can't open that link\./);
    assert.match(
      alert.textContent,
      /Everyone who can read #general will see it\.$/,
    );
    // As the message will read: a quote, the name bold, no markdown showing.
    const quote = container.querySelector('[data-testid="share-link-quote"]');
    assert.ok(quote.querySelector("blockquote"));
    assert.equal(quote.querySelector("strong")?.textContent, "Dana");
    assert.equal(
      quote.textContent.trim().replace(/\s+/g, " "),
      "Quoted from Dana in DM with Dana, today: the plan",
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
