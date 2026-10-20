import { expect, test, type Page } from "@playwright/test";

import { KIND_TYPING_INDICATOR } from "../../src/shared/constants/kinds";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

/**
 * The Inbox thread pane shows what a channel's thread panel shows under its
 * composer (agent activity and who is typing in the thread), and a thread
 * whose root was deleted shows a muted row in its place, not a load error.
 */

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const ROOT_ID = "d0".repeat(32);
const ANCHOR_ID = "d1".repeat(32);
// alice is a relay agent in #general; bob and charlie are people there.
const AGENT = TEST_IDENTITIES.alice.pubkey;
const PERSON = TEST_IDENTITIES.bob.pubkey;

/** Seeds a reply to ROOT_ID that mentions the viewer and opens it. */
async function openInboxReply(page: Page, { withRoot }: { withRoot: boolean }) {
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
      const anchor = emit({
        channelName: "general",
        content: "Inbox anchor reply.",
        id: anchorId,
        mentionPubkeys: [currentPubkey],
        parentEventId: rootId,
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

  await emitTyping(page, AGENT, ROOT_ID);
  const trigger = detail.getByTestId("bot-activity-composer-trigger");
  await expect(trigger).toBeVisible();
  await expect(
    detail.getByTestId(`bot-activity-composer-avatar-${AGENT}`),
  ).toBeVisible();

  // Typing in another thread is not this thread's.
  await emitTyping(page, TEST_IDENTITIES.charlie.pubkey, "ee".repeat(32));
  await emitTyping(page, PERSON, ROOT_ID);
  const typingLabel = detail.getByTestId("message-typing-indicator-label");
  await expect(typingLabel).toContainText("bob is typing");
  await expect(typingLabel).not.toContainText("charlie");

  // The agent's reply in the thread ends its activity at once.
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
  await expect(trigger).toHaveCount(0);
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
