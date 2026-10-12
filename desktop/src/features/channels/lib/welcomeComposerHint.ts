/** What a guest reads over the composer in #welcome-everyone. */
export const GUEST_AGENT_MENTION_HINT =
  "Type @ and an agent's name to ask it for something.";

/**
 * The hint over the composer in the welcome channels. "welcome-team" points
 * at @Fizz and the rest of the Welcome Team; "guest-agents" tells a guest how
 * to ask the community's agents.
 */
export type WelcomeComposerHint = "welcome-team" | "guest-agents";

/**
 * Which hint the welcome channels show above the composer, or null for none.
 * People who got the Welcome Team are pointed at @Fizz. Guests have no
 * Welcome Team, so they are told how to ask the community's agents when they
 * can see any, and see no hint otherwise.
 */
export function resolveWelcomeComposerHint({
  hasVisibleAgents,
  isGuest,
}: {
  hasVisibleAgents: boolean;
  isGuest: boolean;
}): WelcomeComposerHint | null {
  if (!isGuest) return "welcome-team";
  return hasVisibleAgents ? "guest-agents" : null;
}
