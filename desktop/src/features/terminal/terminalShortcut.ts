import { isMacPlatform } from "@/shared/lib/platform";

/** The keys that open and close the terminal, as this platform writes them. */
export function terminalShortcutLabel(isMac = isMacPlatform()): string {
  return isMac ? "⌘J" : "Ctrl+J";
}
