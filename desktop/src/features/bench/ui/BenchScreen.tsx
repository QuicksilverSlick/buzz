import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  type BenchCard as BenchCardData,
  benchSearch,
} from "@/features/bench/lib/benchCards";
import {
  type BenchSections,
  benchLastTickSeconds,
  benchSections,
} from "@/features/bench/lib/benchScreen";
import { BenchCard } from "@/features/bench/ui/BenchCard";
import { useBenchCards } from "@/features/bench/useBenchCards";
import { useIdentityQuery } from "@/shared/api/hooks";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import { Input } from "@/shared/ui/input";
import { PageHeader, SectionHeader } from "@/shared/ui/PageHeader";

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

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * The Bench as a full screen: what needs the owner, what waits on someone
 * else, what is answered and waiting on an agent, what was decided, and where
 * things stand. Cards answer through `bench_answer`, which uses the Bench's
 * own relay and keys, so the screen works whichever community is active.
 * Other and Open thread open the card's thread, where a reply is the answer.
 * Search narrows every section at once.
 */
export function BenchScreen() {
  const { snapshot, error, isPending } = useBenchCards();
  const viewerPubkey = useIdentityQuery().data?.pubkey;
  const { goChannel } = useAppNavigation();
  const [query, setQuery] = React.useState("");
  const matches = React.useMemo(() => benchSearch(query), [query]);
  const sections = React.useMemo(
    () => benchSections(snapshot, viewerPubkey),
    [snapshot, viewerPubkey],
  );
  const benchChannel = snapshot?.channelId ?? "";
  const openThread = React.useCallback(
    (card: BenchCardData) => {
      const channel = card.channelId ?? benchChannel;
      if (channel && card.cardEventId) {
        void goChannel(channel, { thread: card.cardEventId });
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
    content = (
      <>
        <div className="space-y-1 text-sm" data-testid="bench-status">
          <p className="text-muted-foreground" data-testid="bench-last-update">
            {lastTick === null
              ? "Waiting for the Bench's first update."
              : `Last update: ${formatItemTimestamp(lastTick, { withTime: true })}`}
          </p>
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
            key={section.key}
            onOther={openThread}
            ownerPubkey={snapshot.ownerPubkey}
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
  onOther,
  ownerPubkey,
  testId,
  title,
  viewerPubkey,
}: {
  cards: BenchCardData[];
  empty: string;
  onOther: (card: BenchCardData) => void;
  ownerPubkey: string | null;
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
            <li key={card.id}>
              <BenchCard
                card={card}
                onOther={() => onOther(card)}
                ownerPubkey={ownerPubkey}
                viewerPubkey={viewerPubkey}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
