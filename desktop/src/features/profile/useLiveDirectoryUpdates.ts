import * as React from "react";
import {
  type InfiniteData,
  type QueryClient,
  useQueryClient,
} from "@tanstack/react-query";

import {
  myRelayMembershipLookupQueryKey,
  myRelayMembershipQueryKey,
  relayMembersQueryKey,
} from "@/features/community-members/hooks";
import { evictUsersBatchEntries } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import {
  type RelayMembershipLookup,
  relayMembersFromEvent,
} from "@/shared/api/relayMembers";
import type {
  RelayEvent,
  RelayMember,
  UserSearchPage,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

const KIND_PROFILE = 0;
const KIND_NIP43_MEMBERSHIP_LIST = 13534;

/**
 * Refresh at most once per window, timed from the first event and never
 * re-armed: a burst of profile edits costs each open picker one request, and a
 * steady drip cannot postpone it. These refetches go over HTTP `POST /query`,
 * whose budget (about 5/s) every other relay read shares.
 */
export const DIRECTORY_REFRESH_WINDOW_MS = 2_000;

const SUBSCRIBE_RETRY_MAX_MS = 30_000;

type LiveRelay = Pick<typeof relayClient, "subscribeLive">;

/**
 * Push new members and profile edits into the people directory while the app
 * runs.
 *
 * The DM picker, @mention, member pickers and author labels all read cached
 * relay queries (`user-search`, `users-batch`, `user-profile`, `relayMembers`),
 * and nothing told those caches that a kind:0 or kind:13534 had landed, so a
 * member who joined mid-session stayed missing until the app restarted.
 *
 * Reconnect gaps fall to `useRelayAutoHeal`, which invalidates these roots
 * when the socket recovers. ponytail: it does not evict `users-batch-entry`, so
 * a label edited during a gap can stay stale for the entry's 10-minute
 * freshness; evict on heal if that shows up.
 */
export function startLiveDirectoryUpdates(
  queryClient: QueryClient,
  viewerPubkey: string,
  relay: LiveRelay = relayClient,
): () => void {
  const viewer = normalizePubkey(viewerPubkey);
  let disposed = false;
  let unsubscribe: (() => Promise<void>) | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let viewerRoleChanged = false;
  let newestMembershipAt = 0;
  const changedPubkeys = new Set<string>();

  const flush = () => {
    timer = null;
    const pubkeys = [...changedPubkeys];
    changedPubkeys.clear();
    if (pubkeys.length > 0) {
      // Evict first, or the users-batch re-run resolves from the old entry.
      evictUsersBatchEntries(queryClient, pubkeys);
      // An infinite query refetches every loaded page in turn; keep only the
      // first so a scrolled directory costs one request, not one per page.
      queryClient.setQueriesData<InfiniteData<UserSearchPage>>(
        { queryKey: ["user-search", "infinite"] },
        (data) =>
          data && {
            pages: data.pages.slice(0, 1),
            pageParams: data.pageParams.slice(0, 1),
          },
      );
      void queryClient.invalidateQueries({
        predicate: ({ queryKey }) =>
          queryKey[0] === "user-search" ||
          ((queryKey[0] === "user-profile" || queryKey[0] === "users-batch") &&
            pubkeys.some((pubkey) => queryKey.includes(pubkey))),
      });
    }
    if (viewerRoleChanged) {
      viewerRoleChanged = false;
      void queryClient.invalidateQueries({
        queryKey: myRelayMembershipQueryKey,
      });
      void queryClient.invalidateQueries({
        queryKey: myRelayMembershipLookupQueryKey,
      });
    }
  };

  const onEvent = (event: RelayEvent) => {
    if (disposed) return;
    if (event.kind === KIND_NIP43_MEMBERSHIP_LIST) {
      if (event.created_at < newestMembershipAt) return;
      newestMembershipAt = event.created_at;
      const roster = relayMembersFromEvent(event);
      // Same parser `listRelayMembers` feeds the query with, so the roster is
      // written directly instead of costing a request per snapshot.
      queryClient.setQueryData<RelayMember[]>(relayMembersQueryKey, roster);
      // A join changes no one's directory entry (that waits for their kind:0),
      // so only a change to the viewer's own role is worth a refetch.
      const role = roster.find((m) => m.pubkey === viewer)?.role ?? null;
      const shown = [
        queryClient.getQueryData<RelayMember | null>(myRelayMembershipQueryKey),
        queryClient.getQueryData<RelayMembershipLookup>(
          myRelayMembershipLookupQueryKey,
        )?.membership,
      ];
      if (
        !shown.some(
          (member) => member !== undefined && (member?.role ?? null) !== role,
        )
      ) {
        return;
      }
      viewerRoleChanged = true;
    } else if (event.kind === KIND_PROFILE) {
      changedPubkeys.add(normalizePubkey(event.pubkey));
    } else {
      return;
    }
    timer ??= setTimeout(flush, DIRECTORY_REFRESH_WINDOW_MS);
  };

  const subscribe = (attempt: number) => {
    retryTimer = null;
    void relay
      .subscribeLive(
        { kinds: [KIND_PROFILE, KIND_NIP43_MEMBERSHIP_LIST], limit: 0 },
        onEvent,
      )
      .then((dispose) => {
        if (disposed) void dispose();
        else unsubscribe = dispose;
      })
      .catch((error) => {
        if (disposed) return;
        // Retry: a subscription that failed at launch (offline, relay down)
        // would otherwise leave the directory frozen until a restart.
        console.error(
          "Couldn’t subscribe to member and profile updates",
          error,
        );
        retryTimer = setTimeout(
          () => subscribe(attempt + 1),
          Math.min(1_000 * 2 ** attempt, SUBSCRIBE_RETRY_MAX_MS),
        );
      });
  };
  subscribe(0);

  return () => {
    disposed = true;
    if (timer !== null) clearTimeout(timer);
    if (retryTimer !== null) clearTimeout(retryTimer);
    if (unsubscribe) void unsubscribe();
  };
}

/** Mount once per community, alongside the other app-level live subscriptions. */
export function useLiveDirectoryUpdates(): void {
  const queryClient = useQueryClient();
  const viewerPubkey = useIdentityQuery().data?.pubkey;
  React.useEffect(() => {
    if (!viewerPubkey) return;
    return startLiveDirectoryUpdates(queryClient, viewerPubkey);
  }, [queryClient, viewerPubkey]);
}
