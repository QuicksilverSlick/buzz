import { expect, type Page, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// An agent someone else runs, shared with everyone in #general.
const COACH_PUBKEY = "c0".repeat(32);
// An agent this desktop runs, in #general, answering anyone.
const HELPER_PUBKEY = "c1".repeat(32);

function autocomplete(page: Page) {
  return page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete");
}

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

test("the @ list shows an agent's public description on one line", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: COACH_PUBKEY,
        ownerPubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "Reset Coach",
        channelNames: ["general"],
        respondTo: "anyone",
        status: "online",
      },
    ],
    searchProfiles: [
      {
        pubkey: COACH_PUBKEY,
        displayName: "Reset Coach",
        about: "Plans your week.\nAsk me for a check-in.",
        ownerPubkey: TEST_IDENTITIES.charlie.pubkey,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("Ask @Reset");
  const row = autocomplete(page).getByTestId(
    `mention-suggestion-${COACH_PUBKEY}`,
  );
  await expect(row.getByTestId("mention-agent-description")).toHaveText(
    "Plans your week. Ask me for a check-in.",
  );
  await expect(
    row.getByRole("button", { name: "Mention Reset Coach" }),
  ).toHaveAccessibleDescription("Plans your week. Ask me for a check-in.");
  // It answers anyone, so there is no note about who it answers.
  await expect(row.getByTestId("mention-agent-audience-note")).toHaveCount(0);
});

test("agents that answer anyone carry a neutral globe, not a warning", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: HELPER_PUBKEY,
        name: "Helper",
        status: "running",
        channelNames: ["general"],
        respondTo: "anyone",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Happy to help with your plan.",
      pubkey,
    });
  }, HELPER_PUBKEY);

  const badge = page.getByRole("img", {
    name: "Anyone can send instructions to this agent",
  });
  await expect(badge).toBeVisible();
  await expect(
    badge.getByTestId("message-respond-to-anyone-icon"),
  ).toBeVisible();
  await expect(badge.locator(".lucide-triangle-alert")).toHaveCount(0);
});
