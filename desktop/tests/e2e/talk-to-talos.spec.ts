import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";
import {
  ADMITTED_PIN,
  HUDDLE_ID,
  HUDDLE_SEED,
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
];

for (const [title, talosPubkey, why] of [
  ["with no pin", undefined, /this build has no Talos key/],
  ["with a pin that isn't a member", UNADMITTED_PIN, /isn't a member/],
] as const) {
  test(`${title} it's hidden, and the shortcut and tray say why`, async ({
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
    await page.evaluate(() =>
      window.dispatchEvent(new Event("buzz:talk-to-talos")),
    );
    await expect(page.getByText(why)).toHaveCount(seen + 1);

    await expect(page.getByTestId("talk-to-talos")).toHaveCount(0);
    expect(await logged(page, ...WORK)).toEqual([]);
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

  await expect(page.getByText(/Talos was asked to join/)).toBeVisible();
  const [talk, dm, start, ...rest] = await logged(page, ...WORK);
  const context = talk[1].context as Record<string, string>;
  expect(context.route).toBe("channels");
  expect(context.snapshot).toContain("Welcome");
  expect(dm).toEqual(["open_dm", { pubkeys: [ADMITTED_PIN] }]);
  expect(start[0]).toBe("start_huddle");
  expect(start[1].memberPubkeys).toEqual([]);
  expect(start[1].parentChannelId).not.toBe(context.channel_id);
  expect(rest).toEqual([]);
});

test("inside a huddle it adds Talos alone, once, and then says it's there", async ({
  page,
}) => {
  await installMockBridge(page, {
    talosPubkey: ADMITTED_PIN,
    huddle: HUDDLE_SEED,
  });
  await page.goto("/");
  const added = [["talk_to_talos"], ["add_talos_to_huddle"]];
  const commands = async () =>
    (await logged(page, ...WORK)).map(([command]) => [command]);

  await page.getByTestId("talk-to-talos").click();
  await expect.poll(commands).toEqual(added);

  await page.keyboard.press(SHORTCUT);
  await expect(
    page.getByText("Talos is already in this huddle."),
  ).toBeVisible();
  expect(await commands()).toEqual(added);
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

test("on the Bench it sends the card, its slashed id as a short id", async ({
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
      '<article data-bench-item="steward/hold-talos-7">Ship it?</article>',
    ),
  );
  await page.getByTestId("talk-to-talos").click();

  await expect(page.getByText("stop here")).toBeVisible();
  const [talk] = await logged(page, ...WORK);
  expect(talk[1].context).toMatchObject({
    route: "bench",
    card_id: "steward:hold-talos-7",
    snapshot: "Ship it?",
  });
});

test("the room's quick-add adds Talos and then goes away", async ({ page }) => {
  await installMockBridge(page, {
    talosPubkey: ADMITTED_PIN,
    windowLabel: `huddle-${HUDDLE_ID}`,
    huddle: HUDDLE_SEED,
  });
  await page.goto("/");
  const quickAdd = page.getByTestId("talos-quick-add");
  await expect(quickAdd).toHaveText("Add Talos");

  await quickAdd.click();

  await expect(quickAdd).toHaveCount(0);
  expect(await logged(page, "add_talos_to_huddle")).toHaveLength(1);
});
