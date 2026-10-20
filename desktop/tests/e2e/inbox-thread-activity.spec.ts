import { expect, test, type Page } from "@playwright/test";

import type { BenchCards } from "../../src/features/bench/lib/benchCards";
import { KIND_TYPING_INDICATOR } from "../../src/shared/constants/kinds";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

/**
 * The Inbox thread pane shows what a channel's thread panel shows under its
 * composer (agent activity and who is typing in the thread), and a thread
 * whose root was deleted shows a muted row in its place, not a load error.
 * A Bench card's post a newer card replaced shows read-only, as the card
 * its own text is.
 */

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const ROOT_ID = "d0".repeat(32);
const ANCHOR_ID = "d1".repeat(32);
// alice is a relay agent in #general; bob and charlie are people there.
const AGENT = TEST_IDENTITIES.alice.pubkey;
const PERSON = TEST_IDENTITIES.bob.pubkey;

/**
 * Seeds a reply to ROOT_ID (or to `middleId`, itself a reply to ROOT_ID)
 * that mentions the viewer and opens it.
 */
async function openInboxReply(
  page: Page,
  { middleId, withRoot }: { middleId?: string; withRoot: boolean },
) {
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function",
  );
  await page.evaluate(
    ({
      anchorId,
      channelId,
      currentPubkey,
      middleId,
      rootId,
      senderPubkey,
      withRoot,
    }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const push = window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!emit || !push) throw new Error("Bridge helpers not ready");
      if (withRoot) {
        emit({
          channelName: "general",
          content: "Thread root message.",
          id: rootId,
          pubkey: senderPubkey,
        });
      }
      if (middleId) {
        emit({
          channelName: "general",
          content: "Middle reply.",
          id: middleId,
          parentEventId: rootId,
          pubkey: senderPubkey,
        });
      }
      const anchor = emit({
        channelName: "general",
        content: "Inbox anchor reply.",
        id: anchorId,
        mentionPubkeys: [currentPubkey],
        parentEventId: middleId ?? rootId,
        pubkey: senderPubkey,
      });
      push({
        category: "mention",
        channel_id: channelId,
        channel_name: "general",
        content: anchor.content,
        created_at: anchor.created_at,
        id: anchor.id,
        kind: anchor.kind,
        pubkey: anchor.pubkey,
        tags: anchor.tags,
      });
    },
    {
      anchorId: ANCHOR_ID,
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: TEST_IDENTITIES.tyler.pubkey,
      middleId,
      rootId: ROOT_ID,
      senderPubkey: PERSON,
      withRoot,
    },
  );
  await page.getByTestId(`home-inbox-item-${ANCHOR_ID}`).click();
  const detail = page.getByTestId("home-inbox-detail");
  await expect(detail).toContainText("Inbox anchor reply.");
  return detail;
}

async function emitTyping(page: Page, pubkey: string, threadHeadId: string) {
  await page.evaluate(
    (input) => {
      window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
        channelName: "general",
        ...input,
      });
    },
    { pubkey, threadHeadId },
  );
}

test("shows agent activity and thread typing under the Inbox composer", async ({
  page,
}) => {
  // No dock transitions, so the overlay's height is final at once.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  const detail = await openInboxReply(page, { withRoot: true });
  await expect
    .poll(
      () =>
        page.evaluate(
          (kind) =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "general",
              kind,
            }) ?? false,
          KIND_TYPING_INDICATOR,
        ),
      // Live subscriptions are admitted at a paced rate after startup.
      { timeout: 15_000 },
    )
    .toBe(true);

  const overlay = page.getByTestId("home-inbox-detail-composer-overlay");
  const overlayHeight = async () => (await overlay.boundingBox())?.height ?? 0;
  const quietHeight = await overlayHeight();
  expect(quietHeight).toBeGreaterThan(0);

  await emitTyping(page, AGENT, ROOT_ID);
  const trigger = detail.getByTestId("bot-activity-composer-trigger");
  await expect(trigger).toBeVisible();
  // The activity rail fills the dock's reserved space: the composer stays put.
  expect(Math.abs((await overlayHeight()) - quietHeight)).toBeLessThanOrEqual(
    1,
  );
  await expect(
    detail.getByTestId(`bot-activity-composer-avatar-${AGENT}`),
  ).toBeVisible();

  // Typing in another thread is not this thread's.
  await emitTyping(page, TEST_IDENTITIES.charlie.pubkey, "ee".repeat(32));
  await emitTyping(page, PERSON, ROOT_ID);
  const typingLabel = detail.getByTestId("message-typing-indicator-label");
  await expect(typingLabel).toContainText("bob is typing");
  await expect(typingLabel).not.toContainText("charlie");

  // The agent's reply in the thread ends its activity at once: fresh typing
  // first, so the typing TTL cannot be what clears it.
  await emitTyping(page, AGENT, ROOT_ID);
  await expect(trigger).toBeVisible();
  await page.evaluate(
    ({ pubkey, rootId }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "Agent reply done.",
        parentEventId: rootId,
        pubkey,
      });
    },
    { pubkey: AGENT, rootId: ROOT_ID },
  );
  await expect(detail).toContainText("Agent reply done.");
  await expect(trigger).toHaveCount(0, { timeout: 2_000 });
});

test("a deleted thread root shows a muted row, not the load error", async ({
  page,
}) => {
  await installMockBridge(page, { deletedEventIds: [ROOT_ID] });
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  const detail = await openInboxReply(page, { withRoot: false });

  await expect(detail.getByTestId("home-inbox-root-deleted")).toHaveText(
    "Original message was deleted",
  );
  await expect(detail.getByTestId("home-inbox-context-error")).toHaveCount(0);
  await expect(detail).toContainText("Inbox anchor reply.");

  // The deleted root cannot be a reply's parent, so the reply goes under
  // the selected message and still sends.
  const input = detail.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("Reply under a deleted root.");
  await page.keyboard.press("Enter");
  await expect(
    detail
      .getByTestId("home-inbox-context-message")
      .filter({ hasText: "Reply under a deleted root." }),
  ).toBeVisible();
  await expect(input).toHaveText("");
});

test("a deleted reply mid-thread is not a load error", async ({ page }) => {
  const middleId = "d2".repeat(32);
  await installMockBridge(page, { deletedEventIds: [middleId] });
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  const detail = await openInboxReply(page, { middleId, withRoot: true });

  await expect(detail).toContainText("Thread root message.");
  await expect(detail.getByTestId("home-inbox-context-error")).toHaveCount(0);
  await expect(detail.getByTestId("home-inbox-root-deleted")).toHaveCount(0);
});

test("a root that fails to load keeps the load-error banner", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  // Fail get_event for the root at the IPC boundary, as a relay outage would.
  await page.evaluate((rootId) => {
    const w = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload: unknown,
          options: unknown,
        ) => Promise<unknown>;
      };
    };
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
      if (
        command === "get_event" &&
        (payload as { eventId?: string } | undefined)?.eventId === rootId
      ) {
        throw new Error("relay unreachable: request timed out");
      }
      return original(command, payload, options);
    };
  }, ROOT_ID);
  const detail = await openInboxReply(page, { withRoot: false });

  await expect(detail.getByTestId("home-inbox-context-error")).toBeVisible();
  await expect(detail.getByTestId("home-inbox-root-deleted")).toHaveCount(0);
});

test("a replaced Bench card's post shows read-only from its own text", async ({
  page,
}) => {
  const writer = "a".repeat(64);
  const itemId = "orchestrator/ship";
  const oldId = "d3".repeat(32);
  const liveId = "d4".repeat(32);
  const benchCards: BenchCards = {
    enabled: true,
    // The mock's #general stands in for #bench.
    channelId: GENERAL_CHANNEL_ID,
    writerPubkey: writer,
    ownerPubkey: "deadbeef".repeat(8),
    lastError: null,
    lastTick: new Date().toISOString(),
    items: [
      {
        id: itemId,
        writer: "orchestrator",
        kind: "pending",
        title: "Ship it now?",
        body: "Asked again, reworded.",
        area: "28-day-reset",
        severity: "critical",
        options: ["Ship", "Hold"],
        state: null,
        decidedBy: null,
        deadline: null,
        links: [],
        order: 0,
        validity: "current",
        updatedAt: new Date().toISOString(),
        cardEventId: liveId,
        answer: null,
        recorded: null,
        channelId: null,
        project: null,
        scoped: false,
        needs: "you",
        answerers: [],
        ownerName: null,
        recommend: null,
        recommender: null,
        reply: null,
        request: null,
      },
    ],
  };
  await installMockBridge(page, { benchCards });
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function",
  );
  await page.evaluate(
    ({ channelId, itemId, oldId, writer }) => {
      const post = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: [
          "🔴 · 28 Day Reset · orchestrator",
          "**Ship it before Friday?**",
          "",
          "**What changes:** the old welcome email",
          "",
          "1. Ship now",
          "2. Wait",
          "",
          "as of 14:05",
          "",
          "Closed: a tap or reply here is no longer read. If this is still asked, answer on the newer card.",
        ].join("\n"),
        extraTags: [["client", `bench:card:${itemId}@0a1b2c`]],
        id: oldId,
        pubkey: writer,
      });
      if (!post) throw new Error("Bridge helpers not ready");
      window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?.({
        category: "mention",
        channel_id: channelId,
        channel_name: "general",
        content: post.content,
        created_at: post.created_at,
        id: post.id,
        kind: post.kind,
        pubkey: post.pubkey,
        tags: post.tags,
      });
    },
    { channelId: GENERAL_CHANNEL_ID, itemId, oldId, writer },
  );

  // The row and the pane show that post's own wording, never the live card's.
  const row = page.getByTestId(`home-inbox-item-${oldId}`);
  await expect(row.getByTestId("inbox-bench-snippet")).toContainText(
    "Ship it before Friday?",
  );
  await row.click();
  const retired = page
    .getByTestId("home-inbox-detail")
    .getByTestId("bench-card-retired");
  await expect(retired).toContainText("Ship it before Friday?");
  await expect(retired).toContainText("the old welcome email");
  await expect(retired).not.toContainText("Ship it now?");
  await expect(retired.getByRole("button")).toHaveCount(0);
  await expect(retired).not.toContainText("Tap one number");
  await retired.getByRole("link", { name: "Go to the newer card" }).click();
  await expect(page).toHaveURL(new RegExp(`messageId=${liveId}`));
});
