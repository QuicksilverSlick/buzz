import { expect, test } from "@playwright/test";

import { GENERAL_WELCOME_EVENT_ID } from "@/testing/e2eSeedIds";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";
import {
  ADMITTED_PIN,
  GENERAL_CHANNEL_ID,
  HUDDLE_ID,
  HUDDLE_SEED,
  installTerminalBackend,
  logged,
  UNADMITTED_PIN,
} from "./fixtures/talos";

// Huddle voices, slice 10a. Hidden until the pinned key is a member (setup step
// S4), and every way in says why while it can't work.
const SHORTCUT = "ControlOrMeta+Shift+KeyT";
const WORK = [
  "talk_to_talos",
  "open_dm",
  "start_huddle",
  "add_talos_to_huddle",
  // Never these: they add the parent as a bot, and a bot turns speech-to-text on.
  "add_agent_to_huddle",
  "sync_agents_to_active_huddle",
];

for (const [title, talosPubkey, why] of [
  ["with no pin", undefined, /no Talos key\..*owner finishes setup/],
  [
    "with a pin that isn't a member",
    UNADMITTED_PIN,
    /isn't a member.*owner finishes setup/,
  ],
] as const) {
  test(`${title} it's hidden, and the shortcut and tray event say why`, async ({
    page,
  }) => {
    await installMockBridge(page, { talosPubkey });
    await page.goto("/");
    await expect(page.getByTestId("channel-general")).toBeVisible();

    await expect(async () => {
      await page.keyboard.press(SHORTCUT); // Again until the member read settles.
      await expect(page.getByText(why).first()).toBeVisible({ timeout: 500 });
    }).toPass();
    const seen = await page.getByText(why).count();
    await page.keyboard.press("ControlOrMeta+KeyT"); // No Shift: not the chord.
    await page.evaluate(() =>
      window.dispatchEvent(new Event("buzz:talk-to-talos")),
    );
    await expect(page.getByText(why)).toHaveCount(seen + 1);

    await expect(page.getByTestId("talk-to-talos")).toHaveCount(0);
    await openSettings(page);
    await expect(page.getByTestId("talk-to-talos")).toHaveCount(0);
    expect(await logged(page, ...WORK)).toEqual([]);
  });

  test(`${title} the room has no quick-add`, async ({ page }) => {
    await installMockBridge(page, {
      talosPubkey,
      windowLabel: `huddle-${HUDDLE_ID}`,
      huddle: HUDDLE_SEED,
    });
    await page.goto("/");
    await expect(page.getByTestId("huddle-participant-strip")).toBeVisible();
    await expect(page.getByTestId("talos-quick-add")).toHaveCount(0);
  });
}

test("with no huddle it signs /df/talk first, then rings only the Talos DM", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      value: async () =>
        new AudioContext().createMediaStreamDestination().stream,
    });
  });
  await installMockBridge(page, { talosPubkey: ADMITTED_PIN });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("message-timeline")).toContainText("Welcome");
  await expect(page.getByTestId("talk-to-talos")).toBeVisible();
  expect(await logged(page, ...WORK)).toEqual([]); // Nothing before a click.

  await page.getByTestId("talk-to-talos").click();

  await expect(
    page.getByText(/Talos got the text on this screen/),
  ).toBeVisible();
  const [talk, dm, start, ...rest] = await logged(page, ...WORK);
  const context = talk[1].context as Record<string, string>;
  expect(context.route).toBe("channels");
  expect(context.channel_id).toBe(GENERAL_CHANNEL_ID);
  expect(context.snapshot).toContain("Welcome");
  expect(dm).toEqual(["open_dm", { pubkeys: [ADMITTED_PIN] }]);
  expect(start[0]).toBe("start_huddle");
  expect(start[1].memberPubkeys).toEqual([]);
  expect(start[1].parentChannelId).not.toBe(context.channel_id);
  expect(rest).toEqual([]);
});

test("while a mic prompt holds it up, it says so and starts nothing twice", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      value: () =>
        new Promise((resolve) => {
          (window as unknown as Record<string, unknown>).__ANSWER_MIC__ = () =>
            resolve(new AudioContext().createMediaStreamDestination().stream);
        }),
    });
  });
  await installMockBridge(page, { talosPubkey: ADMITTED_PIN });
  await page.goto("/");
  const button = page.getByTestId("talk-to-talos");
  const answerIt = /if a mic prompt is open, answer it/;

  await button.click();

  await expect.poll(() => logged(page, "start_huddle")).toHaveLength(1);
  await expect(button).toHaveAttribute("aria-busy", "true");
  await expect(button).toHaveAttribute("title", answerIt);
  await button.hover(); // Times out unless a pointer can reach the title.
  await page.keyboard.press(SHORTCUT);
  await expect(page.getByText(answerIt)).toBeVisible();
  expect(await logged(page, "talk_to_talos")).toHaveLength(1);

  await page.evaluate(() =>
    (window as unknown as { __ANSWER_MIC__: () => void }).__ANSWER_MIC__(),
  );
  await expect(button).toHaveAttribute("aria-busy", "false");
  expect(await logged(page, ...WORK)).toHaveLength(3);
});

test("inside a huddle it adds Talos alone, and a second try goes out again", async ({
  page,
}) => {
  await installMockBridge(page, {
    talosPubkey: ADMITTED_PIN,
    huddle: HUDDLE_SEED,
  });
  await page.goto("/");
  const button = page.getByTestId("talk-to-talos");
  const added = [["talk_to_talos"], ["add_talos_to_huddle"]];
  const commands = async () =>
    (await logged(page, ...WORK)).map(([command]) => [command]);

  await button.click();
  await expect.poll(commands).toEqual(added);
  // Home has no thread, card or timeline, so no text went and none is claimed.
  await expect(page.getByText(/^Talos was asked to join/)).toBeVisible();
  await expect(button).toHaveAttribute("aria-busy", "false");

  // Membership isn't presence: an offline Talos stays a member, so this must retry.
  await page.keyboard.press(SHORTCUT);
  await expect.poll(commands).toEqual([...added, ...added]);
});

test("from Settings a refusal is a sentence, nothing starts, and no text is sent", async ({
  page,
}) => {
  const refusal = "Talos isn't set up for you yet.";
  await installMockBridge(page, {
    talosPubkey: ADMITTED_PIN,
    talkToTalosError: refusal,
  });
  await page.goto("/");
  await openSettings(page);

  await page.getByTestId("talk-to-talos").click();

  await expect(page.getByText(refusal)).toBeVisible();
  const [talk, ...rest] = await logged(page, ...WORK);
  expect(talk[1].context).toMatchObject({ route: "settings", snapshot: null });
  expect(rest).toEqual([]);
});

test("on the Bench it sends the card, normalized as Talos measures it, its slashed id as a short id", async ({
  page,
}) => {
  await installMockBridge(page, {
    talosPubkey: ADMITTED_PIN,
    talkToTalosError: "stop here",
  });
  await page.goto("/#/bench");
  await page.evaluate(() =>
    document.body.insertAdjacentHTML(
      "beforeend",
      '<article data-bench-item="steward/hold-talos-7">Ship 1½​ days?</article>',
    ),
  );
  await page.getByTestId("talk-to-talos").click();

  await expect(page.getByText("stop here")).toBeVisible();
  const [talk] = await logged(page, ...WORK);
  expect(talk[1].context).toMatchObject({
    route: "bench",
    card_id: "steward:hold-talos-7",
    snapshot: "Ship 11⁄2 days?",
  });
});

test("in a thread it sends the thread, not the timeline behind it", async ({
  page,
}) => {
  await installMockBridge(page, {
    talosPubkey: ADMITTED_PIN,
    talkToTalosError: "stop here",
  });
  await page.goto(
    `/#/channels/${GENERAL_CHANNEL_ID}?messageId=${GENERAL_WELCOME_EVENT_ID}&thread=${GENERAL_WELCOME_EVENT_ID}`,
  );
  const behind = "checking in"; // A timeline message, outside this thread.
  await expect(page.getByTestId("message-timeline")).toContainText(behind);
  await expect(page.getByTestId("message-thread-body")).toContainText(
    "Welcome",
  );

  await page.getByTestId("talk-to-talos").click();

  await expect(page.getByText("stop here")).toBeVisible();
  const [talk] = await logged(page, ...WORK);
  const context = talk[1].context as Record<string, string>;
  expect(context.channel_id).toBe(GENERAL_CHANNEL_ID);
  expect(context.thread_root).toBe(GENERAL_WELCOME_EVENT_ID);
  expect(context.snapshot).toContain("Welcome");
  expect(context.snapshot).not.toContain(behind);
});

test("in the terminal the chord stays the shell's ^T", async ({ page }) => {
  await installTerminalBackend(page);
  await installMockBridge(page, { talosPubkey: ADMITTED_PIN });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("talk-to-talos")).toBeVisible();
  await page.keyboard.press("Meta+j");
  await expect(
    page.locator('[data-terminal-owner="terminal"] textarea'),
  ).toBeFocused();

  await page.keyboard.press(SHORTCUT);

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as Record<string, unknown>).__TERMINAL_INPUTS__,
      ),
    )
    .toContain("\u0014");
  await page.waitForTimeout(250); // Time for a wrongly started talk to show up.
  expect(await logged(page, ...WORK)).toEqual([]);
});

test("the room's quick-add sits beside the tiles, adds Talos and then goes away", async ({
  page,
}) => {
  await page.setViewportSize({ width: 720, height: 520 }); // The room's minimum.
  const { tyler, alice, charlie, outsider } = TEST_IDENTITIES;
  await installMockBridge(page, {
    talosPubkey: ADMITTED_PIN,
    windowLabel: `huddle-${HUDDLE_ID}`,
    huddle: {
      ...HUDDLE_SEED,
      members: [tyler, alice, charlie, outsider].map(({ pubkey }) => ({
        pubkey,
        role: "member" as const,
      })),
    },
  });
  await page.goto("/");
  const quickAdd = page.getByTestId("talos-quick-add");
  await expect(quickAdd).toHaveText("Add Talos");
  await expect(page.getByTestId("huddle-participant-tile")).toHaveCount(4);
  const stripEnd = await page
    .getByTestId("huddle-participant-strip")
    .evaluate((strip) => strip.getBoundingClientRect().right);
  const buttonStart = await quickAdd.evaluate(
    (button) => button.getBoundingClientRect().left,
  );
  expect(stripEnd).toBeLessThanOrEqual(buttonStart);

  await quickAdd.click();

  await expect(quickAdd).toHaveCount(0);
  expect(await logged(page, "add_talos_to_huddle")).toHaveLength(1);
});
