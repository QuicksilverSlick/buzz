import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import {
  getMentionableAgentPubkeys,
  isAgentDirectoryReady,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import type { Channel } from "@/shared/api/types";

/**
 * Whether the @ list in this stream channel offers the viewer at least one
 * agent that answers them, by the same check the mention picker uses: their
 * own agents, plus other people's agents that are members of this channel and
 * whose reply setting includes the viewer. False until both agent directories
 * have loaded, so a hint never promises an agent the list won't show.
 */
export function useHasMentionableAgents(
  channel: Channel | null,
  currentPubkey: string | null | undefined,
): boolean {
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const directoriesReady =
    isAgentDirectoryReady(managedAgentsQuery) &&
    isAgentDirectoryReady(relayAgentsQuery);
  const channelId = channel?.channelType === "stream" ? channel.id : null;
  const managedAgents = managedAgentsQuery.data;
  const relayAgents = relayAgentsQuery.data;

  return React.useMemo(() => {
    if (!directoriesReady || !channelId) return false;
    return (
      getMentionableAgentPubkeys({
        currentPubkey,
        phase: "prepare",
        eligibilityScope: { type: "channel", channelId },
        managedAgentPubkeys: (managedAgents ?? []).map((agent) => agent.pubkey),
        relayAgents,
        // In a channel, whether an agent answers depends on that channel
        // alone, so the viewer's other channels don't change the answer.
        sharedChannelIds: new Set([channelId]),
      }).size > 0
    );
  }, [channelId, currentPubkey, directoriesReady, managedAgents, relayAgents]);
}
