import * as React from "react";
import { useQueries } from "@tanstack/react-query";

import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { getUserProfile } from "@/shared/api/tauriProfiles";
import type {
  AgentPersona,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";
import {
  mentionAgentDetailPubkeys,
  withMentionAgentDetails,
} from "./mentionAgentDetails";

/**
 * An agent's About text from its public profile. It lives under the
 * "user-profile" key so every profile refresh (an agent rename, a persona
 * edit) refreshes it too, but in its own entry: the shared ["user-profile",
 * pubkey] entry is often seeded from the people list, which carries no About.
 */
function agentAboutQueryOptions(pubkey: string) {
  return {
    queryKey: ["user-profile", pubkey, "about"],
    queryFn: async () => (await getUserProfile(pubkey)).about,
    staleTime: 60_000,
  };
}

/**
 * Adds each listed agent's About text and, when it will not answer the person
 * typing, who it does answer. About text is looked up for at most the first
 * few agents in the list.
 */
export function useMentionAgentDetails(
  suggestions: MentionSuggestion[],
  options: {
    currentPubkey: string | null;
    managedAgents: readonly ManagedAgent[] | undefined;
    ownerProfiles?: UserProfileLookup;
    personas: readonly AgentPersona[] | undefined;
    relayAgents: readonly RelayAgent[] | undefined;
  },
): MentionSuggestion[] {
  const pubkeys = React.useMemo(
    () => mentionAgentDetailPubkeys(suggestions),
    [suggestions],
  );
  // Joined into one string so the list only changes when an About does.
  const abouts = useQueries({
    queries: pubkeys.map(agentAboutQueryOptions),
    combine: (results) =>
      results.map((result) => result.data ?? "").join("\u0000"),
  });
  const aboutByPubkey = React.useMemo(() => {
    const values = abouts.split("\u0000");
    return new Map(
      pubkeys.map((pubkey, index) => [pubkey, values[index] || null]),
    );
  }, [abouts, pubkeys]);
  const { currentPubkey, managedAgents, ownerProfiles, personas, relayAgents } =
    options;

  return React.useMemo(
    () =>
      withMentionAgentDetails(suggestions, {
        aboutByPubkey,
        currentPubkey,
        managedAgents,
        ownerProfiles,
        personas,
        relayAgents,
      }),
    [
      aboutByPubkey,
      currentPubkey,
      managedAgents,
      ownerProfiles,
      personas,
      relayAgents,
      suggestions,
    ],
  );
}
