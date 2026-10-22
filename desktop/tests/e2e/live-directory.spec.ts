import { expect, test } from "@playwright/test";

import { installMockBridge, openNewMessagePage } from "../helpers/bridge";

const NEW_MEMBER = "9d".repeat(32);

test("a member who joins mid-session appears in the open DM picker", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openNewMessagePage(page);

  const search = page.getByTestId("new-dm-search");
  await search.fill("Quill");
  await expect(page.getByTestId("new-dm-empty")).toBeVisible();

  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_GLOBAL_KIND_SUBSCRIPTION__?.(0),
  );
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_PROFILE__?.({
      pubkey,
      displayName: "Quill Marsh",
    });
  }, NEW_MEMBER);

  // No reload, no reopen: the live kind:0 alone has to refresh the picker.
  await expect(page.getByTestId(`new-dm-result-${NEW_MEMBER}`)).toBeVisible();
  await expect(search).toHaveValue("Quill");
});
