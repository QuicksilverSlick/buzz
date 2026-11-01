import * as React from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import {
  type BenchCardMatch,
  type BenchCards,
  type BenchMessageRef,
  type BenchRowState,
  type BenchSnapshot,
  benchRowState,
  createPollGroup,
  enabledBenchSnapshot,
  indexBenchCards,
  isBenchCandidate,
} from "@/features/bench/lib/benchCards";
import { benchNeedsYouCount } from "@/features/bench/lib/benchScreen";
import { invokeTauri } from "@/shared/api/tauri";
import { isDocumentVisible } from "@/shared/lib/useDocumentVisible";

export const BENCH_CARDS_QUERY_KEY = ["bench-cards"] as const;
/** Snapshot cadence while a Bench surface is on screen and visible. */
export const BENCH_POLL_INTERVAL_MS = 3_000;
/** Off #bench the snapshot only gates rendering, so a mount reuses it. */
const BENCH_IDLE_STALE_MS = 60_000;
/** The sidebar badge's own cadence, everywhere else in the app. */
const BENCH_SIDEBAR_POLL_MS = 15_000;

export type BenchAnswerInput = {
  itemId: string;
  /** The card the owner saw: the Bench refuses to answer any other. */
  cardEventId: string;
  /** 0-based option; null clears the owner's answer. */
  option: number | null;
};

function fetchBenchCards(): Promise<BenchCards> {
  return invokeTauri<BenchCards>("bench_cards");
}

const benchCardsQueryOptions = {
  queryKey: BENCH_CARDS_QUERY_KEY,
  queryFn: fetchBenchCards,
  staleTime: BENCH_IDLE_STALE_MS,
  // A build or mock without the command must not retry on every row mount.
  retry: false,
  retryOnMount: false,
} as const;

// Holds timers only, never Bench data: every subscriber releases on unmount,
// so a community switch (which remounts the tree) leaves nothing behind.
const benchPoll = createPollGroup(BENCH_POLL_INTERVAL_MS, isDocumentVisible);

function refetchBenchCards(queryClient: QueryClient) {
  void queryClient.refetchQueries({
    queryKey: BENCH_CARDS_QUERY_KEY,
    exact: true,
  });
}

/** Keeps the snapshot fresh every 3 s while `active`, with one shared timer. */
function useBenchPolling(active: boolean) {
  const queryClient = useQueryClient();
  React.useEffect(() => {
    if (!active) return;
    return benchPoll.retain(() => refetchBenchCards(queryClient));
  }, [active, queryClient]);
}

/**
 * The Bench as state.json last recorded it, polled every 3 s while mounted
 * and the window is visible, plus live cards by event id and the answer
 * mutation.
 */
export function useBenchCards() {
  const query = useQuery(benchCardsQueryOptions);
  useBenchPolling(true);
  const snapshot = enabledBenchSnapshot(query.data);
  const cardsByEventId = React.useMemo(
    () => indexBenchCards(snapshot),
    [snapshot],
  );
  const answer = useBenchAnswer();
  return {
    snapshot,
    cardsByEventId,
    error: query.error,
    isPending: query.isPending,
    answer,
  };
}

/**
 * Answers a card with the owner's keycap (or clears it) through
 * `bench_answer`, then refreshes the snapshot. The Bench's own tick records
 * the answer, so callers wait for `recorded` to change, not for this.
 */
export function useBenchAnswer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ itemId, cardEventId, option }: BenchAnswerInput) =>
      invokeTauri<void>("bench_answer", { itemId, cardEventId, option }),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: BENCH_CARDS_QUERY_KEY }),
  });
}

export type BenchReplyInput = {
  itemId: string;
  /** The card the owner saw: the Bench refuses a reply on any other. */
  cardEventId: string;
  /** One line; the Bench refuses text it would read as an option. */
  text: string;
};

/**
 * The owner's one-line reply in the card's thread, through `bench_reply`
 * (the Bench's own relay and keys, like a tap): the Bench reads it as the
 * card's Other and tells the card's writer.
 */
export function sendBenchReply({
  itemId,
  cardEventId,
  text,
}: BenchReplyInput): Promise<void> {
  return invokeTauri<void>("bench_reply", { itemId, cardEventId, text });
}

/**
 * The sidebar's Bench badge: null while the Bench is not enabled (the entry
 * hides), else how many items need the owner. The sidebar is always mounted,
 * so it refreshes on its own slow timer; the Bench screen's 3 s poll and the
 * answer mutation refresh the same snapshot faster.
 */
export function useBenchNeedsYouCount(): number | null {
  const { data } = useQuery({
    ...benchCardsQueryOptions,
    select: benchNeedsYouCount,
    refetchInterval: BENCH_SIDEBAR_POLL_MS,
  });
  return data ?? null;
}

/**
 * The snapshot for surfaces that only label things (the Inbox), or null
 * while the Bench is off or unread. No poll of its own: it is as fresh as
 * the sidebar badge's 15 s timer or an open Bench screen keeps the cache.
 */
export function useBenchSnapshot(): BenchSnapshot | null {
  const { data } = useQuery({
    ...benchCardsQueryOptions,
    select: enabledBenchSnapshot,
  });
  return data ?? null;
}

/**
 * The live Bench card a timeline message is, or null. Only rows in #bench
 * or a card's channel poll; every other row reads the cached snapshot and
 * re-renders only if its own selection changes.
 */
export function useBenchCardForMessage(
  message: BenchMessageRef,
): BenchCardMatch | null {
  const { channelId, createdAt, id, kind, pending, signerPubkey, tags } =
    message;
  const candidate = isBenchCandidate(message);
  const select = React.useCallback(
    (cards: BenchCards): BenchRowState =>
      benchRowState(cards, {
        channelId,
        createdAt,
        id,
        kind,
        pending,
        signerPubkey,
        tags,
      }),
    [channelId, createdAt, id, kind, pending, signerPubkey, tags],
  );
  const { data } = useQuery({
    ...benchCardsQueryOptions,
    enabled: candidate,
    select,
  });
  useBenchPolling(data?.inBenchChannel === true);
  return data?.match ?? null;
}
