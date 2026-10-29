import type { Query, QueryClient } from "@tanstack/react-query";

import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

function isChannelMembersQuery(query: Query): boolean {
  const { queryKey } = query;
  return (
    queryKey.length === 3 &&
    queryKey[0] === "channels" &&
    typeof queryKey[1] === "string" &&
    queryKey[2] === "members"
  );
}

function queryContainsAnyMember(
  query: Query,
  names: ReadonlyMap<string, string | null>,
): boolean {
  if (!isChannelMembersQuery(query) || !Array.isArray(query.state.data)) {
    return false;
  }

  return (query.state.data as ChannelMember[]).some((member) =>
    names.has(normalizePubkey(member.pubkey)),
  );
}

/**
 * Applies a profile rename to every cached channel-member row for one pubkey.
 * Matching queries are marked stale without refetching, so active channels
 * update immediately while a rename never fans out into one relay query per
 * cached channel.
 */
export function updateCachedChannelMemberDisplayName(
  queryClient: QueryClient,
  pubkey: string,
  displayName: string | null,
): Promise<void> {
  return updateCachedChannelMemberDisplayNames(queryClient, [
    [pubkey, displayName],
  ]);
}

/**
 * Applies several renames in one pass. Batch them rather than looping the
 * single form: each pass cancels and restarts in-flight roster fetches, so N
 * calls would re-issue the same `get_channel_members` N times.
 */
export async function updateCachedChannelMemberDisplayNames(
  queryClient: QueryClient,
  renames: ReadonlyArray<readonly [pubkey: string, displayName: string | null]>,
): Promise<void> {
  const names = new Map(
    renames.map(([pubkey, displayName]) => [
      normalizePubkey(pubkey),
      displayName,
    ]),
  );
  const renamed = (member: ChannelMember) => {
    const pubkey = normalizePubkey(member.pubkey);
    return names.has(pubkey) && member.displayName !== names.get(pubkey);
  };
  const matchingMemberPredicate = (query: Query) =>
    queryContainsAnyMember(query, names);
  const matchingMemberQueries = queryClient
    .getQueryCache()
    .findAll({ predicate: matchingMemberPredicate });
  const matchingMemberQueryHashes = new Set(
    matchingMemberQueries.map((query) => query.queryHash),
  );
  const fetchingMemberQueryHashes = new Set(
    queryClient
      .getQueryCache()
      .findAll({
        predicate: (query) =>
          isChannelMembersQuery(query) &&
          query.state.fetchStatus === "fetching",
      })
      .map((query) => query.queryHash),
  );

  // Nothing to rename and nothing in flight that could land an old name:
  // leave every roster's freshness alone, so a republished but unchanged
  // kind:0 does not force a roster refetch on the next channel visit.
  const hasOutdatedRow = matchingMemberQueries.some((query) =>
    (query.state.data as ChannelMember[]).some(renamed),
  );
  if (!hasOutdatedRow && fetchingMemberQueryHashes.size === 0) return;

  // A first-load query has no rows to inspect yet, so cancel every member
  // fetch already in flight. Restart only requests without a matching cached
  // row after applying the local rename; this bounds network work to requests
  // that were already underway while preventing pre-rename snapshots from
  // winning.
  await queryClient.cancelQueries({
    predicate: (query) => fetchingMemberQueryHashes.has(query.queryHash),
  });

  queryClient.setQueriesData<ChannelMember[]>(
    { predicate: matchingMemberPredicate },
    (current) => {
      if (!current) {
        return current;
      }

      return current.map((member) =>
        renamed(member)
          ? {
              ...member,
              displayName: names.get(normalizePubkey(member.pubkey)) ?? null,
            }
          : member,
      );
    },
  );

  await queryClient.invalidateQueries({
    predicate: matchingMemberPredicate,
    refetchType: "none",
  });

  void queryClient.refetchQueries({
    predicate: (query) =>
      fetchingMemberQueryHashes.has(query.queryHash) &&
      !matchingMemberQueryHashes.has(query.queryHash),
    type: "all",
  });
}
