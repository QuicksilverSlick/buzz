/**
 * The Key button: a real button that opens the key, Escape closes it and
 * focus returns to the button. The Inbox's first-run tip shows once, never
 * takes focus, and is gone for good once dismissed or once the key opens.
 */
import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import React from "react";

const AGENT = "c".repeat(64);
let ColorKeyButton;
let COLOR_KEY_TIP_KEY;

before(async () => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  ({ ColorKeyButton, COLOR_KEY_TIP_KEY } = await import("./ColorKey.tsx"));
});

afterEach(() => {
  cleanup();
  localStorage.clear();
});

function mount(props = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(["identity"], { pubkey: "a".repeat(64) });
  client.setQueryData(["projects"], []);
  client.setQueryData(
    ["managed-agents"],
    [
      {
        pubkey: AGENT,
        name: "Reset Worker",
        avatarUrl: null,
        systemPrompt: "never shown",
        status: "running",
      },
    ],
  );
  client.setQueryData(["user-profile", AGENT, "about"], "Builds the Reset.");
  return render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(ColorKeyButton, props),
    ),
  );
}

const trigger = () => screen.getByRole("button", { name: "Color key" });
const keyPopover = () => screen.queryByTestId("color-key-popover");
const tip = () => screen.queryByTestId("color-key-tip");

test("the Key button opens the key; Escape closes it and focus returns", async () => {
  mount();
  const button = trigger();
  assert.equal(button.tagName, "BUTTON");
  assert.equal(button.textContent, "Key", "visible text is Key");
  assert.equal(keyPopover(), null);

  await act(async () => fireEvent.click(button));
  const popover = keyPopover();
  assert.ok(popover, "open");
  assert.equal(popover.getAttribute("aria-label"), "Color key");
  for (const heading of ["Item types", "Bench cards", "Who's who"]) {
    assert.ok(screen.getByRole("heading", { name: heading }), heading);
  }
  assert.match(popover.textContent, /Reset Worker/);
  assert.match(popover.textContent, /Builds the Reset\./);
  assert.doesNotMatch(popover.textContent, /never shown/);
  assert.ok(popover.contains(document.activeElement), "focus moves in");

  await act(async () =>
    fireEvent.keyDown(document.activeElement, { key: "Escape" }),
  );
  assert.equal(keyPopover(), null, "Escape closes");
  assert.equal(document.activeElement, button, "focus returns to the button");
});

test("the first-run tip shows once, keeps focus where it was, and Got it ends it", async () => {
  // Someone is typing when the Inbox mounts.
  const composer = document.createElement("input");
  document.body.append(composer);
  composer.focus();
  const view = mount({ firstRunTip: true });
  assert.ok(tip(), "shown on the first visit");
  assert.match(tip().textContent, /New here\? The Key explains the colors\./);
  assert.equal(document.activeElement, composer, "the tip takes no focus");

  await act(async () =>
    fireEvent.click(screen.getByRole("button", { name: "Got it" })),
  );
  assert.equal(tip(), null);
  assert.equal(localStorage.getItem(COLOR_KEY_TIP_KEY), "seen");
  assert.equal(document.activeElement, composer, "closing takes no focus");

  view.unmount();
  mount({ firstRunTip: true });
  assert.equal(tip(), null, "never again");
  composer.remove();
});

test("opening the key retires the tip for good", async () => {
  const view = mount({ firstRunTip: true });
  assert.ok(tip());
  await act(async () => fireEvent.click(trigger()));
  assert.equal(tip(), null);
  view.unmount();
  mount({ firstRunTip: true });
  assert.equal(tip(), null);
});

test("no tip where it is not asked for (the Bench header)", () => {
  mount();
  assert.equal(tip(), null);
});
