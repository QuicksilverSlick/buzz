import { expect, type Page, test } from "@playwright/test";

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
    // Enough cards above it that the status card opens below the fold.
    ...Array.from({ length: 30 }, (_, i) =>
      hold(
        `steward/hold-buzz-${100 + i}-${"0".repeat(12)}`,
        `${100 + i}`,
        "buzz",
        {},
      ),
    ),
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

const TAB_KEY = "buzz.bench.workbench.v1";

async function open(page: Page, path: string) {
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
  // A saved Dreamforge tab hides the 28 Day Reset cards until a card opens.
  await page.addInitScript(
    (key) => localStorage.setItem(key, "dreamforge"),
    TAB_KEY,
  );
  await page.goto(path);
  await expect(page.getByTestId("sidebar-bench-count")).toBeVisible();
}

test("#792 lists messages with 792 as a whole number, never 7928", async ({
  page,
}) => {
  await open(page, "/");
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByTestId("search-dialog-input").fill("#792");
  const merged = page.getByTestId(`search-result-${MERGED}`);
  await expect(merged.locator("mark")).toHaveText("792");
  await expect(page.getByTestId(`search-result-${DECOY}`)).toHaveCount(0);
});

test("?card= shows All, then scrolls to and focuses that card", async ({
  page,
}) => {
  await open(page, "/#/bench?card=orchestrator%2Fpr-719");
  const focused = page.locator("[data-bench-focused]");
  await expect(focused).toContainText("PR #719 merged and live");
  await expect(focused).toBeInViewport();
  await expect(focused).toBeFocused();
  await expect(page).not.toHaveURL(/card=/);
  expect(await page.evaluate((key) => localStorage.getItem(key), TAB_KEY)).toBe(
    "dreamforge",
  );
});

test("global search finds a Bench card by PR number and opens it", async ({
  page,
}) => {
  await open(page, "/");

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

  await page.keyboard.press("Escape");

  // From a channel the card is selected past "Search in #general", so Enter
  // opens it; scoping to #general hides it.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const input = page.getByTestId("search-dialog-input");
  await page.keyboard.press("ControlOrMeta+k");
  await input.fill("#792");
  const card = page.getByTestId(`search-result-bench-${HOLD_792}`);
  await expect(card).toHaveAttribute("aria-selected", "true");
  await input.press("ArrowUp");
  await input.press("Enter");
  await expect(dialog).toContainText("No messages for #792 in #general");
  await expect(card).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.keyboard.press("ControlOrMeta+k");
  await input.fill("#792");
  await expect(card).toHaveAttribute("aria-selected", "true");
  await input.press("Enter");
  await expect(page).toHaveURL(/#\/bench/);
  const focused = page.locator("[data-bench-focused]");
  await expect(focused).toContainText("reset PR 792 waits for you");
  await expect(focused).toBeFocused();
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
  await dialog.getByRole("option").first().click();
  await expect(focused).toContainText("PR #719 merged and live");
  await expect(focused).toBeFocused();
});

test("global search finds Inbox rows and agent-library personas", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: "release-captain",
        displayName: "Release Captain",
        description: "Cuts desktop releases",
        systemPrompt: "Never searched.",
      },
    ],
  });
  await page.goto("/");
  // Search reads the Inbox the app already loaded.
  const row = page.getByTestId("home-inbox-item-mock-feed-mention");
  await expect(row).toBeVisible();
  const dialog = page.getByTestId("search-results");
  const input = page.getByTestId("search-dialog-input");
  const section = (key: string) =>
    dialog.locator(`[data-search-section="${key}"]`).getByRole("option");

  await page.keyboard.press("ControlOrMeta+k");
  await input.fill("release");
  await expect(section("agent-library")).toContainText("Cuts desktop releases");
  await expect(section("inbox")).toContainText(
    "Please review the release checklist.Mention",
  );
  await waitForAnimations(page);
  await dialog.screenshot({
    path: "test-results/bench-search/05-agent-library-and-inbox.png",
  });
  await section("inbox").click();
  await expect(row).toHaveAttribute("aria-current", "true");

  // Channel activity the Inbox hides stays a message result.
  await page.keyboard.press("ControlOrMeta+k");
  await input.fill("shipped");
  await expect(section("messages").first()).toContainText(
    "Engineering shipped the desktop build.",
  );
  await expect(section("inbox")).toHaveCount(0);

  await input.fill("release captain");
  await section("agent-library").click();
  await expect(page).toHaveURL(/#\/agents\?.*profilePersona=release-captain/);
  await expect(page.getByTestId("user-profile-panel")).toContainText(
    "Release Captain",
  );
});
