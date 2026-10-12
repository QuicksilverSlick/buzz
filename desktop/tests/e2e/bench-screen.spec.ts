import { expect, test } from "@playwright/test";

import type { BenchCards } from "../../src/features/bench/lib/benchCards";
import { installMockBridge } from "../helpers/bridge";

// The mock bridge's identity, so the viewer is the Bench owner.
const OWNER = "deadbeef".repeat(8);
const ITEM_ID = "dreamforge/ship-release";

function benchSnapshot(answeredOption: number | null): BenchCards {
  const answeredAt = Math.floor(Date.now() / 1_000);
  return {
    enabled: true,
    channelId: "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10",
    writerPubkey: "a".repeat(64),
    ownerPubkey: OWNER,
    lastError: null,
    lastTick: new Date().toISOString(),
    items: [
      {
        id: ITEM_ID,
        writer: "dreamforge",
        kind: "pending",
        title: "Ship the 0.6 release?",
        body: "QA signed off on desktop; mobile is one build behind.",
        area: "release",
        severity: "critical",
        options: ["Ship it", "Hold for mobile"],
        state: null,
        decidedBy: null,
        deadline: null,
        links: [],
        order: 0,
        validity: "current",
        cardEventId: "e".repeat(64),
        answer:
          answeredOption === null
            ? null
            : { option: answeredOption, answeredAt },
        recorded: answeredOption,
      },
      {
        id: "dreamforge/relay",
        writer: "dreamforge",
        kind: "status",
        title: "Relay migration",
        body: "",
        area: "infra",
        severity: null,
        options: [],
        state: "in-progress",
        decidedBy: null,
        deadline: null,
        links: [],
        order: 0,
        validity: "current",
        cardEventId: "f".repeat(64),
        answer: null,
        recorded: null,
      },
    ],
  };
}

test("Bench screen answers a pending card and shows it recorded", async ({
  page,
}) => {
  await installMockBridge(page, { benchCards: benchSnapshot(null) });
  await page.goto("/");

  const entry = page.getByTestId("open-bench-view");
  await expect(entry).toBeVisible();
  await expect(page.getByTestId("sidebar-bench-count")).toHaveText("1");
  await entry.click();
  await expect(page).toHaveURL(/#\/bench$/);
  await expect(page.getByTestId("bench-screen")).toBeVisible();
  await expect(
    page
      .getByTestId("bench-section-status")
      .getByText("Relay migration", { exact: true }),
  ).toBeVisible();

  const needsYou = page.getByTestId("bench-section-needsYou");
  const card = needsYou.getByTestId("bench-card");
  await expect(card).toContainText("Ship the 0.6 release?");
  await card.getByRole("button", { name: /^2\. Hold for mobile/ }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])
          .filter((entry) => entry.command === "bench_answer")
          .map((entry) => entry.payload),
      ),
    )
    .toEqual([{ itemId: ITEM_ID, option: 1 }]);
  await expect(card.getByText("Saving...")).toBeVisible();

  // The Bench's tick records the owner's keycap; the screen's 3 s poll
  // picks up the new snapshot.
  await page.evaluate((next) => {
    const config = window.__BUZZ_E2E__;
    if (config) config.mock = { ...config.mock, benchCards: next };
  }, benchSnapshot(1));

  const answered = page.getByTestId("bench-section-answered");
  await expect(answered.getByTestId("bench-card-recorded")).toHaveText(
    /^Recorded: 2\. Hold for mobile/,
    { timeout: 10_000 },
  );
  await expect(
    needsYou.getByText("Nothing needs you right now."),
  ).toBeVisible();
  await expect(page.getByTestId("sidebar-bench-count")).toHaveCount(0);
});
