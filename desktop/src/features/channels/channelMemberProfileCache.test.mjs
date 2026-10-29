import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryObserver } from "@tanstack/react-query";

import {
  updateCachedChannelMemberDisplayName,
  updateCachedChannelMemberDisplayNames,
} from "./channelMemberProfileCache.ts";

function member(pubkey, displayName, overrides = {}) {
  return {
    pubkey,
    role: "bot",
    isAgent: true,
    joinedAt: null,
    displayName,
    ...overrides,
  };
}

test("updateCachedChannelMemberDisplayName_updatesOnlyMatchingMemberQueries", async () => {
  const queryClient = new QueryClient();
  const matchingKey = ["channels", "general", "members"];
  const unrelatedMembersKey = ["channels", "random", "members"];
  const detailKey = ["channels", "general", "detail"];

  queryClient.setQueryData(matchingKey, [
    member("AABB", "Old name"),
    member("ccdd", "Someone else", { role: "member", isAgent: false }),
  ]);
  queryClient.setQueryData(unrelatedMembersKey, [member("eeff", "Other")]);
  queryClient.setQueryData(detailKey, { name: "General" });

  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");

  assert.deepEqual(queryClient.getQueryData(matchingKey), [
    member("AABB", "New name"),
    member("ccdd", "Someone else", { role: "member", isAgent: false }),
  ]);
  assert.deepEqual(queryClient.getQueryData(unrelatedMembersKey), [
    member("eeff", "Other"),
  ]);
  assert.deepEqual(queryClient.getQueryData(detailKey), { name: "General" });
  assert.equal(queryClient.getQueryState(matchingKey)?.isInvalidated, true);
  assert.equal(
    queryClient.getQueryState(unrelatedMembersKey)?.isInvalidated,
    false,
  );
  assert.equal(queryClient.getQueryState(detailKey)?.isInvalidated, false);
});

test("updateCachedChannelMemberDisplayName_leavesRostersFreshWhenNameUnchanged", async () => {
  // A republished kind:0 with the same name must not cost a roster refetch on
  // the next channel visit.
  const queryClient = new QueryClient();
  const matchingKey = ["channels", "general", "members"];
  queryClient.setQueryData(matchingKey, [member("aabb", "Same name")]);

  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "Same name");

  assert.equal(queryClient.getQueryState(matchingKey)?.isInvalidated, false);
});

test("updateCachedChannelMemberDisplayName_doesNotRefetchMatchingQueries", async () => {
  let fetchCount = 0;
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        queryFn: async () => {
          fetchCount += 1;
          return [member("aabb", "Relay name")];
        },
      },
    },
  });
  const matchingKey = ["channels", "general", "members"];
  queryClient.setQueryData(matchingKey, [member("aabb", "Old name")]);

  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");

  assert.equal(fetchCount, 0);
  assert.equal(
    queryClient.getQueryData(matchingKey)[0].displayName,
    "New name",
  );
});

test("updateCachedChannelMemberDisplayName_cancelsStaleInFlightFetches", async () => {
  const queryClient = new QueryClient();
  const matchingKey = ["channels", "general", "members"];
  let resolveFetch;

  queryClient.setQueryData(matchingKey, [member("aabb", "Old name")]);
  const fetch = queryClient.fetchQuery({
    queryKey: matchingKey,
    queryFn: () =>
      new Promise((resolve) => {
        resolveFetch = resolve;
      }),
  });

  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");
  resolveFetch([member("aabb", "Stale relay name")]);
  await fetch.catch(() => undefined);

  assert.equal(
    queryClient.getQueryData(matchingKey)[0].displayName,
    "New name",
  );
});

test("updateCachedChannelMemberDisplayName_restartsFirstLoadMemberFetches", async () => {
  const queryClient = new QueryClient();
  const matchingKey = ["channels", "general", "members"];
  const fetchResolvers = [];
  const observer = new QueryObserver(queryClient, {
    queryKey: matchingKey,
    queryFn: () =>
      new Promise((resolve) => {
        fetchResolvers.push(resolve);
      }),
  });
  let resolveObservedData;
  const observedData = new Promise((resolve) => {
    resolveObservedData = resolve;
  });
  const unsubscribe = observer.subscribe((result) => {
    if (result.data) {
      resolveObservedData();
    }
  });

  assert.equal(fetchResolvers.length, 1);
  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");

  assert.equal(fetchResolvers.length, 2);
  fetchResolvers[1]([member("aabb", "New name")]);
  fetchResolvers[0]([member("aabb", "Stale relay name")]);
  await observedData;

  assert.equal(
    queryClient.getQueryData(matchingKey)[0].displayName,
    "New name",
  );
  unsubscribe();
});

test("updateCachedChannelMemberDisplayNames_restartsAnInFlightFetchOncePerBatch", async () => {
  // A burst of republished agent profiles must not re-issue a loading roster
  // once per profile: each restart is a full get_channel_members.
  const queryClient = new QueryClient();
  const loadingKey = ["channels", "general", "members"];
  const cachedKey = ["channels", "random", "members"];
  queryClient.setQueryData(cachedKey, [
    member("aabb", "Old A"),
    member("ccdd", "Old C"),
  ]);
  const fetchResolvers = [];
  const observer = new QueryObserver(queryClient, {
    queryKey: loadingKey,
    queryFn: () =>
      new Promise((resolve) => {
        fetchResolvers.push(resolve);
      }),
  });
  const unsubscribe = observer.subscribe(() => {});
  assert.equal(fetchResolvers.length, 1);

  await updateCachedChannelMemberDisplayNames(queryClient, [
    ["AABB", "New A"],
    ["ccdd", "New C"],
    ["eeff", "Not cached anywhere"],
  ]);

  assert.equal(fetchResolvers.length, 2);
  assert.deepEqual(
    queryClient.getQueryData(cachedKey).map((row) => row.displayName),
    ["New A", "New C"],
  );
  for (const resolve of fetchResolvers) resolve([]);
  unsubscribe();
});
