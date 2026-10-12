import { expect, type Page, test } from "@playwright/test";

import { PRODUCT_NAME } from "../../src/shared/constants/brand";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";

// Community deep links that arrive before machine onboarding complete are
// drained from Rust into a persisted transaction and acknowledged immediately.
// Invite claiming waits until setup finishes and the final identity is known.

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const COMMUNITY_ONBOARDING_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const TRANSACTION_STORAGE_KEY = "buzz-community-onboarding-transaction.v1";
const COMMUNITY_RELAY_URL = "wss://hive.example.com";

const PENDING_JOIN_LINK = {
  id: "dl-join-1",
  kind: "join" as const,
  relayUrl: "wss://hive.example.com",
  code: "abc.def",
};

const PENDING_CONNECT_LINK = {
  id: "dl-connect-1",
  kind: "connect" as const,
  relayUrl: "wss://hive.example.com",
  code: null,
};

const PENDING_ADD_COMMUNITY_LINK = {
  id: "dl-add-community-1",
  kind: "add-community" as const,
  relayUrl: "wss://acme.communities.buzz.xyz",
  code: null,
  name: "Acme Team",
};

const SECOND_PENDING_ADD_COMMUNITY_LINK = {
  id: "dl-add-community-2",
  kind: "add-community" as const,
  relayUrl: "wss://beta.communities.buzz.xyz",
  code: null,
  name: "Beta Team",
};

test("join deep link is acknowledged without claiming before setup", async ({
  page,
}) => {
  let claimCalls = 0;
  await page.route("**/api/invites/claim", async (route) => {
    claimCalls++;
    await route.abort();
  });
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_JOIN_LINK] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Opening community link" }),
  ).toBeVisible();
  await page.getByTestId("pending-invite-continue").click();
  await expect(gate).toHaveCount(0);
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  expect(claimCalls).toBe(0);
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"stage":"claiming"');
});

test("connect deep link shows a static acknowledgment during setup", async ({
  page,
}) => {
  // No invite code means nothing to confirm against the relay — the gate
  // acknowledges the link and waits for the user instead of auto-advancing.
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_CONNECT_LINK] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Opening community link" }),
  ).toBeVisible();
  await expect(gate).toContainText("hive");

  // Continue setup dismisses the gate but keeps the transaction: the
  // connect resumes in CommunityOnboardingFlow after machine setup.
  await page.getByTestId("pending-invite-continue").click();
  await expect(gate).toHaveCount(0);
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"acknowledged":true');
});

test("add-community deep link starts onboarding when no community is configured", async ({
  page,
}) => {
  // profileReadError forces the fallback path (error → profile step), so the
  // test asserts pre-existing-profile behavior without the default mock
  // identity's has_profile_event:true triggering the skip.
  await installMockBridge(
    page,
    {
      pendingCommunityDeepLinks: [PENDING_ADD_COMMUNITY_LINK],
      profileReadError: "no-kind-0",
    },
    { skipCommunitySeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("community-onboarding-flow")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Build your profile" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"source":"add-community"');
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"communityName":"Acme Team"');
});

test("add-community deep link skips profile step when identity has an existing kind:0 profile", async ({
  page,
}) => {
  // The default mock identity (deadbeef...) is pre-seeded with
  // has_profile_event:true. The skip should fire on connecting → clear the
  // transaction entirely, never showing the profile step.
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_ADD_COMMUNITY_LINK] },
    { skipCommunitySeed: true },
  );
  await page.goto("/");

  // Onboarding flow must disappear — the skip cleared the transaction.
  await expect(page.getByTestId("community-onboarding-flow")).toHaveCount(0);
  // handleCommunityOnboardingConnect already added the community when the
  // transaction reached "connecting", so the app lands in the full UI.
  await expect(page.getByTestId("sidebar-profile-avatar-button")).toBeVisible();
});

test("add-community deep link opens one editable prefill and acknowledges the queue", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_ADD_COMMUNITY_LINK] },
    { seedPreviewFeatures: true },
  );
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "Join an existing community" }),
  ).toBeVisible();
  const communityInput = page.getByLabel("Community URL or invite link");
  await expect(communityInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.relayUrl);
  await expect(page.getByLabel("Name")).toHaveCount(0);

  await page.getByRole("button", { name: "Close" }).click();
  await expect(
    page.getByRole("heading", { name: "Join an existing community" }),
  ).toHaveCount(0);

  await page.getByTestId("sidebar-profile-avatar-button").click();
  await page.getByTestId("community-switcher").click();
  await page.getByRole("menuitem", { name: "Add a community" }).click();
  await page.getByTestId("add-community-join").click();
  await expect(communityInput).toHaveValue("");

  const acknowledgements = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
      (entry) => entry.command === "acknowledge_pending_community_deep_link",
    ),
  );
  expect(acknowledgements).toEqual([
    {
      command: "acknowledge_pending_community_deep_link",
      payload: { id: PENDING_ADD_COMMUNITY_LINK.id },
    },
  ]);
});

test("queued add-community links open and acknowledge one at a time", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      pendingCommunityDeepLinks: [
        PENDING_ADD_COMMUNITY_LINK,
        SECOND_PENDING_ADD_COMMUNITY_LINK,
      ],
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/");

  const communityInput = page.getByLabel("Community URL or invite link");
  await expect(communityInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.relayUrl);

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
          .filter(
            (entry) =>
              entry.command === "acknowledge_pending_community_deep_link",
          )
          .map((entry) => entry.payload),
      ),
    )
    .toEqual([{ id: PENDING_ADD_COMMUNITY_LINK.id }]);

  await page.getByRole("button", { name: "Close" }).click();

  await expect(communityInput).toHaveValue(
    SECOND_PENDING_ADD_COMMUNITY_LINK.relayUrl,
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
          .filter(
            (entry) =>
              entry.command === "acknowledge_pending_community_deep_link",
          )
          .map((entry) => entry.payload),
      ),
    )
    .toEqual([
      { id: PENDING_ADD_COMMUNITY_LINK.id },
      { id: SECOND_PENDING_ADD_COMMUNITY_LINK.id },
    ]);
});

test("deleted public starter channels do not strand community onboarding", async ({
  page,
}) => {
  const starterError =
    "starter channels created but metadata not yet available";
  await seedActiveIdentity(page, TEST_IDENTITIES.tyler);
  await page.addInitScript(
    ({ pubkey, relayUrl, storageKey }) => {
      window.localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storageKey,
        JSON.stringify({
          id: "txn-deleted-starters-1",
          source: "deep-link-join",
          stage: "team-intro",
          relayUrl,
          communityName: "hive",
          communityId: "e2e-default-community",
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    {
      pubkey: COMMUNITY_ONBOARDING_PUBKEY,
      relayUrl: COMMUNITY_RELAY_URL,
      storageKey: TRANSACTION_STORAGE_KEY,
    },
  );
  await installMockBridge(
    page,
    { ensureStarterChannelsErrors: [starterError] },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page
    .getByRole("button", { name: `Take me to ${PRODUCT_NAME}` })
    .click();

  await expect(page.getByTestId("community-onboarding-flow")).toHaveCount(0);
  await expect(page).toHaveURL(/#\/channels\/[^/]+$/);
  await expect(page.getByTestId("chat-title")).toContainText("Welcome");
  await expect(page.getByText(starterError)).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        window.__BUZZ_E2E_COMMANDS__?.filter(
          (command) => command === "ensure_starter_channels",
        ).length ?? 0,
    ),
  ).toBe(1);
});

test("required Welcome creation failure keeps community onboarding open", async ({
  page,
}) => {
  const welcomeError = "Channel creation is not permitted.";
  await seedActiveIdentity(page, TEST_IDENTITIES.tyler);
  await page.addInitScript(
    ({ pubkey, relayUrl, storageKey }) => {
      window.localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storageKey,
        JSON.stringify({
          id: "txn-welcome-failure-1",
          source: "deep-link-join",
          stage: "team-intro",
          relayUrl,
          communityName: "hive",
          communityId: "e2e-default-community",
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    {
      pubkey: COMMUNITY_ONBOARDING_PUBKEY,
      relayUrl: COMMUNITY_RELAY_URL,
      storageKey: TRANSACTION_STORAGE_KEY,
    },
  );
  await installMockBridge(
    page,
    { createChannelErrors: [welcomeError] },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page
    .getByRole("button", { name: `Take me to ${PRODUCT_NAME}` })
    .click();

  await expect(page.getByTestId("community-onboarding-flow")).toBeVisible();
  await expect(page.getByText(`${welcomeError} Try again.`)).toBeVisible();
  await expect(
    page.getByRole("button", { name: `Take me to ${PRODUCT_NAME}` }),
  ).toBeEnabled();
  await expect(page.getByTestId("chat-title")).toHaveCount(0);
});

test("persisted deep-link invite hands off to Joining after machine onboarding", async ({
  page,
}) => {
  // Deterministic claim failure (no real relay behind the mock bridge): the
  // spec asserts the handoff reaches the "Joining …" claiming screen, not
  // that the claim itself succeeds.
  await page.route("**/api/invites/claim", (route) => route.abort());
  await page.addInitScript(
    ({ pubkey, storageKey }) => {
      window.localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storageKey,
        JSON.stringify({
          id: "txn-deep-link-1",
          source: "deep-link-join",
          stage: "claiming",
          relayUrl: "wss://hive.example.com",
          inviteCode: "abc.def",
          communityName: "hive",
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    { pubkey: DEFAULT_MOCK_PUBKEY, storageKey: TRANSACTION_STORAGE_KEY },
  );
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  // Machine onboarding is complete, so the transaction owns the screen.
  await expect(page.getByTestId("community-onboarding-flow")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Joining hive" }),
  ).toBeVisible();
  await expect(page.getByTestId("pending-invite-gate")).toHaveCount(0);

  // The claim was attempted and its failure surfaced with a Retry.
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
});

// Guest mode: someone who joins a community through an invite skips
// AI-provider setup, gets no private Welcome channel, Welcome Team or
// kickoff, joins the owner's #general and #welcome-everyone, and lands in a
// channel the owner set up for them.

const GUEST_STORAGE_KEY = `buzz-community-guest.v1:${encodeURIComponent(
  COMMUNITY_RELAY_URL,
)}:${DEFAULT_MOCK_PUBKEY}`;
const GUEST_HINT_DONE_KEY = `buzz-guest-agent-hint-done.v1:${encodeURIComponent(
  COMMUNITY_RELAY_URL,
)}:${DEFAULT_MOCK_PUBKEY}`;
// The fixture's most recently active channel this member belongs to, other
// than #general and #welcome-everyone.
const GUEST_LANDING_CHANNEL_ID = "fa11bac0-0000-4000-8000-000000000012";
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const WELCOME_EVERYONE_CHANNEL_ID = "5f0b1b3c-2a37-5366-9b8c-31a4b21d8e77";
const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
// A key that belongs to no channel yet, like Craig's on a new Mac.
const NEW_GUEST = TEST_IDENTITIES.outsider;
const RESET_WORKER_PUBKEY = "ab".repeat(32);
const GUEST_HINT = "Type @ and an agent's name to ask it for something.";

async function commandCount(page: Page, command: string) {
  return page.evaluate(
    (target) =>
      window.__BUZZ_E2E_COMMANDS__?.filter((entry) => entry === target)
        .length ?? 0,
    command,
  );
}

async function joinedChannelIds(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
      .filter((entry) => entry.command === "join_channel")
      .map((entry) => (entry.payload as { channelId: string }).channelId),
  );
}

async function seedInviteTransaction(
  page: Page,
  stage: "claiming" | "profile",
  pubkey: string = DEFAULT_MOCK_PUBKEY,
) {
  await page.addInitScript(
    ({ pubkey, relayUrl, stage, storageKey }) => {
      window.localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storageKey,
        JSON.stringify({
          id: `txn-guest-${stage}`,
          source: "deep-link-join",
          stage,
          relayUrl,
          inviteCode: "abc.def",
          communityName: "hive",
          ...(stage === "profile"
            ? { communityId: "e2e-default-community" }
            : {}),
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    {
      pubkey,
      relayUrl: COMMUNITY_RELAY_URL,
      stage,
      storageKey: TRANSACTION_STORAGE_KEY,
    },
  );
}

async function expectGuestLanding(
  page: Page,
  landing = { channelId: GUEST_LANDING_CHANNEL_ID, name: "all-replies" },
) {
  await expect(page.getByTestId("community-onboarding-flow")).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`#/channels/${landing.channelId}$`));
  await expect(page.getByTestId("chat-title")).toHaveText(landing.name);
  await expect(page.getByTestId("channel-Welcome")).toHaveCount(0);
  // Guest mode is remembered for this community, whatever the identity.
  expect(
    await page.evaluate(
      (prefix) =>
        Object.keys(window.localStorage).filter(
          (key) =>
            key.startsWith(prefix) &&
            window.localStorage.getItem(key) === "true",
        ).length,
      GUEST_STORAGE_KEY.slice(0, GUEST_STORAGE_KEY.lastIndexOf(":") + 1),
    ),
  ).toBe(1);
  // Nothing is created in the guest's name: no private Welcome channel, no
  // starter channels, no Welcome Team, no kickoff. Joining the owner's
  // channels is fine.
  for (const command of [
    "create_channel",
    "ensure_starter_channels",
    "create_managed_agent",
    "send_managed_agent_channel_message",
  ]) {
    expect(await commandCount(page, command), command).toBe(0);
  }
}

function fulfillInviteClaim(page: Page) {
  return page.route("**/api/invites/claim", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "joined",
        community_id: "mock-community",
        host: "hive.example.com",
        role: "member",
      }),
    }),
  );
}

test("an invite link takes a new person past AI-provider setup to their channel", async ({
  page,
}) => {
  await fulfillInviteClaim(page);
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_JOIN_LINK], profileHasEvent: false },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("pending-invite-continue").click();
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByRole("button", { name: "Create my private key" }).click();
  await page.getByTestId("onboarding-next").click();

  // Straight from the identity key to the community, as "Set up later" would.
  await expect(
    page.getByRole("heading", { name: "Build your profile" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Connect your AI provider" }),
  ).toHaveCount(0);
  await page.getByTestId("community-profile-name-key").fill("Craig");
  await page.getByTestId("community-profile-next").click();

  await expectGuestLanding(page);
});

test("an invited guest skips the starter team and lands in their channel", async ({
  page,
}) => {
  await seedInviteTransaction(page, "profile");
  await installMockBridge(
    page,
    { profileHasEvent: false },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "Build your profile" }),
  ).toBeVisible();
  await page.getByTestId("community-profile-name-key").fill("Craig");
  await page.getByTestId("community-profile-next").click();

  await expectGuestLanding(page);
  await expect(
    page.getByRole("heading", { name: "Meet your starter team" }),
  ).toHaveCount(0);
});

test("a guest with a new key joins #general and #welcome-everyone and lands there", async ({
  page,
}) => {
  // Accepting an invite joins the community but none of its channels, and
  // nobody could add this key to a channel before it existed.
  await seedActiveIdentity(page, NEW_GUEST);
  await seedInviteTransaction(page, "profile", NEW_GUEST.pubkey);
  await installMockBridge(
    page,
    { profileHasEvent: false },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("community-profile-name-key").fill("Craig");
  await page.getByTestId("community-profile-next").click();

  await expectGuestLanding(page, {
    channelId: WELCOME_EVERYONE_CHANNEL_ID,
    name: "welcome-everyone",
  });
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(page.getByTestId("channel-welcome-everyone")).toBeVisible();
  // Joined once each, at entry; opening the app does not join them again.
  await expect(page.getByTestId("message-input")).toBeVisible();
  expect((await joinedChannelIds(page)).sort()).toEqual(
    [GENERAL_CHANNEL_ID, WELCOME_EVERYONE_CHANNEL_ID].sort(),
  );
});

test("starter channels a guest could not join at entry are joined when the app opens", async ({
  page,
}) => {
  await seedActiveIdentity(page, NEW_GUEST);
  await seedInviteTransaction(page, "profile", NEW_GUEST.pubkey);
  await installMockBridge(
    page,
    { joinChannelErrors: ["relay unavailable"], profileHasEvent: false },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("community-profile-name-key").fill("Craig");
  await page.getByTestId("community-profile-next").click();

  // #general's join failed at entry; #welcome-everyone's went through.
  await expectGuestLanding(page, {
    channelId: WELCOME_EVERYONE_CHANNEL_ID,
    name: "welcome-everyone",
  });
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect
    .poll(() => joinedChannelIds(page))
    .toEqual([
      GENERAL_CHANNEL_ID,
      WELCOME_EVERYONE_CHANNEL_ID,
      GENERAL_CHANNEL_ID,
    ]);
});

test("Next on the profile step says the community is opening while guest entry waits", async ({
  page,
}) => {
  await seedInviteTransaction(page, "profile");
  await installMockBridge(
    page,
    { channelsReadDelayMs: 1_500, profileHasEvent: false },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("community-profile-name-key").fill("Craig");
  await page.getByTestId("community-profile-next").click();

  const next = page.getByTestId("community-profile-next");
  await expect(next).toHaveAccessibleName("Opening your community");
  await expect(next).toBeDisabled();
  await expectGuestLanding(page);
});

test("an invited guest whose profile already exists waits on a locked form", async ({
  page,
}) => {
  // The default mock identity already has a kind:0 profile, so the profile
  // step enters by itself.
  await seedInviteTransaction(page, "profile");
  await installMockBridge(
    page,
    { channelsReadDelayMs: 1_500 },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("community-profile-name-key")).toBeDisabled();
  await expect(page.getByTestId("community-profile-next")).toHaveAccessibleName(
    "Opening your community",
  );
  await expectGuestLanding(page);
});

test("an invite link opened on the AI-provider step skips the rest of it", async ({
  page,
}) => {
  await fulfillInviteClaim(page);
  await installMockBridge(
    page,
    { profileHasEvent: false },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByRole("button", { name: "Create my private key" }).click();
  await page.getByTestId("onboarding-next").click();
  await expect(
    page.getByRole("heading", { name: "Connect your AI provider" }),
  ).toBeVisible();

  await page.evaluate(async (link) => {
    await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "e2e_queue_pending_community_deep_link",
      link,
    );
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("deep-link-join", null);
  }, PENDING_JOIN_LINK);

  await expect(
    page.getByRole("heading", { name: "Build your profile" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Connect your AI provider" }),
  ).toHaveCount(0);
});

test("an invited guest with an existing profile still lands as a guest", async ({
  page,
}) => {
  // The default mock identity already has a kind:0 profile, so the claim
  // skips the profile step and finishes onboarding right after connecting.
  await fulfillInviteClaim(page);
  await seedInviteTransaction(page, "claiming");
  await installMockBridge(page, undefined, {
    relayWsUrl: COMMUNITY_RELAY_URL,
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await expectGuestLanding(page);
});

test("a community admin who uses an invite keeps the starter team", async ({
  page,
}) => {
  await seedInviteTransaction(page, "profile");
  await installMockBridge(
    page,
    {
      profileHasEvent: false,
      relayRequiresMembership: true,
      relayRole: "admin",
    },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("community-profile-name-key").fill("Riley");
  await page.getByTestId("community-profile-next").click();

  await expect(
    page.getByRole("heading", { name: "Meet your starter team" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      (key) => window.localStorage.getItem(key),
      GUEST_STORAGE_KEY,
    ),
  ).toBeNull();
});

async function openChannelAsGuest(page: Page, name: string) {
  await page.getByTestId(`channel-${name}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(name);
}

async function openWelcomeEveryoneAsGuest(
  page: Page,
  mock: Parameters<typeof installMockBridge>[1],
) {
  await page.addInitScript((key) => {
    window.localStorage.setItem(key, "true");
  }, GUEST_STORAGE_KEY);
  await installMockBridge(page, mock, { relayWsUrl: COMMUNITY_RELAY_URL });
  await page.goto("/");
  await openChannelAsGuest(page, "welcome-everyone");
}

function guestHint(page: Page) {
  return page
    .getByTestId("welcome-composer-guide-banner")
    .getByTestId("welcome-composer-guest-hint");
}

function resetWorker(
  agent: Partial<{
    channelNames: string[];
    ownerPubkey: string;
    respondTo: "anyone" | "owner-only";
  }> = {},
) {
  return {
    pubkey: RESET_WORKER_PUBKEY,
    name: "Reset Worker",
    respondTo: "anyone" as const,
    channelNames: ["welcome-everyone"],
    ...agent,
  };
}

test("in #welcome-everyone a guest is told how to ask the community's agents", async ({
  page,
}) => {
  await openWelcomeEveryoneAsGuest(page, { relayAgents: [resetWorker()] });

  const banner = page.getByTestId("welcome-composer-guide-banner");
  await expect(guestHint(page)).toHaveText(GUEST_HINT);
  await expect(banner).not.toContainText("Fizz");
  await expect(
    page.getByTestId("welcome-composer-persona-mention"),
  ).toHaveCount(0);
  // The @ list offers the agent the hint is about.
  await page.getByTestId("message-input").fill("@Reset");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId(`mention-suggestion-${RESET_WORKER_PUBKEY}`),
  ).toBeVisible();
});

for (const { agent, title } of [
  {
    title: "answers anyone but is not in the channel",
    agent: resetWorker({ channelNames: ["random"] }),
  },
  {
    title: "answers only its owner",
    agent: resetWorker({
      ownerPubkey: TEST_IDENTITIES.charlie.pubkey,
      respondTo: "owner-only",
    }),
  },
]) {
  test(`a guest gets no hint where the only agent ${title}`, async ({
    page,
  }) => {
    await openWelcomeEveryoneAsGuest(page, { relayAgents: [agent] });

    // #general's agents answer anyone, so the hint shows there once the
    // agent lists have loaded...
    await openChannelAsGuest(page, "general");
    await expect(guestHint(page)).toHaveText(GUEST_HINT);
    // ...and still not in #welcome-everyone, where the @ list has none.
    await openChannelAsGuest(page, "welcome-everyone");
    await expect(page.getByTestId("message-input")).toBeVisible();
    await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
      0,
    );
  });
}

test("a guest is told how to ask agents in their own channel until they dismiss it", async ({
  page,
}) => {
  await openWelcomeEveryoneAsGuest(page, {
    relayAgents: [resetWorker({ channelNames: ["random"] })],
  });
  await openChannelAsGuest(page, "random");
  await expect(guestHint(page)).toHaveText(GUEST_HINT);

  await page.getByTestId("welcome-composer-dismiss-button").click();
  await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
    0,
  );
  expect(
    await page.evaluate(
      (key) => localStorage.getItem(key),
      GUEST_HINT_DONE_KEY,
    ),
  ).toBe("true");

  // Gone for good in this community: other channels, and after a restart.
  await openChannelAsGuest(page, "general");
  await expect(page.getByTestId("message-input")).toBeVisible();
  await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
    0,
  );
  await page.reload();
  await openChannelAsGuest(page, "random");
  await expect(page.getByTestId("message-input")).toBeVisible();
  await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
    0,
  );
});

test("asking an agent finishes the guest hint for good", async ({ page }) => {
  await openWelcomeEveryoneAsGuest(page, {
    relayAgents: [resetWorker({ channelNames: ["random"] })],
  });
  // The owner added the agent to the guest's channel.
  await page.evaluate(
    async ({ channelId, pubkey }) => {
      await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });
    },
    { channelId: RANDOM_CHANNEL_ID, pubkey: RESET_WORKER_PUBKEY },
  );
  await openChannelAsGuest(page, "random");
  await expect(guestHint(page)).toHaveText(GUEST_HINT);

  await page.getByTestId("message-input").fill("@Reset");
  await page
    .getByTestId("message-composer")
    .getByTestId(`mention-suggestion-${RESET_WORKER_PUBKEY}`)
    .click();
  await page.getByTestId("message-input").press("End");
  await page.getByTestId("message-input").pressSequentially(" plan my week");
  await page.getByTestId("message-input").press("Enter");

  await expect(page.getByTestId("welcome-composer-guide-banner")).toContainText(
    "Nice work.",
  );
  await expect
    .poll(() =>
      page.evaluate((key) => localStorage.getItem(key), GUEST_HINT_DONE_KEY),
    )
    .toBe("true");
  await page.reload();
  await openChannelAsGuest(page, "random");
  await expect(page.getByTestId("message-input")).toBeVisible();
  await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
    0,
  );
});

for (const { relayRole, staysGuest } of [
  { relayRole: "owner" as const, staysGuest: false },
  { relayRole: "admin" as const, staysGuest: false },
  { relayRole: "member" as const, staysGuest: true },
]) {
  test(`a saved guest whom the relay lists as ${relayRole} ${
    staysGuest ? "stays a guest" : "gets the normal experience back"
  }`, async ({ page }) => {
    // Guest entry decides on the invite alone when the role lookup doesn't
    // answer; the relay's later answer corrects an owner or admin.
    await openWelcomeEveryoneAsGuest(page, {
      relayAgents: [resetWorker()],
      relayRequiresMembership: true,
      relayRole,
    });

    if (staysGuest) {
      await expect(guestHint(page)).toHaveText(GUEST_HINT);
    } else {
      await expect(
        page.getByTestId("welcome-composer-persona-mention"),
      ).toBeVisible();
      await expect(guestHint(page)).toHaveCount(0);
    }
    expect(
      await page.evaluate(
        (key) => localStorage.getItem(key),
        GUEST_STORAGE_KEY,
      ),
    ).toBe(staysGuest ? "true" : null);
  });
}

test("a saved guest whom a slow relay lists as owner is corrected in the open channel", async ({
  page,
}) => {
  // Hold the relay's role answer so #welcome-everyone is already open when it
  // arrives; the open view must drop guest mode without a reload.
  await page.addInitScript(() => {
    const internals = window.__TAURI_INTERNALS__ ?? {};
    let wrapped: unknown;
    Object.defineProperty(internals, "invoke", {
      configurable: true,
      get: () => wrapped,
      set: (fn: (command: string, ...rest: unknown[]) => Promise<unknown>) => {
        wrapped = async (command: string, ...rest: unknown[]) => {
          if (command === "relay_requires_membership") {
            await new Promise((resolve) => setTimeout(resolve, 4_000));
          }
          return fn(command, ...rest);
        };
      },
    });
    window.__TAURI_INTERNALS__ = internals;
  });
  await openWelcomeEveryoneAsGuest(page, {
    relayAgents: [resetWorker()],
    relayRequiresMembership: true,
    relayRole: "owner",
  });
  expect(
    await page.evaluate((key) => localStorage.getItem(key), GUEST_STORAGE_KEY),
  ).toBe("true");

  await expect(
    page.getByTestId("welcome-composer-persona-mention"),
  ).toBeVisible({ timeout: 15_000 });
  await expect(guestHint(page)).toHaveCount(0);
  expect(
    await page.evaluate((key) => localStorage.getItem(key), GUEST_STORAGE_KEY),
  ).toBe(null);
});

test("in #welcome-everyone a guest who can see no agents gets no hint", async ({
  page,
}) => {
  // The fixture community has relay agents; an unreadable agent directory
  // leaves this guest with none they can see.
  await openWelcomeEveryoneAsGuest(page, {
    relayAgentListErrors: Array.from({ length: 50 }, () => "unavailable"),
  });

  await expect(page.getByTestId("message-input")).toBeVisible();
  // Let both agent sources answer before checking that no hint appeared.
  await expect
    .poll(async () =>
      Math.min(
        await commandCount(page, "list_relay_agents"),
        await commandCount(page, "list_managed_agents"),
      ),
    )
    .toBeGreaterThan(0);
  await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
    0,
  );
});
