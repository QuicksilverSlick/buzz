import type { Page } from "@playwright/test";

import { TEST_IDENTITIES } from "../../helpers/bridge";

// The mock relay's kind 13534 lists tyler, alice and bob. So Bob stands in for an
// admitted Talos key, and "outsider" for a pinned key before setup step S4.
export const ADMITTED_PIN = TEST_IDENTITIES.bob.pubkey;
export const UNADMITTED_PIN = TEST_IDENTITIES.outsider.pubkey;
export const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
export const HUDDLE_ID = "11111111-1111-4111-8111-111111111111";
export const HUDDLE_SEED = {
  parentChannelId: GENERAL_CHANNEL_ID,
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

/**
 * Just enough terminal backend to open the real terminal and read what it sends
 * the shell. The mock bridge throws on `terminal_*`, as terminal-wheel.spec notes.
 */
export async function installTerminalBackend(page: Page) {
  await page.addInitScript(() => {
    type Invoke = (cmd: string, args: unknown, opts: unknown) => unknown;
    const w = window as unknown as Record<string, unknown>;
    const inputs: unknown[] = [];
    const viewport = { generation: 1, columns: 80, screenLines: 24 };
    const internals = {};
    let inner: Invoke | undefined;
    Object.defineProperty(internals, "invoke", {
      get: () => (cmd: string, args: { data?: string }, opts: unknown) => {
        if (!cmd.startsWith("terminal_")) return inner?.(cmd, args, opts);
        if (cmd === "terminal_input") inputs.push(args.data);
        if (cmd === "terminal_resize") return Promise.resolve(viewport);
        if (cmd !== "terminal_attach") return Promise.resolve(null);
        return Promise.resolve({
          sessionId: "talos-session",
          subscriptionId: "talos-sub",
          viewport,
        });
      },
      set: (fn: Invoke) => {
        inner = fn;
      },
    });
    w.isTauri = true;
    w.__TAURI_INTERNALS__ = internals;
    w.__TERMINAL_INPUTS__ = inputs;
  });
}
