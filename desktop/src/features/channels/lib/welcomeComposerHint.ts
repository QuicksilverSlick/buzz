import type { ChannelType } from "@/shared/api/types";

/** What a guest reads over the composer where an agent will answer them. */
export const GUEST_AGENT_MENTION_HINT =
  "Type @ and an agent's name to ask it for something.";

/**
 * The hint over the composer. "welcome-team" points at @Fizz and the rest of
 * the Welcome Team; "guest-agents" tells a guest how to ask the community's
 * agents.
 */
export type WelcomeComposerHint = "welcome-team" | "guest-agents";

/**
 * Which hint shows above the composer, or null for none. People who got the
 * Welcome Team are pointed at @Fizz in the welcome channels. Guests have no
 * Welcome Team: in any channel where the @ list offers them an agent that
 * answers them, they are told how to ask it, until they have used the hint
 * or dismissed it. Where no agent would answer, they see no hint.
 */
export function resolveWelcomeComposerHint({
  channelType,
  guestHintDone,
  hasMentionableAgents,
  isGuest,
  isWelcomeChannel,
}: {
  channelType: ChannelType | null;
  guestHintDone: boolean;
  hasMentionableAgents: boolean;
  isGuest: boolean;
  isWelcomeChannel: boolean;
}): WelcomeComposerHint | null {
  if (!isGuest) return isWelcomeChannel ? "welcome-team" : null;
  return channelType === "stream" && hasMentionableAgents && !guestHintDone
    ? "guest-agents"
    : null;
}
