import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const TIP_KEY = "buzz.color-key.tip.v1";
const HEADINGS = ["Item types", "Your projects", "Bench cards", "Who's who"];

test("the Inbox's first-run tip points at the Key until Got it", async ({
  page,
}) => {
  await installMockBridge(page);
  // The bridge marks the tip seen for every other spec; unmark it once.
  await page.addInitScript((key) => {
    if (sessionStorage.getItem("color-key-tip-reset")) return;
    sessionStorage.setItem("color-key-tip-reset", "1");
    localStorage.removeItem(key);
  }, TIP_KEY);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();

  const tip = page.getByTestId("color-key-tip");
  await expect(tip).toHaveText(/New here\? The Key explains the colors\./);
  await tip.getByRole("button", { name: "Got it" }).click();
  await expect(tip).toHaveCount(0);
  expect(await page.evaluate((key) => localStorage.getItem(key), TIP_KEY)).toBe(
    "seen",
  );
});

test("the Inbox Key opens from the keyboard; Escape closes it", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();

  const key = page.getByRole("button", { name: "Color key" });
  await expect(key).toHaveText("Key");
  await key.focus();
  await page.keyboard.press("Enter");
  const popover = page.getByRole("dialog", { name: "Color key" });
  for (const name of HEADINGS) {
    await expect(popover.getByRole("heading", { name })).toBeVisible();
  }
  await page.keyboard.press("Escape");
  await expect(popover).toHaveCount(0);
  await expect(key).toBeFocused();
});

test("the Bench Key fits a narrow window and scrolls inside", async ({
  page,
}) => {
  await page.setViewportSize({ width: 420, height: 640 });
  await installMockBridge(page, {
    benchCards: {
      enabled: true,
      channelId: "",
      writerPubkey: "a".repeat(64),
      ownerPubkey: "deadbeef".repeat(8),
      lastError: null,
      lastTick: new Date().toISOString(),
      items: [],
    },
  });
  await page.goto("/#/bench");
  const screen = page.getByTestId("bench-screen");
  await expect(screen).toBeVisible();

  const key = screen.getByRole("button", { name: "Color key" });
  await key.click();
  const popover = page.getByRole("dialog", { name: "Color key" });
  await expect(
    popover.getByRole("heading", { name: "Item types" }),
  ).toBeVisible();
  const box = await popover.boundingBox();
  expect(box).not.toBeNull();
  expect(box?.x ?? -1).toBeGreaterThanOrEqual(0);
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(420);
  expect((box?.y ?? 0) + (box?.height ?? 0)).toBeLessThanOrEqual(640);
  expect(
    await popover.evaluate((el) => el.scrollHeight > el.clientHeight),
  ).toBe(true);
  await page.keyboard.press("Escape");
  await expect(popover).toHaveCount(0);
  await expect(key).toBeFocused();
});

test("Settings has the same key under Color key", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-color-key").click();
  const panel = page.getByTestId("settings-color-key");
  await expect(panel.getByRole("heading", { name: "Color key" })).toBeVisible();
  for (const name of HEADINGS) {
    await expect(panel.getByRole("heading", { name })).toBeVisible();
  }
});
