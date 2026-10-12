import type { Channel } from "@/shared/api/types";

/** The authored channel detail shown consistently across channel surfaces. */
export function getChannelDetail(channel: Channel): string | null {
  return (
    [channel.topic, channel.description, channel.purpose]
      .find((value) => value && value.trim().length > 0)
      ?.trim() ?? null
  );
}

/**
 * The full hover text for a channel: status prefixes plus its detail. Empty
 * when the channel has neither, so no filler text reaches the header.
 */
export function getChannelDescription(channel: Channel | null): string {
  if (!channel) {
    return "Connect to the relay to browse channels and read messages.";
  }

  const prefixes = [
    channel.archivedAt ? "Archived." : null,
    !channel.isMember ? "Read-only until you join this open channel." : null,
  ].filter((value) => value && value.trim().length > 0);

  // Show only the first non-empty field to avoid duplication when
  // topic, description, and purpose contain overlapping text.
  const detail = getChannelDetail(channel);

  // Join the status prefixes with spaces, but keep the detail text's own
  // line breaks intact (native `title` tooltips render newlines) and separate
  // it from the prefixes with a newline so paragraphs stay readable.
  const prefixText = prefixes.join(" ");
  const parts = [prefixText || null, detail ?? null].filter(Boolean);

  return parts.join("\n");
}

/** What the channel header shows on the line under the channel name. */
export type ChannelHeaderLine =
  | { kind: "description"; text: string; title: string }
  | { kind: "add-description" }
  | { kind: "none" };

/**
 * Picks the line under the channel name in the header. Everyone sees the
 * channel's description there. When a channel has none, people who can
 * manage it get an "Add a description" action and everyone else sees
 * nothing. `text` is one line; `title` is the full text shown on hover.
 */
export function getChannelHeaderLine(
  channel: Channel | null,
  canManageChannel: boolean,
): ChannelHeaderLine {
  if (!channel || channel.channelType === "dm") {
    return { kind: "none" };
  }

  const detail = getChannelDetail(channel);
  if (detail) {
    return {
      kind: "description",
      text: detail.replace(/\s+/g, " "),
      title: getChannelDescription(channel),
    };
  }

  return canManageChannel && !channel.archivedAt
    ? { kind: "add-description" }
    : { kind: "none" };
}
