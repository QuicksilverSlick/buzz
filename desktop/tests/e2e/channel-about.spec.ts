import { expect, type Page, test } from "@playwright/test";
import { GENERAL_WELCOME_EVENT_ID } from "@/testing/e2eSeedIds";

import { installMockBridge } from "../helpers/bridge";

// The mock bridge seeds canvas revisions on the starter #general channel.
const CANVAS = "# Start here\n\nAsk the Reset Worker when you need a hand.";
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const GENERAL_THREAD_ROOT_ID = GENERAL_WELCOME_EVENT_ID;
// The mock's first project uses the "buzz" channel as its home.
const PROJECT_HOME_CHANNEL_ID = "cf63feec-21bb-5bf0-a2f8-0e4c3de8ec73";

async function openChannel(page: Page, name: string) {
  await page.getByTestId(`channel-${name}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(name);
}

test("About this channel opens by itself once, then from its header button", async ({
  page,
}) => {
  await installMockBridge(page, { canvasRevisions: [{ content: CANVAS }] });
  await page.goto("/");
  await openChannel(page, "general");

  const about = page.getByTestId("channel-about-panel");
  const trigger = page.getByTestId("channel-about-trigger");
  await expect(about).toContainText("Start here");
  await expect(
    page.getByRole("heading", { name: "About this channel" }),
  ).toBeVisible();
  await expect(trigger).toHaveAccessibleName("About this channel");
  await expect(trigger).toHaveAttribute("aria-pressed", "true");
  // Read-only, even for someone who can edit the canvas in channel settings.
  await expect(about.getByTestId("channel-canvas-edit")).toHaveCount(0);
  await expect(about.getByTestId("channel-canvas-editor")).toHaveCount(0);

  await trigger.click();
  await expect(about).toHaveCount(0);
  await expect(trigger).toHaveAttribute("aria-pressed", "false");

  // A channel without a canvas has no button.
  await openChannel(page, "random");
  await expect(trigger).toHaveCount(0);
  await expect(about).toHaveCount(0);

  // Coming back, or restarting the app, does not open it again by itself.
  await openChannel(page, "general");
  await expect(trigger).toBeVisible();
  await expect(about).toHaveCount(0);
  await page.reload();
  await openChannel(page, "general");
  await expect(trigger).toBeVisible();
  await expect(about).toHaveCount(0);

  await trigger.click();
  await expect(about).toContainText("Start here");
  await expect(trigger).toHaveAttribute("aria-pressed", "true");
});

test("About and channel settings take turns in the side panel", async ({
  page,
}) => {
  await installMockBridge(page, { canvasRevisions: [{ content: CANVAS }] });
  await page.goto("/");
  await openChannel(page, "general");

  const about = page.getByTestId("channel-about-panel");
  const trigger = page.getByTestId("channel-about-trigger");
  const settings = page.getByTestId("channel-management-sheet");
  await expect(about).toBeVisible();

  await page.getByTestId("channel-management-trigger").click();
  await expect(settings).toBeVisible();
  await expect(about).toHaveCount(0);
  await expect(trigger).toHaveAttribute("aria-pressed", "false");

  await trigger.click();
  await expect(about).toContainText("Start here");
  await expect(settings).toHaveCount(0);

  await page.getByTestId("auxiliary-panel-close").click();
  await expect(about).toHaveCount(0);
  await expect(settings).toHaveCount(0);
});

test("a link that opens a thread is not covered by About", async ({ page }) => {
  await installMockBridge(page, { canvasRevisions: [{ content: CANVAS }] });
  await page.goto(
    `/#/channels/${GENERAL_CHANNEL_ID}?messageId=${GENERAL_THREAD_ROOT_ID}&thread=${GENERAL_THREAD_ROOT_ID}`,
    { waitUntil: "domcontentloaded" },
  );

  const about = page.getByTestId("channel-about-panel");
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await expect(page.getByTestId("channel-about-trigger")).toBeVisible();
  await expect(about).toHaveCount(0);

  // It still counts as unseen, so the next plain visit opens it.
  await openChannel(page, "random");
  await openChannel(page, "general");
  await expect(about).toContainText("Start here");
});

test("a narrow window keeps the messages and leaves About to the button", async ({
  page,
}) => {
  await page.setViewportSize({ height: 720, width: 820 });
  await installMockBridge(page, { canvasRevisions: [{ content: CANVAS }] });
  await page.goto("/");
  await openChannel(page, "general");

  const about = page.getByTestId("channel-about-panel");
  const trigger = page.getByTestId("channel-about-trigger");
  await expect(trigger).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toBeVisible();
  await expect(about).toHaveCount(0);

  await trigger.click();
  await expect(about).toContainText("Start here");
});

test("a project's home channel leaves its command list to the About button", async ({
  page,
}) => {
  // Wide enough for About to fit beside the project overview and messages.
  await page.setViewportSize({ height: 1080, width: 1920 });
  await installMockBridge(page, {
    canvasRevisions: [
      {
        channelId: PROJECT_HOME_CHANNEL_ID,
        content: "# Project Channel: buzz\n\n| Action | Command |",
      },
    ],
  });
  await page.goto("/");
  await openChannel(page, "buzz");

  const about = page.getByTestId("channel-about-panel");
  const trigger = page.getByTestId("channel-about-trigger");
  // Known as a project home, and the canvas has loaded.
  await expect(
    page.getByTestId("chat-header").getByTestId("project-channel-icon"),
  ).toBeVisible();
  await expect(trigger).toBeVisible();
  await expect(trigger).toHaveAttribute("aria-pressed", "false");
  await expect(about).toHaveCount(0);

  await trigger.click();
  await expect(about).toContainText("Project Channel");
});

test("channels without a canvas show no About button and open nothing", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openChannel(page, "general");

  await expect(page.getByTestId("message-timeline")).toBeVisible();
  await expect(page.getByTestId("channel-about-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-about-panel")).toHaveCount(0);
});
