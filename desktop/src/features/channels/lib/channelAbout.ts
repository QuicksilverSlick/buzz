import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

/**
 * "About this channel" shows a channel's canvas, read-only, in the channel's
 * side panel. The header button appears only when the canvas has content, and
 * the panel opens by itself the first time this device shows such a channel.
 */

/** Channels this device has already shown the About panel for. */
const CHANNEL_ABOUT_SEEN_STORAGE_KEY = "buzz-channel-about-seen.v1";

/** The seen list keeps the most recent channels and drops the oldest. */
export const CHANNEL_ABOUT_SEEN_LIMIT = 500;

/**
 * Remembers this session's marks even when storage refuses the write, so a
 * blocked or full storage never reopens the panel on every visit. The next
 * launch may open it once more; that is the whole cost of a refused write.
 */
const seenThisSession = new Set<string>();

type ChannelCanvasSnapshot = { content: string | null } | null | undefined;

/** Whether a channel's canvas has anything worth showing. */
export function hasChannelCanvasContent(canvas: ChannelCanvasSnapshot) {
  return (canvas?.content?.trim().length ?? 0) > 0;
}

function readSeenChannelIds(): string[] {
  const raw = getStorageItem(CHANNEL_ABOUT_SEEN_STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return [];
  }
}

export function hasSeenChannelAbout(channelId: string) {
  return (
    seenThisSession.has(channelId) || readSeenChannelIds().includes(channelId)
  );
}

export function markChannelAboutSeen(channelId: string) {
  seenThisSession.add(channelId);
  const next = [
    ...readSeenChannelIds().filter((id) => id !== channelId),
    channelId,
  ].slice(-CHANNEL_ABOUT_SEEN_LIMIT);
  setStorageItem(CHANNEL_ABOUT_SEEN_STORAGE_KEY, JSON.stringify(next));
}

/**
 * Whether to open the About panel on its own for this visit: the canvas has
 * content, this device has not shown it before, and nothing else (a thread,
 * a profile, channel settings, agent activity) already holds the side panel,
 * so a link that opened a thread is never covered.
 */
export function shouldAutoOpenChannelAbout({
  hasContent,
  otherPanelOpen,
  seen,
}: {
  hasContent: boolean;
  otherPanelOpen: boolean;
  seen: boolean;
}) {
  return hasContent && !seen && !otherPanelOpen;
}
