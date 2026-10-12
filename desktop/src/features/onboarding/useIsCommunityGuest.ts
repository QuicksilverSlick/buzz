import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import { isCommunityGuest } from "@/features/onboarding/guestMode";
import { useIdentityQuery } from "@/shared/api/hooks";

/**
 * Whether the current identity joined the active community as a guest, by
 * invite. Guest mode is recorded before the app mounts for that community, so
 * reading it once per identity and community is enough.
 */
export function useIsCommunityGuest(): boolean {
  const pubkey = useIdentityQuery().data?.pubkey ?? null;
  const relayUrl = useCommunities().activeCommunity?.relayUrl ?? null;
  return React.useMemo(
    () => isCommunityGuest(pubkey, relayUrl),
    [pubkey, relayUrl],
  );
}
