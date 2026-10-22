import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const OUTDIR = "test-results/invites-settings";
const DIRECT_ADD_HEX =
  "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const DIRECT_ADD_NPUB =
  "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";
const SECOND_DIRECT_ADD_HEX =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECOND_DIRECT_ADD_NPUB =
  "npub1424242424242424242424242424242424242424242424242424qamrcaj";
const MOCK_VIEWER_PUBKEY = "deadbeef".repeat(8);
const MOCK_RELAY_HOST = new URL(
  process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000",
).host;

test.beforeEach(async ({ page }, testInfo) => {
  await installMockBridge(page, {
    relayRequiresMembership: true,
    relayRole: testInfo.title.includes("admin can add members")
      ? "admin"
      : "owner",
    ...(testInfo.title.includes("no member list")
      ? { relayMembershipSnapshot: false }
      : {}),
    ...(testInfo.title.includes("hosted owner")
      ? {
          builderlabAuth: { expiresAt: "2099-01-01T00:00:00Z" },
          builderlabCommunities: [
            {
              normalized_host: MOCK_RELAY_HOST,
              owner_pubkey: MOCK_VIEWER_PUBKEY,
            },
          ],
        }
      : {}),
  });
  await page.route("**/api/invites", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      json: {
        code: "community-email-test",
        expires_at: Math.floor(Date.now() / 1000) + 3 * 86_400,
        url: "https://alpha.example.com/invite/community-email-test",
      },
      status: 200,
    });
  });
  await page.goto("/");
  await openSettings(page, "community-members");
});

test("opens a profile from a community member avatar", async ({ page }) => {
  await page.getByRole("button", { name: "Open profile for alice" }).click();

  await expect(page).toHaveURL(
    new RegExp(`/pulse\\?profile=${TEST_IDENTITIES.alice.pubkey}$`),
  );
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
});

test("capture: consolidated invites settings", async ({ page }) => {
  const panel = page.getByTestId("settings-panel-community-members");

  await expect(
    page.getByTestId("settings-nav-community-members"),
  ).toContainText("Members");
  await expect(
    page.getByRole("heading", { name: "Members", exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("add-relay-member")).toHaveText("Add member");
  await expect(page.getByTestId("community-members-degraded")).toHaveCount(0);
  await expect(page.getByTestId("community-icon-settings")).toHaveCount(0);
  await expect(page.getByTestId("community-invite-dialog-trigger")).toHaveText(
    "Invite link",
  );
  await expect(page.getByTestId("community-invite-email-field")).toHaveCount(0);
  await expect(page.getByTestId("copy-invite-link")).toHaveCount(0);
  await expect(page.getByText("alice", { exact: true })).toBeVisible();
  await expect(page.getByText("bob", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Manage roles or remove access.", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("People who use the link join as members."),
  ).toHaveCount(0);
  await expect(page.getByTestId("community-icon-save")).toHaveCount(0);

  const aliceName = page.getByText("alice", { exact: true });
  const aliceRow = page
    .locator('[data-testid^="relay-member-row-"]')
    .filter({ has: aliceName });
  const aliceNpub = aliceRow.locator('[data-testid^="relay-member-npub-"]');
  await expect(aliceName).toHaveCSS("opacity", "1");
  await expect(aliceNpub).toHaveCSS("opacity", "0");
  await aliceRow.hover();
  await expect(aliceName).toHaveCSS("opacity", "0");
  await expect(aliceNpub).toHaveCSS("opacity", "1");
  await page.mouse.move(0, 0);

  await waitForAnimations(page);
  await panel.screenshot({ path: `${OUTDIR}/01-invites-settings.png` });
});

test("capture: share-style community invite dialog", async ({ page }) => {
  await page.getByTestId("community-invite-dialog-trigger").click();

  const dialog = page.getByTestId("community-invite-dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Invite to community" }),
  ).toBeVisible();
  await expect(page.getByTestId("community-invite-email-field")).toHaveCount(0);
  await expect(page.getByPlaceholder("Type an email address")).toHaveCount(0);
  await expect(
    dialog.getByText(
      "Add someone directly or share a link they can use to join.",
    ),
  ).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Add someone", exact: true }),
  ).toHaveCount(0);
  await expect(dialog.getByTestId("invite-options-divider")).toBeVisible();
  await expect(
    dialog.getByText("Or, copy a link", { exact: true }),
  ).toBeVisible();
  await expect(dialog.getByText("Link settings", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByTestId("member-pubkey-input")).toBeVisible();
  await expect(page.getByTestId("member-role")).toHaveCount(0);
  await expect(page.getByTestId("confirm-add-member")).toHaveCount(0);
  await expect(page.getByTestId("invite-link-url")).toHaveValue(
    "https://alpha.example.com/invite/community-email-test",
  );
  await expect(page.getByTestId("copy-invite-link")).toHaveText("Copy link");
  await expect(page.getByTestId("invite-link-ttl-trigger")).toHaveText(
    "3 days",
  );

  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await expect(page.getByTestId("member-search-popover")).toBeVisible();
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  const memberRole = page.getByTestId("member-role");
  const selectedChip = page.getByTestId(
    `member-search-selection-remove-${DIRECT_ADD_HEX}`,
  );
  await expect(memberRole).toHaveText("Member");
  const inviteButton = page.getByTestId("confirm-add-member");
  await expect(inviteButton).toHaveText("Invite");
  await waitForAnimations(page);
  await expect(inviteButton).toHaveCSS("height", "44px");
  await expect(inviteButton).toHaveJSProperty(
    "offsetHeight",
    await page
      .getByTestId("member-recipient-field")
      .evaluate((field) => Math.round(field.getBoundingClientRect().height)),
  );
  const selectedChipRemoveIcon = selectedChip.locator("span.absolute");
  await expect(selectedChipRemoveIcon).toHaveCSS("opacity", "0");
  await selectedChip.hover();
  await expect(selectedChipRemoveIcon).toHaveCSS("opacity", "1");
  const memberSearch = page.getByTestId("member-pubkey-input");
  await expect(memberSearch).toBeFocused();
  await memberSearch.fill(SECOND_DIRECT_ADD_NPUB);
  await expect(page.getByTestId("member-search-popover")).toBeVisible();
  await page
    .getByTestId(`member-search-result-${SECOND_DIRECT_ADD_HEX}`)
    .click();
  await expect(
    page.getByTestId(`member-search-selection-remove-${SECOND_DIRECT_ADD_HEX}`),
  ).toBeVisible();
  await expect(memberSearch).toBeFocused();
  await memberRole.click();
  await expect(
    page.getByRole("menuitemradio", { name: "Admin" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("confirm-add-member")).toBeEnabled();
  await waitForAnimations(page);
  await page.mouse.move(0, 0);
  await dialog.screenshot({ path: `${OUTDIR}/02-invite-dialog.png` });
});

test("admin can add members but cannot assign the admin role", async ({
  page,
}) => {
  await page.getByTestId("community-invite-dialog-trigger").click();

  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  const memberRole = page.getByTestId("member-role");
  await expect(memberRole).toHaveText("Member");
  await memberRole.click();
  await expect(page.getByRole("menuitemradio", { name: "Admin" })).toHaveCount(
    0,
  );
  await page.keyboard.press("Escape");
});

test("owner can add multiple admins directly by npub from live Invites UI", async ({
  page,
}) => {
  await page.getByTestId("community-invite-dialog-trigger").click();
  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  await page.getByTestId("member-pubkey-input").fill(SECOND_DIRECT_ADD_NPUB);
  await page
    .getByTestId(`member-search-result-${SECOND_DIRECT_ADD_HEX}`)
    .click();
  await page.getByTestId("member-role").click();
  await page.getByRole("menuitemradio", { name: "Admin" }).click();
  await page.getByTestId("confirm-add-member").click();

  await expect
    .poll(async () =>
      page.evaluate(
        ({ targetPubkeys, role }) =>
          targetPubkeys.every((targetPubkey) =>
            (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some((entry) => {
              if (entry.command !== "plugin:websocket|send") return false;
              const wireMessage = (
                entry.payload as {
                  message?: { data?: unknown };
                }
              )?.message?.data;
              if (typeof wireMessage !== "string") return false;
              const message = JSON.parse(wireMessage) as unknown[];
              if (message[0] !== "EVENT") return false;
              const event = message[1] as
                | { kind?: number; tags?: string[][] }
                | undefined;
              return (
                event?.kind === 9030 &&
                event.tags?.some(
                  (tag) => tag[0] === "p" && tag[1] === targetPubkey,
                ) &&
                event.tags.some((tag) => tag[0] === "role" && tag[1] === role)
              );
            }),
          ),
        {
          targetPubkeys: [DIRECT_ADD_HEX, SECOND_DIRECT_ADD_HEX],
          role: "admin",
        },
      ),
    )
    .toBe(true);
});

test("owner adds a member by npub from Add member", async ({ page }) => {
  await page.getByTestId("add-relay-member").click();
  const dialog = page.getByTestId("add-relay-member-dialog");
  await expect(dialog).toBeVisible();

  await dialog.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  await dialog.getByTestId("member-role").click();
  await expect(
    page.getByRole("menuitemradio", { name: "Admin" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await dialog.getByTestId("confirm-add-member").click();

  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByTestId(`relay-member-row-${DIRECT_ADD_HEX}`),
  ).toBeVisible();
});

test("owner removes a member via the ... menu with confirm", async ({
  page,
}) => {
  const bobRow = page.getByTestId(
    `relay-member-row-${TEST_IDENTITIES.bob.pubkey}`,
  );
  await expect(bobRow).toBeVisible();

  await page
    .getByTestId(`relay-member-actions-${TEST_IDENTITIES.bob.pubkey}`)
    .click();
  await page.getByRole("menuitem", { name: "Remove from community" }).click();
  const confirm = page.getByTestId("confirm-remove-member-dialog");
  await expect(confirm).toBeVisible();
  // Nothing is removed until the owner confirms.
  await expect(bobRow).toBeVisible();

  await confirm.getByTestId("confirm-remove-member").click();
  await expect(confirm).toHaveCount(0);
  await expect(bobRow).toHaveCount(0);
});

test("shows Members with a retry when the relay sends no member list", async ({
  page,
}) => {
  await expect(
    page.getByTestId("settings-nav-community-members"),
  ).toContainText("Members");
  const degraded = page.getByTestId("community-members-degraded");
  await expect(degraded).toHaveText(
    /This relay didn’t return its member list\.\s*Retry/,
  );
  await expect(page.getByTestId("community-members-retry")).toBeEnabled();
  await page.getByTestId("community-members-retry").click();
  await expect(degraded).toBeVisible();

  // With no role known, Add member still works but offers members only;
  // the relay decides whether the add is allowed.
  await page.getByTestId("add-relay-member").click();
  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  await page.getByTestId("member-role").click();
  await expect(page.getByRole("menuitemradio", { name: "Admin" })).toHaveCount(
    0,
  );
});

test("hosted owner keeps owner controls when the relay sends no member list", async ({
  page,
}) => {
  await expect(page.getByTestId("community-members-degraded")).toContainText(
    "Your hosted community account shows you as its owner.",
  );

  await page.getByTestId("add-relay-member").click();
  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  await page.getByTestId("member-role").click();
  await expect(
    page.getByRole("menuitemradio", { name: "Admin" }),
  ).toBeVisible();
});
