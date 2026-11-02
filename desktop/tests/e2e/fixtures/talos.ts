import type { Page } from "@playwright/test";

import { TEST_IDENTITIES } from "../../helpers/bridge";

// The mock relay's kind 13534 lists tyler, alice and bob. So Bob stands in for an
// admitted Talos key, and "outsider" for a pinned key before setup step S4.
export const ADMITTED_PIN = TEST_IDENTITIES.bob.pubkey;
export const UNADMITTED_PIN = TEST_IDENTITIES.outsider.pubkey;
export const HUDDLE_ID = "11111111-1111-4111-8111-111111111111";
export const HUDDLE_SEED = {
  parentChannelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50", // #general
  ephemeralChannelId: HUDDLE_ID,
  members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "owner" as const }],
};

/** The bridge stands in at the invoke boundary. talos.rs tests the real request. */
export function logged(page: Page, ...commands: string[]) {
  return page.evaluate(
    (wanted) =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
        .filter(({ command }) => wanted.includes(command))
        .map(
          ({ command, payload }) =>
            [command, payload as Record<string, unknown>] as const,
        ),
    commands,
  );
}
