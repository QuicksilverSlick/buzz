import { normalizePubkey } from "@/shared/lib/pubkey";

import { initializePersistentAgentAudience } from "./persistentAgentAudience";

/** A thread message, as far as who is in the thread goes. */
export type ThreadParticipantSource = {
  pubkey?: string | null;
  tags?: readonly (readonly string[])[];
};

/**
 * Owner rule (29 Oct 2026): a thread that is only the viewer plus one agent
 * needs no tag. Returns that agent when it wrote the root or the parent being
 * replied to; "shared" once anyone else has posted or been tagged; otherwise
 * null, including while the thread is loading (`participants` null).
 */
export function selectOneOnOneThreadAgent({
  currentPubkey,
  isAgentPubkey,
  participants,
  replyToPubkeys,
}: {
  currentPubkey: string | null;
  isAgentPubkey: (pubkey: string) => boolean;
  participants: readonly ThreadParticipantSource[] | null | undefined;
  replyToPubkeys: readonly (string | null | undefined)[];
}): string | null {
  if (!participants || !currentPubkey) return null;
  const self = normalizePubkey(currentPubkey);
  const others = new Set<string>();
  for (const { pubkey, tags = [] } of participants) {
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
  return agent &&
    isAgentPubkey(agent) &&
    replyToPubkeys.some((pubkey) => pubkey && normalizePubkey(pubkey) === agent)
    ? agent
    : null;
}

const autoAddressedByScope = new Map<string, string>();

/**
 * Pre-addresses a one-on-one thread's agent with the usual removable chip (a
 * removed chip is an exclusion the audience store keeps honouring). Once the
 * thread is "shared", returns the agent this rule added so the caller can drop
 * it; explicit pins are never returned.
 */
export function syncOneOnOneThreadAgent(
  scope: string,
  decision: string | null,
): string | null {
  if (decision === "shared") {
    const added = autoAddressedByScope.get(scope) ?? null;
    autoAddressedByScope.delete(scope);
    return added;
  }
  if (decision) {
    autoAddressedByScope.set(scope, decision);
    initializePersistentAgentAudience(scope, [decision]);
  }
  return null;
}
