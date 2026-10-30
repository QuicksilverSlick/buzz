import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  useOpenDmMutation,
  useUpsertCachedChannel,
} from "@/features/channels/hooks";
import {
  CHANNEL_MEMBERS_STALE_TIME_MS,
  channelMembersQueryKey,
} from "@/features/channels/rosterFreshness";
import { getChannelMembers } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function usePrepareDmSendChannel(
  activeChannel: Channel | null,
  currentPubkey?: string,
) {
  const openDmMutation = useOpenDmMutation();
  const upsertCachedChannel = useUpsertCachedChannel();
  const queryClient = useQueryClient();

  return React.useCallback(
    async (additionalParticipantPubkeys: string[] = []) => {
      if (activeChannel?.channelType !== "dm") {
        return activeChannel?.id ?? null;
      }

      const currentParticipantPubkeys = new Set(
        activeChannel.participantPubkeys.map(normalizePubkey),
      );
      const requiresExpandedDm = additionalParticipantPubkeys.some(
        (pubkey) => !currentParticipantPubkeys.has(normalizePubkey(pubkey)),
      );
      if (!requiresExpandedDm) {
        return activeChannel.id;
      }

      const currentNormalizedPubkey = currentPubkey
        ? normalizePubkey(currentPubkey)
        : null;
      const pubkeys = [
        ...new Set(
          [
            ...activeChannel.participantPubkeys,
            ...additionalParticipantPubkeys,
          ].map(normalizePubkey),
        ),
      ].filter((pubkey) => pubkey && pubkey !== currentNormalizedPubkey);
      const expandedDm = await openDmMutation.mutateAsync({ pubkeys });
      await upsertCachedChannel(expandedDm);
      // The send tells the new group DM's agents apart by its member list
      // (dmRecipientPubkeys), which no screen has loaded yet.
      await queryClient.prefetchQuery({
        queryKey: channelMembersQueryKey(expandedDm.id),
        queryFn: () => getChannelMembers(expandedDm.id),
        staleTime: CHANNEL_MEMBERS_STALE_TIME_MS,
      });
      return expandedDm.id;
    },
    [
      activeChannel,
      currentPubkey,
      openDmMutation.mutateAsync,
      queryClient,
      upsertCachedChannel,
    ],
  );
}
