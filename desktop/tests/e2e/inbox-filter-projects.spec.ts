import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("the Projects inbox filter is hidden while Projects is off", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { seedPreviewFeatures: false });
  await page.goto("/");

  await page.getByTestId("inbox-filter-trigger").click();
  await expect(
    page.getByRole("menuitemradio", { name: "Mentions" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitemradio", { name: "Projects" }),
  ).toHaveCount(0);
});

test("the Projects inbox filter is offered while Projects is on", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("inbox-filter-trigger").click();
  await expect(
    page.getByRole("menuitemradio", { name: "Projects" }),
  ).toBeVisible();
});
