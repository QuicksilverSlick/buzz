import { normalizePubkey } from "@/shared/lib/pubkey";
import { channelAgentMembers } from "@/shared/lib/rosterDerivations";

/**
 * Pure merge behind `useKnownAgentPubkeys`: managed agents ∪ relay agents,
 * normalised via `normalizePubkey` so membership checks work against
 * normalised pubkeys.
 *
 * Structurally typed on `{ pubkey }` so node unit tests don't need to build
 * full `ManagedAgent`/`RelayAgent` values.
 */
export function mergeKnownAgentPubkeys(
  managedAgents: readonly { pubkey: string }[] | undefined,
  relayAgents: readonly { pubkey: string }[] | undefined,
): ReadonlySet<string> {
  const pubkeys = new Set<string>();
  for (const agent of managedAgents ?? []) {
    pubkeys.add(normalizePubkey(agent.pubkey));
  }
  for (const agent of relayAgents ?? []) {
    pubkeys.add(normalizePubkey(agent.pubkey));
  }
  return pubkeys;
}

/** Agent identities controlled by the current user. */
export function mergeOwnedAgentPubkeys(
  managedAgents: readonly { pubkey: string }[] | undefined,
  profiles:
    | Readonly<Record<string, { ownerPubkey?: string | null }>>
    | undefined,
  currentPubkey: string | null | undefined,
): ReadonlySet<string> {
  const pubkeys = new Set<string>();
  for (const agent of managedAgents ?? []) {
    pubkeys.add(normalizePubkey(agent.pubkey));
  }

  if (!currentPubkey) return pubkeys;

  const ownerPubkey = normalizePubkey(currentPubkey);
  for (const [pubkey, profile] of Object.entries(profiles ?? {})) {
    if (
      profile.ownerPubkey &&
      normalizePubkey(profile.ownerPubkey) === ownerPubkey
    ) {
      pubkeys.add(normalizePubkey(pubkey));
    }
  }

  return pubkeys;
}

/**
 * Whether the current user owns any agent: one this desktop manages, or one
 * the relay lists with the current user as its owner. Agents on
 * `starterTeamId` don't count on either list, because onboarding gives that
 * team to every new member, guests included. Guests who own no other agent
 * have no use for agent tooling such as the terminal.
 */
export function ownsAnyAgent(
  managedAgents:
    | readonly { pubkey: string; teamId?: string | null }[]
    | undefined,
  relayAgents:
    | readonly { pubkey: string; ownerPubkey: string | null }[]
    | undefined,
  currentPubkey: string | null | undefined,
  starterTeamId?: string,
): boolean {
  const starterPubkeys = new Set<string>();
  for (const agent of managedAgents ?? []) {
    if (starterTeamId === undefined || agent.teamId !== starterTeamId) {
      return true;
    }
    starterPubkeys.add(normalizePubkey(agent.pubkey));
  }
  if (!currentPubkey) return false;

  const ownerPubkey = normalizePubkey(currentPubkey);
  return (relayAgents ?? []).some(
    (agent) =>
      agent.ownerPubkey !== null &&
      normalizePubkey(agent.ownerPubkey) === ownerPubkey &&
      !starterPubkeys.has(normalizePubkey(agent.pubkey)),
  );
}

/**
 * Channel-scoped variant: the managed ∪ relay baseline plus this channel's
 * bot members (role `bot` or `isAgent`), so member-only agents are included.
 */
export function mergeChannelKnownAgentPubkeys(
  channelMembers:
    | readonly { pubkey: string; role: string; isAgent: boolean }[]
    | undefined,
  managedAgents: readonly { pubkey: string }[] | undefined,
  relayAgents: readonly { pubkey: string }[] | undefined,
): ReadonlySet<string> {
  const pubkeys = new Set(mergeKnownAgentPubkeys(managedAgents, relayAgents));
  // Identity-cached agent subset: avoids walking the full roster per call.
  for (const member of channelMembers
    ? channelAgentMembers(channelMembers)
    : []) {
    pubkeys.add(normalizePubkey(member.pubkey));
  }
  return pubkeys;
}
