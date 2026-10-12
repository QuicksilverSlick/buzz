import { effectiveAgentDescription } from "@/features/agents/lib/agentDescription";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import {
  formatOwnerLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type {
  AgentPersona,
  ManagedAgent,
  RelayAgent,
  RespondToMode,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Extra lines the @-mention list shows under an agent: its public
 * description (the profile About text) and, when its reply setting leaves out
 * the person typing, who it does answer. Built only from data the app already
 * holds: this desktop's agents, the community's agent directory, the
 * agents' public profiles and local personas.
 */

/** Agents whose public profile the list looks up at once. */
export const MENTION_AGENT_DETAIL_LIMIT = 20;

export type AgentRespondPolicy = {
  ownerPubkey: string | null;
  respondTo: RespondToMode | null;
  respondToAllowlist: readonly string[];
};

/** One line of About text, or null when there is none. */
export function mentionAgentDescription(
  about: string | null | undefined,
): string | null {
  const line = about?.replace(/\s+/g, " ").trim() ?? "";
  return line.length > 0 ? line : null;
}

/**
 * The reply setting the app knows for an agent. An agent on this desktop is
 * the viewer's own and its local setting wins; otherwise the community's
 * agent directory carries the owner's published setting, when there is one.
 */
export function resolveAgentRespondPolicy(
  pubkey: string,
  {
    currentPubkey,
    managedAgents,
    relayAgents,
  }: {
    currentPubkey: string | null;
    managedAgents: readonly ManagedAgent[] | undefined;
    relayAgents: readonly RelayAgent[] | undefined;
  },
): AgentRespondPolicy | null {
  const key = normalizePubkey(pubkey);
  const managed = managedAgents?.find(
    (agent) => normalizePubkey(agent.pubkey) === key,
  );
  if (managed) {
    return {
      ownerPubkey: currentPubkey,
      respondTo: managed.respondTo,
      respondToAllowlist: managed.respondToAllowlist,
    };
  }
  const relay = relayAgents?.find(
    (agent) => normalizePubkey(agent.pubkey) === key,
  );
  return relay
    ? {
        ownerPubkey: relay.ownerPubkey,
        respondTo: relay.respondTo,
        respondToAllowlist: relay.respondToAllowlist,
      }
    : null;
}

/**
 * Whether the agent's known reply setting leaves out the person typing. Its
 * owner is always answered; an unknown setting is never treated as a refusal.
 */
export function agentIgnoresViewer(
  policy: AgentRespondPolicy | null,
  currentPubkey: string | null,
): boolean {
  if (!policy?.respondTo || !currentPubkey) return false;
  const viewer = normalizePubkey(currentPubkey);
  if (policy.ownerPubkey && normalizePubkey(policy.ownerPubkey) === viewer) {
    return false;
  }
  if (policy.respondTo === "owner-only") return true;
  if (policy.respondTo === "allowlist") {
    return !policy.respondToAllowlist.some(
      (pubkey) => normalizePubkey(pubkey) === viewer,
    );
  }
  return false;
}

/** "Answers only <owner>" when the agent will not answer the person typing. */
export function mentionAgentAudienceNote(
  policy: AgentRespondPolicy | null,
  currentPubkey: string | null,
  ownerProfiles?: UserProfileLookup,
): string | null {
  if (!policy?.ownerPubkey || !agentIgnoresViewer(policy, currentPubkey)) {
    return null;
  }
  const owner = formatOwnerLabel(
    policy.ownerPubkey,
    currentPubkey,
    ownerProfiles,
  );
  return owner ? `Answers only ${owner}` : null;
}

/** Agent identities in the list whose public profile is worth fetching. */
export function mentionAgentDetailPubkeys(
  suggestions: readonly MentionSuggestion[],
): string[] {
  const pubkeys: string[] = [];
  for (const suggestion of suggestions) {
    if (!suggestion.isAgent || !suggestion.pubkey) continue;
    if (suggestion.kind === "team" || suggestion.kind === "persona") continue;
    const pubkey = normalizePubkey(suggestion.pubkey);
    if (!pubkeys.includes(pubkey)) pubkeys.push(pubkey);
    if (pubkeys.length === MENTION_AGENT_DETAIL_LIMIT) break;
  }
  return pubkeys;
}

/**
 * Adds the description and audience note to agent suggestions. Returns the
 * same array when nothing changes, so the list keeps its identity.
 */
export function withMentionAgentDetails(
  suggestions: MentionSuggestion[],
  {
    aboutByPubkey,
    currentPubkey,
    managedAgents,
    ownerProfiles,
    personas,
    relayAgents,
  }: {
    aboutByPubkey: ReadonlyMap<string, string | null>;
    currentPubkey: string | null;
    managedAgents: readonly ManagedAgent[] | undefined;
    ownerProfiles?: UserProfileLookup;
    personas: readonly AgentPersona[] | undefined;
    relayAgents: readonly RelayAgent[] | undefined;
  },
): MentionSuggestion[] {
  const personaById = new Map(
    (personas ?? []).map((persona) => [persona.id, persona]),
  );
  const personaDescription = (personaId: string | undefined) => {
    const persona = personaId ? personaById.get(personaId) : undefined;
    return persona ? effectiveAgentDescription(persona) : null;
  };
  let changed = false;
  const next = suggestions.map((suggestion) => {
    if (!suggestion.isAgent || suggestion.kind === "team") return suggestion;
    const pubkey = suggestion.pubkey
      ? normalizePubkey(suggestion.pubkey)
      : null;
    const description =
      suggestion.kind === "persona"
        ? mentionAgentDescription(personaDescription(suggestion.personaId))
        : ((pubkey
            ? mentionAgentDescription(aboutByPubkey.get(pubkey))
            : null) ??
          mentionAgentDescription(personaDescription(suggestion.personaId)));
    const audienceNote = pubkey
      ? mentionAgentAudienceNote(
          resolveAgentRespondPolicy(pubkey, {
            currentPubkey,
            managedAgents,
            relayAgents,
          }),
          currentPubkey,
          ownerProfiles,
        )
      : null;
    if (
      (suggestion.description ?? null) === description &&
      (suggestion.audienceNote ?? null) === audienceNote
    ) {
      return suggestion;
    }
    changed = true;
    return { ...suggestion, audienceNote, description };
  });
  return changed ? next : suggestions;
}
