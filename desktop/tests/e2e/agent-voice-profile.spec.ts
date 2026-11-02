import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// Huddle voices, slice 9: the voice lives in Agents > card > Edit > Advanced,
// is off by default, and persists with the definition.
const PERSONA_NAME = "Voice E2E Persona";

async function openAdvanced(page: Page) {
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: `${PERSONA_NAME} agent profile` })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("persona-dialog")).toBeVisible();
  await page.getByRole("button", { name: "Advanced", exact: true }).click();
}

test("the huddle voice is off by default and persists with the definition", async ({
  page,
}) => {
  await installMockBridge(page, {
    globalAgentConfig: { env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" } },
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.tyler.pubkey,
        name: "Tyler Agent",
        personaId: "persona-voice-e2e",
        status: "stopped",
        channelNames: ["agents"],
      },
    ],
    personas: [
      {
        id: "persona-voice-e2e",
        displayName: PERSONA_NAME,
        systemPrompt: "You are the voice e2e persona.",
        runtime: "goose",
        model: "claude-opus-4-5",
        provider: "anthropic",
      },
    ],
  });
  await page.goto("/");
  await openAdvanced(page);

  const voiceSwitch = page.locator("#persona-voice-enabled");
  await expect(voiceSwitch).toHaveAttribute("aria-checked", "false");
  await expect(page.locator("#persona-voice-voice")).toHaveCount(0);
  await voiceSwitch.click();
  await page.locator("#persona-voice-voice").fill("marin");
  await page.locator("#persona-voice-persona").fill("Warm and brief.");
  await page.locator("#persona-voice-aliases").fill("Orch, O");
  await page.getByTestId("persona-dialog-submit").click();
  await expect(page.getByTestId("persona-dialog")).toHaveCount(0);

  // The mock store holds what the backend would persist.
  const stored = await page.evaluate(async () => {
    const invoke = (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (c: string) => Promise<unknown>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    const personas = (await invoke?.("list_personas")) as
      | Array<{ id: string; voice_profile?: unknown }>
      | undefined;
    return personas?.find((p) => p.id === "persona-voice-e2e")?.voice_profile;
  });
  expect(stored).toEqual({
    enabled: true,
    voice: "marin",
    persona: "Warm and brief.",
    aliases: ["Orch", "O"],
    voice_without_owner: false,
  });

  // Reopen: the saved voice reads back into the dialog.
  await openAdvanced(page);
  await expect(voiceSwitch).toHaveAttribute("aria-checked", "true");
  await expect(page.locator("#persona-voice-voice")).toHaveValue("marin");
  await expect(page.locator("#persona-voice-aliases")).toHaveValue("Orch, O");
});
