import { normalizePubkey } from "@/shared/lib/pubkey";

import {
  getPersistentAgentAudienceSnapshot,
  initializePersistentAgentAudience,
  isPersistentAgentAudienceMemberExcluded,
} from "./persistentAgentAudience";

/** A thread message, as far as who is in the thread goes. */
export type ThreadParticipantSource = {
  pubkey?: string | null;
  tags?: readonly (readonly string[])[];
};

/**
 * Owner rule (29 Oct 2026): a thread that is only the viewer plus one agent
 * needs no tag. Returns that agent once it has posted in the thread; "shared"
 * once anyone else has posted or been tagged; otherwise null, including while
 * the thread is loading (`participants` null).
 */
export function selectOneOnOneThreadAgent({
  currentPubkey,
  isAgentPubkey,
  participants,
}: {
  currentPubkey: string | null;
  isAgentPubkey: (pubkey: string) => boolean;
  participants: readonly ThreadParticipantSource[] | null | undefined;
}): string | null {
  if (!participants || !currentPubkey) return null;
  const self = normalizePubkey(currentPubkey);
  const others = new Set<string>();
  const posters = new Set<string>();
  for (const { pubkey, tags = [] } of participants) {
    if (pubkey) posters.add(normalizePubkey(pubkey));
    for (const candidate of [
      pubkey,
      ...tags.filter((tag) => tag[0] === "p").map((tag) => tag[1]),
    ]) {
      const normalized = candidate ? normalizePubkey(candidate) : "";
      if (normalized && normalized !== self) others.add(normalized);
    }
  }
  if (others.size > 1) return "shared";
  const [agent] = others;
  return agent && isAgentPubkey(agent) && posters.has(agent) ? agent : null;
}

// ponytail: per-session, one entry per thread visited; bound these like the
// audience store if a session ever visits thousands of threads.
const autoAddressedByScope = new Map<string, string>();
// Chips the user removed. Turning the switch off resets the store and its
// exclusions, so removals are remembered here too.
const declined = new Set<string>();

/**
 * Pre-addresses a one-on-one thread's agent with the usual removable chip.
 * Once the thread is "shared", returns the agent if this rule added it and it
 * is still addressed, so the caller can drop it; explicit pins never return.
 */
export function syncOneOnOneThreadAgent(
  scope: string,
  decision: string | null,
): string | null {
  const audience = getPersistentAgentAudienceSnapshot().audiences[scope] ?? [];
  const added = autoAddressedByScope.get(scope);
  // Gone from the chips (removed, maybe re-added by hand): no longer ours.
  if (added && !audience.includes(added)) autoAddressedByScope.delete(scope);
  if (decision === "shared") {
    const revoke = autoAddressedByScope.get(scope) ?? null;
    autoAddressedByScope.delete(scope);
    return revoke;
  }
  if (!decision) return null;
  const key = `${scope}\n${decision}`;
  if (isPersistentAgentAudienceMemberExcluded(scope, decision)) {
    declined.add(key);
  }
  if (declined.has(key)) return null;
  if (initializePersistentAgentAudience(scope, [decision]).length > 0) {
    autoAddressedByScope.set(scope, decision);
  }
  return null;
}
