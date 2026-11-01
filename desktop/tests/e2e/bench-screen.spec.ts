import { expect, type Page, test } from "@playwright/test";

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
    .toEqual([{ itemId: ITEM_ID, option: 1, cardEventId: "e".repeat(64) }]);
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

test("a card's typed labels read as bold fields; long details fold", async ({
  page,
}) => {
  const snapshot = benchSnapshot(null);
  if (!snapshot.enabled) throw new Error("the Bench is on");
  snapshot.items[0] = {
    ...snapshot.items[0],
    writer: "orchestrator",
    area: "28-day-reset",
    body: [
      "What changes: Admins get a Copy Script button.",
      "Asked by: Craig at 9:37 AM CT.",
      "Risk: Low; Undo: Easy; Reach: administrators.",
      "",
      `Details: ${"Every check passed on head a30d88a. ".repeat(12)}`,
    ].join("\n"),
  };
  await installMockBridge(page, { benchCards: snapshot });
  await page.goto("/");
  await page.getByTestId("open-bench-view").click();
  const card = page
    .getByTestId("bench-section-needsYou")
    .getByTestId("bench-card");
  await expect(card).toContainText("Pending · 28 Day Reset · orchestrator");
  await expect(card).not.toContainText("28-day-reset");
  const label = card.getByText("What changes:", { exact: true });
  await expect(label).toHaveCSS("font-weight", "600");
  await expect(card.getByText("Undo:", { exact: true })).toBeVisible();
  const details = card.getByTestId("bench-card-details");
  const more = details.getByRole("button", { name: "Show more" });
  await expect(more).toHaveAttribute("aria-expanded", "false");
  const clamped = await details.locator("p").nth(1).boundingBox();
  await more.click();
  await expect(
    details.getByRole("button", { name: "Show less" }),
  ).toHaveAttribute("aria-expanded", "true");
  const open = await details.locator("p").nth(1).boundingBox();
  expect(open?.height ?? 0).toBeGreaterThan(clamped?.height ?? 0);
});

/** A card as first asked, closed once a newer card replaced it (item.rs). */
function closedCardText(channelId: string, postId: string) {
  return [
    "🔴 · Dreamforge · release · dreamforge",
    "**Ship 0.6 before Friday?**",
    "",
    "**What changes:** desktop ships first.",
    "",
    "1. Ship now",
    "2. Wait",
    "",
    "as of 14:05",
    "",
    "Closed: a tap or reply here is no longer read. If this is still asked, answer on the newer card.",
    `Open thread: buzz://message?channel=${channelId}&id=${postId}`,
  ].join("\n");
}

test("a replaced card's post shows read-only from its own text and links the live card", async ({
  page,
}) => {
  // The mock's #general stands in for #bench.
  const generalId = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
  const cardId = "9".repeat(64);
  const oldId = "8".repeat(64);
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
    ({ cardId, itemId, oldId, owner, text, writer }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const marker = [["client", `bench:card:${itemId}@0a1b2c`]];
      const react = (content: string, pubkey: string) =>
        emit?.({
          channelName: "general",
          content,
          extraTags: [["e", oldId]],
          kind: 7,
          pubkey,
        });
      emit?.({
        channelName: "general",
        content: text,
        // Before the tick that wrote the snapshot.
        createdAt: Math.floor(Date.now() / 1_000) - 3_600,
        extraTags: marker,
        id: oldId,
        pubkey: writer,
      });
      // The writer's seeds and the owner's own tap on the old card.
      react("1️⃣", writer);
      react("2️⃣", writer);
      react("2️⃣", owner);
      emit?.({
        channelName: "general",
        content: "card",
        extraTags: marker,
        id: cardId,
        pubkey: writer,
      });
      // Since that tick: maybe the new card the snapshot does not name yet.
      emit?.({
        channelName: "general",
        content: "a newer post of the card",
        extraTags: marker,
        id: "7".repeat(64),
        pubkey: writer,
      });
    },
    {
      cardId,
      itemId: ITEM_ID,
      oldId,
      owner: OWNER,
      text: closedCardText(generalId, oldId),
      writer: WRITER,
    },
  );

  const rows = page.getByTestId("message-row");
  const retired = page.getByTestId("bench-card-retired");
  const oldRow = rows.filter({ has: retired });
  await expect(retired).toContainText("Ship 0.6 before Friday?");
  await expect(retired).not.toContainText("Ship the 0.6 release?");
  await expect(retired.getByText("What changes:", { exact: true })).toHaveCSS(
    "font-weight",
    "600",
  );
  await expect(
    retired.getByRole("list", { name: "Options" }).getByRole("listitem"),
  ).toHaveText(["1. Ship now", "2. Wait"]);
  await expect(retired).toContainText("Closed: a tap or reply here");
  await expect(retired).not.toContainText("Tap one number");
  // Read-only: no answer or Other buttons, no fieldset.
  await expect(retired.getByRole("button")).toHaveCount(0);
  await expect(retired.locator("fieldset")).toHaveCount(0);
  const pill = (emoji: string) =>
    oldRow.getByRole("button", { name: `Toggle ${emoji} reaction` });
  await expect(pill("2️⃣")).toBeVisible();
  await expect(pill("1️⃣")).toHaveCount(0);
  // The live card keeps its buttons.
  const live = rows.filter({ has: page.getByTestId("bench-card") });
  await expect(
    live.getByRole("button", { name: /^1\. Ship it/ }),
  ).toBeEnabled();
  // The post newer than the snapshot is plain text until the next read.
  const fresh = rows.filter({ hasText: "a newer post of the card" });
  await expect(fresh).toHaveCount(1);
  await expect(fresh.getByTestId("bench-card-retired")).toHaveCount(0);
  await expect(retired).toHaveCount(1);

  // In its own channel the jump lands on the live card (here, its thread).
  await retired.getByRole("link", { name: "Go to the newer card" }).click();
  await expect(page).toHaveURL(new RegExp(cardId));
});

/** A steward hold card (answered `recorded`) and an Orchestrator card. */
function holdSnapshot(recorded: number | null): BenchCards {
  const snapshot = benchSnapshot(recorded);
  if (!snapshot.enabled) throw new Error("the Bench is on");
  const [card] = snapshot.items;
  snapshot.items = [
    {
      ...card,
      id: "steward/hold-104",
      writer: "steward",
      title: "Ship buzz PR 104?",
      options: ["Ship it", "Send it back", "Keep holding"],
    },
    {
      ...card,
      id: "orchestrator/pr-767",
      writer: "orchestrator",
      title: "Merge PR 767?",
      options: ["Merge it", "Send back to Reset Worker", "Hold"],
      cardEventId: "c".repeat(64),
      answer: null,
      recorded: null,
    },
  ];
  return snapshot;
}

/** The Bench commands sent, in order, as [command, payload]. */
function sent(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])
      .filter(
        (e) => e.command === "bench_answer" || e.command === "bench_reply",
      )
      .map((e) => [e.command, e.payload]),
  );
}

const tap = (itemId: string, cardEventId: string, option: number) => [
  "bench_answer",
  { itemId, cardEventId, option },
];

test("a hold with a reason sends the tap, then one reply on the card", async ({
  page,
}) => {
  await installMockBridge(page, { benchCards: holdSnapshot(null) });
  await page.goto("/");
  await page.getByTestId("open-bench-view").click();
  await page.getByRole("button", { name: /^3\. Keep holding/ }).click();
  const hold = tap("steward/hold-104", "e".repeat(64), 2);
  await expect.poll(() => sent(page)).toEqual([hold]);
  const why = page.getByRole("textbox", { name: "Why? (optional)" });
  await expect(why).toBeFocused();
  await expect(page.getByText("Sent to steward as a reply")).toBeVisible();
  await why.fill("Waiting on the mobile build");

  // The tick records the tap: the card moves to Answered mid-reason.
  await page.evaluate((next) => {
    const config = window.__BUZZ_E2E__;
    if (config) config.mock = { ...config.mock, benchCards: next };
  }, holdSnapshot(2));
  const answered = page.getByTestId("bench-section-answered");
  await expect(answered.getByTestId("bench-card-recorded")).toHaveText(
    /^Recorded: 3\. Keep holding/,
    { timeout: 10_000 },
  );
  await expect(why).toHaveValue("Waiting on the mobile build");
  await answered.getByRole("button", { name: "Send", exact: true }).click();
  await expect(answered.getByTestId("bench-card-why-sent")).toHaveText(
    "Your reason went to steward as a reply on this card.",
  );
  expect(await sent(page)).toEqual([
    hold,
    [
      "bench_reply",
      {
        itemId: "steward/hold-104",
        cardEventId: "e".repeat(64),
        text: "Waiting on the mobile build",
      },
    ],
  ]);
});

test("a hold without a reason sends only the tap; Ship it never asks", async ({
  page,
}) => {
  await installMockBridge(page, { benchCards: holdSnapshot(null) });
  await page.goto("/");
  await page.getByTestId("open-bench-view").click();
  const box = page.getByRole("form", { name: "Why? (optional)" });
  await page.getByRole("button", { name: /^2\. Send back to Reset/ }).click();
  await expect(page.getByText("Sent to orchestrator as a reply")).toBeVisible();
  await page.getByRole("button", { name: "Skip", exact: true }).click();
  await expect(box).toHaveCount(0);
  await page.getByRole("button", { name: /^1\. Ship it/ }).click();
  const ship = tap("steward/hold-104", "e".repeat(64), 0);
  await expect.poll(() => sent(page)).toContainEqual(ship);
  await expect(box).toHaveCount(0);
  expect(await sent(page)).toEqual([
    tap("orchestrator/pr-767", "c".repeat(64), 1),
    ship,
  ]);
});
