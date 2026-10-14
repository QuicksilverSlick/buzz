import { expect, test } from "@playwright/test";

import type { BenchCards } from "../../src/features/bench/lib/benchCards";
import { installMockBridge } from "../helpers/bridge";

// The mock bridge's identity, so the viewer is the Bench owner.
const OWNER = "deadbeef".repeat(8);
const ITEM_ID = "dreamforge/ship-release";

const WRITER = "a".repeat(64);

// A legacy (unscoped) card: #bench, owner-only, no Other.
const LEGACY = {
  updatedAt: new Date().toISOString(),
  channelId: null,
  project: null,
  scoped: false,
  needs: "you" as const,
  answerers: [],
  ownerName: null,
  recommend: null,
  recommender: null,
  reply: null,
  request: null,
};

function benchSnapshot(
  answeredOption: number | null,
  channelId = "8f6c1a52-3f0b-4f5e-9a51-0e2f5c7d9b10",
  cardEventId = "e".repeat(64),
): BenchCards {
  const answeredAt = Math.floor(Date.now() / 1_000);
  return {
    enabled: true,
    channelId,
    writerPubkey: WRITER,
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
        cardEventId,
        answer:
          answeredOption === null
            ? null
            : {
                option: answeredOption,
                answeredAt,
                by: OWNER,
                byName: null,
                byOwner: true,
                source: "tap",
              },
        recorded: answeredOption,
        ...LEGACY,
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
        ...LEGACY,
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

test("a live card hides its keycap pills; other messages keep theirs", async ({
  page,
}) => {
  // The mock's #general stands in for #bench.
  const generalId = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
  const cardId = "9".repeat(64);
  await installMockBridge(page, {
    benchCards: benchSnapshot(null, generalId, cardId),
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(
    () =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
        kind: 7,
      }) === true,
  );

  await page.evaluate(
    ({ cardId, owner, writer }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const react = (target: string, content: string, pubkey: string) =>
        emit?.({
          channelName: "general",
          content,
          extraTags: [["e", target]],
          kind: 7,
          pubkey,
        });
      emit?.({
        channelName: "general",
        content: "card",
        id: cardId,
        pubkey: writer,
      });
      // The writer's seeded keycaps and the owner's tap, as phones see them.
      react(cardId, "1️⃣", writer);
      react(cardId, "2️⃣", writer);
      react(cardId, "2️⃣", owner);
      // The mock's "React to me with a custom emoji" message.
      react("d".repeat(64), "1️⃣", owner);
      react(cardId, "\u{1F44D}", owner);
    },
    { cardId, owner: OWNER, writer: WRITER },
  );

  const rows = page.getByTestId("message-row");
  const cardRow = rows.filter({ has: page.getByTestId("bench-card") });
  const plainRow = rows
    .filter({ hasText: "React to me with a custom emoji" })
    .last();
  const pill = (row: typeof rows, emoji: string) =>
    row.getByRole("button", { name: `Toggle ${emoji} reaction` });
  await expect(cardRow).toContainText("Ship the 0.6 release?");
  await expect(pill(cardRow, "\u{1F44D}")).toBeVisible();
  await expect(pill(plainRow, "1️⃣")).toBeVisible();
  await expect(pill(cardRow, "1️⃣")).toHaveCount(0);
  await expect(pill(cardRow, "2️⃣")).toHaveCount(0);
});
