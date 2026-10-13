import * as React from "react";

import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  clearCommunityGuest,
  isCommunityGuest,
  isGuestAgentHintDone,
  markGuestAgentHintDone,
} from "@/features/onboarding/guestMode";
import { useIdentityQuery } from "@/shared/api/hooks";
import { canManageCommunityMembers } from "@/shared/api/relayMembers";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type CommunityGuestStatus = {
  /** This identity is a guest of this community. */
  isGuest: boolean;
  /**
   * The answer is final for now: nobody saved as a guest, or the relay has
   * answered the role check for someone who was.
   */
  isSettled: boolean;
};

/**
 * Whether `pubkey` joined the community at `relayUrl` as a guest, by invite.
 * The saved guest flag corrects itself: guest entry decides on the invite
 * alone when the role lookup doesn't answer, so when the relay later lists
 * this person as the community's owner or admin, the flag is cleared and they
 * get the normal experience back. Only people saved as guests ask the relay.
 */
export function useCommunityGuestStatus(
  pubkey: string | null,
  relayUrl: string | null,
): CommunityGuestStatus {
  const isSavedGuest = React.useMemo(
    () => isCommunityGuest(pubkey, relayUrl),
    [pubkey, relayUrl],
  );
  const membershipQuery = useMyRelayMembershipLookupQuery(isSavedGuest);
  const lookup = isSavedGuest ? membershipQuery.data : undefined;
  const isOwnerOrAdmin =
    pubkey !== null &&
    lookup?.membership != null &&
    normalizePubkey(lookup.membership.pubkey) === normalizePubkey(pubkey) &&
    canManageCommunityMembers(lookup);

  React.useEffect(() => {
    if (isOwnerOrAdmin && pubkey && relayUrl) {
      clearCommunityGuest(pubkey, relayUrl);
    }
  }, [isOwnerOrAdmin, pubkey, relayUrl]);

  return {
    isGuest: isSavedGuest && !isOwnerOrAdmin,
    isSettled: !isSavedGuest || membershipQuery.status !== "pending",
  };
}

/** Whether the current identity joined the active community as a guest. */
export function useIsCommunityGuest(): boolean {
  return useCommunityGuest().isGuest;
}

/**
 * Guest mode for the current identity in the active community, with the
 * guest's hint about asking agents: whether it was used or dismissed on an
 * earlier visit, and a way to record that it has been now. Recording does not
 * change `agentHintDone` until the next visit, so the hint can finish its own
 * "Nice work." exit.
 */
export function useCommunityGuest() {
  const pubkey = useIdentityQuery().data?.pubkey ?? null;
  const relayUrl = useCommunities().activeCommunity?.relayUrl ?? null;
  const { isGuest } = useCommunityGuestStatus(pubkey, relayUrl);
  const agentHintDone = React.useMemo(
    () => isGuestAgentHintDone(pubkey, relayUrl),
    [pubkey, relayUrl],
  );
  const markAgentHintDone = React.useCallback(() => {
    if (pubkey && relayUrl) markGuestAgentHintDone(pubkey, relayUrl);
  }, [pubkey, relayUrl]);
  return { agentHintDone, isGuest, markAgentHintDone };
}
