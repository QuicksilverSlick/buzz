import { expect, test } from "@playwright/test";
import { GENERAL_WELCOME_EVENT_ID } from "@/testing/e2eSeedIds";

import { installMockBridge } from "../helpers/bridge";
import {
  openGeneral,
  pressRowAction,
  waitForMockChannelHeadReady,
} from "../helpers/channelHeadReady";
import { clearComposer, replaceComposerText } from "../helpers/composer";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

// The mock identity's own pre-seeded message in #general (authored by
// DEFAULT_MOCK_IDENTITY.pubkey in e2eBridge.ts). Editing/deleting one's own
// message is exactly Sam's workflow: "delete a message by clearing its edit."
const OWN_MESSAGE_ID = GENERAL_WELCOME_EVENT_ID;
const RENDERED_ORIGINAL_CONTENT = "Welcome to general";

// Open the more-actions menu for a message row and wait for the menu to mount.
async function openMoreActionsMenu(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  await pressRowAction(page.getByTestId(`more-actions-${messageId}`));
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible({
    timeout: 5_000,
  });
}

// Enter edit mode for a message, clear it to empty, and submit — the gesture
// that triggers the empty-edit delete confirmation.
async function submitEmptyEdit(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  await openMoreActionsMenu(page, messageId);
  await page.getByTestId(`edit-message-${messageId}`).click();
  await expect(page.getByTestId("edit-target")).toBeVisible({ timeout: 5_000 });
  // Edit mode sets the editor content via Tiptap's async transaction pipeline;
  // wait for it to populate before we clear it.
  const input = page.getByTestId("message-input");
  await expect(input).not.toBeEmpty({ timeout: 5_000 });
  await clearComposer(input);
  await page.keyboard.press("Enter");
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openGeneral(page);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("the timeline applies edit and delete events for the seeded message", async ({
  page,
}) => {
  await waitForMockChannelHeadReady(page, "general", GENERAL_CHANNEL_ID);
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);

  // Both events bypass the UI mutations, so there is no cache patch: the row
  // changes only if the timeline honours an `e` tag naming the seeded id.
  const editedContent = `Edited by event ${Date.now()}`;
  await page.evaluate(
    (input) =>
      window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("edit_message", { input }),
    {
      channelId: GENERAL_CHANNEL_ID,
      eventId: OWN_MESSAGE_ID,
      content: editedContent,
    },
  );
  await expect(row).toContainText(editedContent);

  await page.evaluate(
    (eventId) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        kind: 5,
        content: "",
        extraTags: [["e", eventId]],
      }),
    OWN_MESSAGE_ID,
  );
  await expect(row).toBeHidden();
});

test("clearing an edit to empty prompts to delete, then deletes on confirm", async ({
  page,
}) => {
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });

  await submitEmptyEdit(page, OWN_MESSAGE_ID);

  // The same "Delete message?" confirmation the Delete menu action shows — an
  // empty edit is routed through it, not silently deleted.
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await expect(dialog).toContainText("Delete message?");
  // Edit mode stays active while the dialog is open — it exits only on confirm.
  await expect(page.getByTestId("edit-target")).toBeVisible();

  // Confirm → the message row is removed and edit mode has exited.
  await dialog.getByRole("button", { name: "Delete" }).click();
  await expect(dialog).toBeHidden({ timeout: 5_000 });
  await expect(page.getByTestId("edit-target")).toBeHidden();
  await expect(row).toBeHidden({ timeout: 5_000 });
});

test("cancelling the empty-edit delete keeps the message", async ({ page }) => {
  const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });

  await submitEmptyEdit(page, OWN_MESSAGE_ID);

  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });

  // Cancel → nothing is deleted, the original message survives, and the user is
  // left in edit mode (the editing session is preserved, not discarded).
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden({ timeout: 5_000 });
  await expect(page.getByTestId("edit-target")).toBeVisible();
  await expect(row).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toContainText(
    RENDERED_ORIGINAL_CONTENT,
  );
  await expect(row.getByLabel("Open channel general")).toBeVisible();
});

test("a non-empty edit still edits and never deletes", async ({ page }) => {
  // Edit a message this test sends, not the seeded one: the timeline applies an
  // edit event only when its target is a real 64-hex event id, so an edit of
  // "mock-general-welcome" shows only through the save's cache patch, which the
  // channel's post-subscribe window refresh can overwrite with the original.
  // Ending on "#general" puts the caret after a channel name in edit mode.
  const originalMarker = `Original ${Date.now()}`;
  const input = page.getByTestId("message-input");
  // The trailing space keeps the channel suggestion list closed so Enter sends.
  await input.fill(`${originalMarker} for #general `);
  await input.press("Enter");
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .filter({ hasText: originalMarker });
  let messageId = "";
  await expect
    .poll(
      async () =>
        (messageId = (await row.getAttribute("data-message-id")) ?? ""),
      {
        timeout: 10_000,
      },
    )
    .toMatch(/^[0-9a-f]{64}$/);

  await openMoreActionsMenu(page, messageId);
  await page.getByTestId(`edit-message-${messageId}`).click();
  await expect(page.getByTestId("edit-target")).toBeVisible({ timeout: 5_000 });
  await expect(input).not.toBeEmpty({ timeout: 5_000 });
  // The loaded body ends on "#general", so its suggestion list opens. Typing
  // over it must close it, or Enter picks "#general" instead of saving.
  const channelSuggestion = page
    .getByTestId("message-composer")
    .getByRole("button", { name: "#general" });
  await expect(channelSuggestion).toBeVisible({ timeout: 5_000 });
  const editedContent = `Edited, not deleted ${Date.now()}`;

  await replaceComposerText(input, editedContent);
  await page.keyboard.press("Enter");

  // No delete confirmation, edit mode exits, the row survives with new text.
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(page.getByTestId("edit-target")).toBeHidden({ timeout: 10_000 });
  const editedRow = page.locator(`[data-message-id="${messageId}"]`);
  await expect(editedRow).toContainText(editedContent);
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    originalMarker,
  );
});

// Clearing a draft must close its autocomplete at once. An Enter inside the
// 120 ms debounce used to pick the stale suggestion instead of submitting, so
// the delete prompt never appeared. The draft's trailing `#general` opens the
// channel list on its own; the other triggers are typed after it.
for (const { trigger, typed, popover } of [
  { trigger: "#channel", typed: "", popover: "channel-autocomplete" },
  { trigger: "@mention", typed: " @al", popover: "mention-autocomplete" },
  { trigger: ":emoji", typed: " :smi", popover: "emoji-autocomplete" },
]) {
  test(`an empty edit submits while the ${trigger} autocomplete is open`, async ({
    page,
  }) => {
    const row = page.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`);
    await expect(row).toBeVisible({ timeout: 10_000 });
    await openMoreActionsMenu(page, OWN_MESSAGE_ID);
    await page.getByTestId(`edit-message-${OWN_MESSAGE_ID}`).click();
    await expect(page.getByTestId("edit-target")).toBeVisible({
      timeout: 5_000,
    });
    // The channel list opens only once the caret has landed after `#general`.
    await expect(page.getByTestId("channel-autocomplete")).toBeVisible({
      timeout: 5_000,
    });
    await page.keyboard.type(typed);
    await expect(page.getByTestId(popover)).toBeVisible({ timeout: 5_000 });

    // Clear and submit back-to-back, well inside the debounce.
    await page.keyboard.press("ControlOrMeta+A");
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Enter");

    await expect(page.getByRole("alertdialog")).toContainText(
      "Delete message?",
      { timeout: 5_000 },
    );
    await expect(page.getByTestId("message-input")).toBeEmpty();
  });
}
