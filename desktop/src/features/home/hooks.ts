import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import { isOwnWorkflowPost } from "@/features/messages/lib/workflowPost";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getHomeFeed } from "@/shared/api/tauri";
import type { HomeFeedResponse } from "@/shared/api/types";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

/** Keeps focused polling at the established 30-second cadence. */
export const HOME_FEED_REFETCH_INTERVAL_MS = 30_000;
/** Suppresses the expensive focus refetch until the home feed is old. */
export const HOME_FEED_FOCUS_STALE_TIME_MS = 5 * 60_000;

/** Focus-refetch policy for the home feed query; consumed by focusRefetchPolicy.test.mjs. */
export const homeFeedFocusRefetchPolicy = {
  staleTime: HOME_FEED_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: false,
} as const;

/**
 * Drops the viewer's own workflow posts from every feed section. They reach
 * the feed only because the engine p-tags the owner, so they are not mentions;
 * the Inbox, its badge and desktop notifications all read this feed.
 */
export function withoutOwnWorkflowPosts(
  response: HomeFeedResponse,
  currentPubkey: string | null | undefined,
  relaySelfPubkey: string | null | undefined,
): HomeFeedResponse {
  if (!currentPubkey || !relaySelfPubkey) return response;
  const keep = (items: HomeFeedResponse["feed"]["mentions"]) =>
    items.filter(
      (item) => !isOwnWorkflowPost(item, currentPubkey, relaySelfPubkey),
    );
  return {
    ...response,
    feed: {
      mentions: keep(response.feed.mentions),
      needsAction: keep(response.feed.needsAction),
      activity: keep(response.feed.activity),
      agentActivity: keep(response.feed.agentActivity),
    },
  };
}

export function useHomeFeedQuery() {
  const connectionState = useRelayConnection();
  const connected = connectionState === "connected";
  const refetchInterval = useFocusedRefetchInterval(
    connected ? HOME_FEED_REFETCH_INTERVAL_MS : false,
  );
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const relaySelfPubkey = useRelaySelfQuery().data;
  const select = React.useCallback(
    (response: HomeFeedResponse) =>
      withoutOwnWorkflowPosts(response, currentPubkey, relaySelfPubkey),
    [currentPubkey, relaySelfPubkey],
  );

  return useQuery({
    queryKey: ["home-feed"],
    queryFn: () =>
      getHomeFeed({
        limit: 50,
        types: "mentions,needs_action,activity,agent_activity",
      }),
    gcTime: 5 * 60 * 1_000,
    // Pause background polling on degraded/stalled/disconnected connections.
    // The relay can't serve the request anyway, and the spurious failures
    // consume quota that the recovery path needs.
    refetchInterval,
    select,
    ...homeFeedFocusRefetchPolicy,
  });
}
