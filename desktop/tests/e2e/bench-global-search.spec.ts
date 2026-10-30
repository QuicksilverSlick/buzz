import { expect, test } from "@playwright/test";

import type { BenchCard } from "../../src/features/bench/lib/benchCards";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// The mock bridge's identity, so the viewer is the Bench owner.
const OWNER = "deadbeef".repeat(8);
const HOLD_792 = "steward/hold-reset-792-b59153c6ba09-5dd0d86acaa3";
const CARD_POST = "c".repeat(64);
const DECOY = "d".repeat(64);
const MERGED = "a1".repeat(32);

// A pending steward hold card, as the kit writes it.
function hold(id: string, pr: string, repo: string, more: Partial<BenchCard>) {
  return {
    id,
    writer: "steward",
    kind: "pending",
    title: `PR ${pr} waits for you`,
    body: "What changes: X.",
    area: "buzz",
    severity: "attention",
    options: ["Ship it", "Send it back", "Keep holding"],
    state: null,
    decidedBy: null,
    deadline: null,
    links: [
      {
        label: `Pull request ${pr}`,
        url: `https://github.com/QuicksilverSlick/${repo}/pull/${pr}`,
      },
    ],
    order: 0,
    validity: "current",
    updatedAt: new Date().toISOString(),
    cardEventId: null,
    answer: null,
    recorded: null,
    channelId: null,
    project: null,
    scoped: false,
    needs: "you",
    answerers: [],
    ownerName: null,
    recommend: 1,
    recommender: null,
    reply: null,
    request: null,
    ...more,
  } satisfies BenchCard;
}

const benchCards = {
  enabled: true as const,
  channelId: "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10",
  writerPubkey: "a".repeat(64),
  ownerPubkey: OWNER,
  lastError: null,
  lastTick: new Date().toISOString(),
  items: [
    hold("steward/hold-buzz-94-287009e1603b-49fbb657c0aa", "94", "buzz", {
      body: "What changes: commit 792d3edd. 7928 tests.",
      cardEventId: "b".repeat(64),
    }),
    hold(HOLD_792, "792", "Metabolic-reset", {
      title: "reset PR 792 waits for you: Coach invite emails",
      area: "28-day-reset",
      cardEventId: CARD_POST,
    }),
    hold("orchestrator/pr-719", "719", "Metabolic-reset", {
      writer: "orchestrator",
      kind: "status",
      title: "PR #719 merged and live",
      area: "28-day-reset",
      severity: null,
      options: [],
      state: "done",
    }),
  ],
};

const hit = (event_id: string, content: string) => ({
  event_id,
  content,
  kind: 9,
  pubkey: OWNER,
  channel_id: "28da7000-0000-4000-8000-000000000001",
  channel_name: "28 Day Reset",
  created_at: Math.floor(Date.now() / 1_000) - 120,
  score: 9,
});

test("global search finds a Bench card by PR number and opens it", async ({
  page,
}) => {
  // The mock matches by substring, as a prefix relay would: "792" also
  // returns the 7928 decoy, so this checks the client's own guard.
  await installMockBridge(page, {
    benchCards,
    searchHits: [
      hit(DECOY, "CI: 7928/7929 passed on 792d3edd"),
      hit(MERGED, "Merged PR 792 into main"),
      hit(CARD_POST, "reset PR 792 waits for you"),
    ],
  });
  await page.goto("/");
  await expect(page.getByTestId("sidebar-bench-count")).toBeVisible();

  await page.keyboard.press("ControlOrMeta+k");
  await page.getByTestId("search-dialog-input").fill("#792");
  const dialog = page.getByTestId("search-results");
  const first = dialog.getByRole("option").first();
  await expect(first).toHaveAttribute(
    "data-testid",
    `search-result-bench-${HOLD_792}`,
  );
  await expect(dialog.locator("[data-search-section]").first()).toHaveAttribute(
    "data-search-section",
    "bench",
  );
  await expect(first).toContainText("Red tier");
  await expect(first).toContainText("Needs you");
  await expect(first).toContainText("28 Day Reset");
  await expect(page.getByTestId(`search-result-${MERGED}`)).toBeVisible();
  // The decoy, the card's own post and the buzz 94 card never show.
  await expect(page.getByTestId(`search-result-${DECOY}`)).toHaveCount(0);
  await expect(page.getByTestId(`search-result-${CARD_POST}`)).toHaveCount(0);
  await expect(dialog.getByRole("option")).toHaveCount(2);
  await waitForAnimations(page);
  await dialog.screenshot({
    path: "test-results/bench-search/01-pr-792-dialog.png",
  });

  await first.click();
  await expect(page).toHaveURL(/#\/bench/);
  const focused = page.locator("[data-bench-focused]");
  await expect(focused).toContainText("reset PR 792 waits for you");
  await expect(
    focused.getByRole("button", { name: /^1\. Ship it/ }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.getByTestId("bench-screen").screenshot({
    path: "test-results/bench-search/02-bench-card-focused.png",
  });

  await page.keyboard.press("ControlOrMeta+k");
  await page.getByTestId("search-dialog-input").fill("PR 719");
  await expect(dialog.getByRole("option").first()).toHaveAttribute(
    "data-testid",
    "search-result-bench-orchestrator/pr-719",
  );
  await waitForAnimations(page);
  await dialog.screenshot({
    path: "test-results/bench-search/03-pr-719-dialog.png",
  });
});
