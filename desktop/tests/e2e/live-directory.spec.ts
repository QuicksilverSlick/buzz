import { expect, type Page, test } from "@playwright/test";

import {
  installMockBridge,
  openNewMessagePage,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const NEW_MEMBER = "9d".repeat(32);
const GAP_MEMBER = "9e".repeat(32);

async function waitForLiveProfiles(page: Page) {
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_GLOBAL_KIND_SUBSCRIPTION__?.(0),
  );
}

async function emitProfile(page: Page, pubkey: string, displayName: string) {
  await page.evaluate(
    (input) => window.__BUZZ_E2E_EMIT_MOCK_PROFILE__?.(input),
    { pubkey, displayName },
  );
}

test("a member who joins mid-session appears in the open DM picker", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openNewMessagePage(page);

  const search = page.getByTestId("new-dm-search");
  await search.fill("Quill");
  await expect(page.getByTestId("new-dm-empty")).toBeVisible();

  await waitForLiveProfiles(page);
  await emitProfile(page, NEW_MEMBER, "Quill Marsh");

  // No reload, no reopen: the live kind:0 alone has to refresh the picker.
  await expect(page.getByTestId(`new-dm-result-${NEW_MEMBER}`)).toBeVisible();
  await expect(search).toHaveValue("Quill");
});

test("a member renamed mid-session is renamed in an open channel roster", async ({
  page,
}) => {
  const charlie = TEST_IDENTITIES.charlie.pubkey;
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await page.getByTestId("channel-members-trigger").click();
  const row = page.getByTestId(`sidebar-member-${charlie}`);
  await expect(row).toBeVisible();
  await expect(row).not.toContainText("Charlie Reyes");

  await waitForLiveProfiles(page);
  await emitProfile(page, charlie, "Charlie Reyes");

  // The roster stays fresh for minutes, so only the live rename can do this.
  await expect(row).toContainText("Charlie Reyes");
});

test("an own-profile edit from another device reaches this desktop", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  const name = page.getByTestId("sidebar-profile-name");
  await expect(name).toBeVisible();
  const { pubkey } = (await page.evaluate(() =>
    window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_identity"),
  )) as { pubkey: string };

  await waitForLiveProfiles(page);
  await emitProfile(page, pubkey, "Edited On Phone");

  await expect(name).toHaveText("Edited On Phone");
});

test("a profile edited while the socket was down is current after reconnect", async ({
  page,
}) => {
  // Seeded rather than live-published, so no live refresh is still pending
  // when the gap starts and only the reconnect path can update the label.
  await installMockBridge(page, {
    searchProfiles: [{ pubkey: GAP_MEMBER, displayName: "Before Gap" }],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForLiveProfiles(page);
  await page.evaluate(
    (pubkey) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "posted before the gap",
        pubkey,
      }),
    GAP_MEMBER,
  );
  const author = page
    .getByTestId("message-author")
    .filter({ hasText: /Before Gap|After Gap/ });
  await expect(author).toHaveText("Before Gap");

  // The edit lands while no socket is open, so no live kind:0 reaches the app.
  await page.evaluate(() =>
    window.__BUZZ_E2E_SET_MOCK_WEBSOCKET_UNAVAILABLE__?.(true),
  );
  await page.evaluate(() => window.__BUZZ_E2E_DISCONNECT_MOCK_WEBSOCKETS__?.());
  await emitProfile(page, GAP_MEMBER, "After Gap");
  await page.evaluate(() => {
    window.__BUZZ_E2E_SET_MOCK_WEBSOCKET_UNAVAILABLE__?.(false);
    window.dispatchEvent(new Event("online"));
  });
  await expect
    .poll(() =>
      page.evaluate(() => window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.()),
    )
    .toBe("connected");

  // Auto-heal must refetch the label, not re-serve the 10-minute entry.
  await expect(author).toHaveText("After Gap");
});
