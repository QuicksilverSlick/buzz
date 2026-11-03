import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useViewerAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import {
  type BenchCard as BenchCardData,
  benchQueuedLine,
  benchSearch,
} from "@/features/bench/lib/benchCards";
import {
  type BenchSections,
  benchLastTickSeconds,
  benchSections,
} from "@/features/bench/lib/benchScreen";
import {
  type BenchTab,
  benchTabs,
  cardWorkbench,
  otherReason,
  splitBenchTabs,
  workbenchColor,
  workbenchCounts,
  workbenchLabel,
  workbenchSnapshot,
  workbenches,
} from "@/features/bench/lib/workbenches";
import { BenchCard } from "@/features/bench/ui/BenchCard";
import { ColorKeyButton } from "@/features/color-key/ui/ColorKey";
import { useBenchCards } from "@/features/bench/useBenchCards";
import { useProjectsQuery } from "@/features/projects/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { cn } from "@/shared/lib/cn";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { PageHeader, SectionHeader } from "@/shared/ui/PageHeader";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";

type SectionKey = Exclude<keyof BenchSections, "retiring">;

const SECTIONS: ReadonlyArray<{
  key: SectionKey;
  title: string;
  empty: string;
}> = [
  {
    key: "needsYou",
    title: "Needs you",
    empty: "Nothing needs you right now.",
  },
  {
    key: "waitingOnOthers",
    title: "Waiting on others",
    empty: "No card is waiting on anyone else.",
  },
  {
    key: "answered",
    title: "Answered, waiting for action",
    empty: "Nothing you answered is waiting on an agent.",
  },
  { key: "decided", title: "Decided", empty: "No decisions yet." },
  { key: "held", title: "Held", empty: "Nothing is on hold." },
  {
    key: "status",
    title: "Where things stand",
    empty: "No status updates yet.",
  },
];

/** The last tab on this device; All the first time or if unreadable. A tab
 * the registry no longer has shows All, and comes back if it returns. */
const BENCH_TAB_KEY = "buzz.bench.workbench.v1";

function readBenchTab(): BenchTab {
  return getStorageItem(BENCH_TAB_KEY) ?? "all";
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function needsYouText(count: number): string {
  return count === 1 ? "1 needs you" : `${count} need you`;
}

/**
 * The Bench as a full screen: what needs the owner, what waits on someone
 * else, what is answered and waiting on an agent, what was decided, and where
 * things stand. Cards answer through `bench_answer`, which uses the Bench's
 * own relay and keys, so the screen works whichever community is active.
 * Other and Open thread open the card's thread, where a reply is the answer.
 * Project tabs (All, one per registry project, and Other when it has
 * cards) narrow the board first; search then narrows every section at once.
 * The sidebar badge still counts every project. `focusCardId` (global
 * search's ?card=) shows All, clears the search, and scrolls to and rings
 * that card for a few seconds.
 */
export function BenchScreen({ focusCardId }: { focusCardId?: string } = {}) {
  const { snapshot, error, isPending } = useBenchCards();
  const viewerPubkey = useIdentityQuery().data?.pubkey;
  const { data: projects } = useProjectsQuery();
  const viewerAgents = useViewerAgentPubkeys();
  const { goBench, goChannel } = useAppNavigation();
  const [query, setQuery] = React.useState("");
  const matches = React.useMemo(() => benchSearch(query), [query]);
  const [savedTab, setSavedTab] = React.useState(readBenchTab);
  // A new object per pick, so the same card picked again scrolls again.
  const [focused, setFocused] = React.useState<{ id: string } | null>(null);
  const focusedId = focused?.id ?? null;
  React.useEffect(() => {
    if (!focusCardId) return;
    // All for this visit only: the saved tab stays the owner's.
    setSavedTab("all");
    setQuery("");
    setFocused({ id: focusCardId });
    // Drop ?card= so picking the same card again focuses it again.
    void goBench({ replace: true });
  }, [focusCardId, goBench]);
  const hasSnapshot = snapshot !== null;
  React.useEffect(() => {
    if (!focused || !hasSnapshot) return;
    const frame = window.requestAnimationFrame(() => {
      const item = document
        .querySelector(`[data-bench-item="${CSS.escape(focused.id)}"]`)
        ?.closest("li");
      item?.scrollIntoView({ block: "center" });
      item?.focus({ preventScroll: true });
    });
    const timer = window.setTimeout(() => setFocused(null), 4_000);
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
    };
  }, [focused, hasSnapshot]);
  const counts = React.useMemo(
    () => (snapshot ? workbenchCounts(snapshot, viewerPubkey) : null),
    [snapshot, viewerPubkey],
  );
  const tabs = benchTabs().filter(
    (t) => t.id !== "other" || (counts?.other.cards ?? 0) > 0,
  );
  // A saved Other tab that has emptied shows All, and comes back with cards.
  const tab = tabs.some((t) => t.id === savedTab) ? savedTab : "all";
  const sections = React.useMemo(
    () =>
      benchSections(
        snapshot ? workbenchSnapshot(snapshot, tab) : null,
        viewerPubkey,
      ),
    [snapshot, tab, viewerPubkey],
  );
  const benchChannel = snapshot?.channelId ?? "";
  const openThread = React.useCallback(
    (card: BenchCardData) => {
      const channel = card.channelId ?? benchChannel;
      if (channel && card.cardEventId) {
        // A message target opens its thread panel even when the card is
        // older than the loaded timeline; `thread` alone did not.
        void goChannel(channel, { messageId: card.cardEventId });
      }
    },
    [benchChannel, goChannel],
  );

  let content: React.ReactNode;
  if (!snapshot) {
    content = isPending ? (
      <p className="text-sm text-muted-foreground">Loading the Bench...</p>
    ) : error ? (
      <p className="text-sm text-destructive" role="alert">
        Couldn't read the Bench: {errorText(error)}
      </p>
    ) : (
      <p className="text-sm text-muted-foreground" data-testid="bench-disabled">
        The Bench isn't enabled on this computer.
      </p>
    );
  } else {
    const lastTick = benchLastTickSeconds(snapshot.lastTick);
    // Every tab's cards: the cap holds back the whole Bench, not one tab.
    const queuedLine = benchQueuedLine(snapshot);
    const otherLine =
      tab === "other"
        ? otherReason(workbenchSnapshot(snapshot, "other").items)
        : null;
    const { inline: inlineTabs, more: moreTabs } = splitBenchTabs(
      tabs,
      tab,
      counts,
    );
    const moreNeedsYou = moreTabs.reduce(
      (sum, t) => sum + (counts?.[t.id]?.needsYou ?? 0),
      0,
    );
    const pickTab = (next: BenchTab) => {
      setSavedTab(next);
      setStorageItem(BENCH_TAB_KEY, next);
    };
    const tabBody = (id: BenchTab, label: string) => {
      const count = counts?.[id];
      const workbench = workbenches().find((w) => w.id === id);
      const color = workbench
        ? workbenchColor(workbench, projects ?? [], viewerPubkey, viewerAgents)
        : null;
      return (
        <>
          {color ? (
            <span
              aria-hidden
              className={cn("size-2 shrink-0 rounded-full", color.dot)}
              data-project-color={color.id}
            />
          ) : null}
          {label}
          <span className="font-normal tabular-nums text-muted-foreground">
            {count?.cards ?? 0}
          </span>
          {count && count.needsYou > 0 ? (
            <span className="text-xs font-medium text-amber-700 dark:text-amber-400">
              <span aria-hidden className="text-muted-foreground">
                ·{" "}
              </span>
              {needsYouText(count.needsYou)}
            </span>
          ) : null}
        </>
      );
    };
    content = (
      <>
        <div className="flex flex-wrap items-center gap-2">
          <Tabs
            onValueChange={(value) => pickTab(value as BenchTab)}
            value={tab}
          >
            <TabsList
              aria-label="Projects"
              className="h-auto flex-wrap justify-start"
            >
              {inlineTabs.map(({ id, label, parent }) => (
                <TabsTrigger
                  className="gap-1.5"
                  data-testid={`bench-tab-${id}`}
                  key={id}
                  title={
                    parent
                      ? `${label}, part of ${workbenchLabel(parent)}`
                      : undefined
                  }
                  value={id}
                >
                  {tabBody(id, label)}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
          {moreTabs.length > 0 ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  data-testid="bench-tab-more"
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  More ({moreTabs.length})
                  {moreNeedsYou > 0 ? ` · ${needsYouText(moreNeedsYou)}` : ""}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                {moreTabs.map(({ id, label }) => (
                  <DropdownMenuItem
                    className="gap-1.5"
                    key={id}
                    onSelect={() => pickTab(id)}
                  >
                    {tabBody(id, label)}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
        </div>
        <div className="space-y-1 text-sm" data-testid="bench-status">
          {snapshot.projectsNotice ? (
            <p
              className="text-amber-700 dark:text-amber-400 [overflow-wrap:anywhere]"
              data-testid="bench-projects-notice"
              role="status"
            >
              Project tabs: {snapshot.projectsNotice}
            </p>
          ) : null}
          {otherLine ? (
            <p
              className="text-muted-foreground [overflow-wrap:anywhere]"
              data-testid="bench-other-reason"
            >
              {otherLine}
            </p>
          ) : null}
          <p className="text-muted-foreground" data-testid="bench-last-update">
            {lastTick === null
              ? "Waiting for the Bench's first update."
              : `Last update: ${formatItemTimestamp(lastTick, { withTime: true })}`}
          </p>
          {queuedLine ? (
            <p
              className="text-amber-700 dark:text-amber-400 [overflow-wrap:anywhere]"
              data-testid="bench-queued"
              role="status"
            >
              {queuedLine}
            </p>
          ) : null}
          {snapshot.lastError ? (
            <p
              className="text-destructive [overflow-wrap:anywhere]"
              data-testid="bench-last-error"
              role="alert"
            >
              The Bench's last update failed: {snapshot.lastError}
            </p>
          ) : null}
          {error ? (
            <p
              className="text-destructive [overflow-wrap:anywhere]"
              role="alert"
            >
              Couldn't refresh the Bench: {errorText(error)}
            </p>
          ) : null}
        </div>
        <Input
          aria-label="Search the Bench"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search by title, PR number, project, name or date"
          type="search"
          value={query}
        />
        {SECTIONS.map((section) => (
          <BenchSection
            cards={sections[section.key].filter(matches)}
            empty={query.trim() ? "No matching cards." : section.empty}
            focusedId={focusedId}
            key={section.key}
            maxCards={snapshot.maxCards}
            onOther={openThread}
            ownerPubkey={snapshot.ownerPubkey}
            showProject={tab === "all"}
            testId={`bench-section-${section.key}`}
            title={section.title}
            viewerPubkey={viewerPubkey}
          />
        ))}
        {sections.retiring > 0 ? (
          <p className="text-xs text-muted-foreground">
            {sections.retiring === 1
              ? "1 stale or superseded item is being retired and isn't shown."
              : `${sections.retiring} stale or superseded items are being retired and aren't shown.`}
          </p>
        ) : null}
      </>
    );
  }

  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      data-testid="bench-screen"
    >
      <div
        className="flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8"
        data-scroll-restoration-id="bench"
      >
        <div className="mx-auto w-full max-w-3xl space-y-8">
          <PageHeader
            action={<ColorKeyButton />}
            description="Decisions and status from your agents."
            title="Bench"
          />
          {content}
        </div>
      </div>
    </div>
  );
}

function BenchSection({
  cards,
  empty,
  focusedId,
  maxCards,
  onOther,
  ownerPubkey,
  showProject,
  testId,
  title,
  viewerPubkey,
}: {
  cards: BenchCardData[];
  empty: string;
  /** Global search's pick, ringed until it fades. */
  focusedId: string | null;
  maxCards: number | undefined;
  onOther: (card: BenchCardData) => void;
  ownerPubkey: string | null;
  /** The All tab names each card's project; a project tab already does. */
  showProject: boolean;
  testId: string;
  title: string;
  viewerPubkey: string | undefined;
}) {
  return (
    <section aria-label={title} className="space-y-3" data-testid={testId}>
      <SectionHeader
        title={
          <>
            {title}{" "}
            <span className="font-normal tabular-nums text-muted-foreground">
              {cards.length}
            </span>
          </>
        }
      />
      {cards.length === 0 ? (
        <p className="text-sm text-muted-foreground">{empty}</p>
      ) : (
        <ul className="space-y-3">
          {cards.map((card) => (
            // A replaced card is a new row: nothing typed or saving on the
            // card the owner saw carries over to the one that replaced it.
            <li
              className={cn(
                "outline-none",
                card.id === focusedId &&
                  "[&>[data-bench-item]]:ring-2 [&>[data-bench-item]]:ring-primary/60 [&>[data-bench-item]]:ring-offset-2 [&>[data-bench-item]]:ring-offset-background",
              )}
              data-bench-focused={card.id === focusedId || undefined}
              key={card.cardEventId ?? card.id}
              tabIndex={-1}
            >
              <BenchCard
                card={card}
                maxCards={maxCards}
                onOther={() => onOther(card)}
                ownerPubkey={ownerPubkey}
                projectLabel={
                  showProject ? workbenchLabel(cardWorkbench(card)) : undefined
                }
                viewerPubkey={viewerPubkey}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
