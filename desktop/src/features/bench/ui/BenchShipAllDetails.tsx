import type { BenchCard as BenchCardData } from "@/features/bench/lib/benchCards";
import { benchCardMode } from "@/features/bench/lib/benchCards";
import {
  isStewardDigest,
  shipAllGate,
  shipAllPick,
} from "@/features/bench/lib/benchShipAll";
import { BenchCard } from "@/features/bench/ui/BenchCard";
import { BenchShipAllBar } from "@/features/bench/ui/BenchShipAll";
import {
  useBenchCardForMessage,
  useBenchSnapshot,
} from "@/features/bench/useBenchCards";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Give me details: under the steward's digest at the head of its thread,
 * every card Ship all would tap in full, each with its own buttons, then the
 * Ship all bar below the last of them. That Ship all includes read-first
 * cards, since each is shown here above it. Then what Ship all leaves out,
 * and why. Renders nothing for any other thread.
 */
export function BenchShipAllDetails({
  channelId,
  message,
  viewerPubkey,
}: {
  channelId: string | null;
  message: TimelineMessage;
  viewerPubkey?: string;
}) {
  const match = useBenchCardForMessage({ ...message, channelId });
  const snapshot = useBenchSnapshot();
  if (!match || match.retired || !snapshot || !isStewardDigest(match.card)) {
    return null;
  }
  const { ownerPubkey, maxCards } = snapshot;
  if (!benchCardMode(match.card, ownerPubkey, viewerPubkey).interactive) {
    return null;
  }
  const gate = shipAllGate(snapshot, "details", Date.now());
  const left = [...gate.left, ...shipAllPick(snapshot).left];
  const show = (card: BenchCardData) => (
    <BenchCard
      card={card}
      maxCards={maxCards}
      ownerPubkey={ownerPubkey}
      viewerPubkey={viewerPubkey}
    />
  );
  return (
    <section
      aria-label="Waiting on your tap"
      className="pb-2"
      data-testid="bench-ship-all-thread"
    >
      <div className="sticky top-0 z-10 bg-background pb-1 pt-2">
        <h3 className="text-sm font-semibold text-foreground">
          Waiting on your tap ({gate.cards.length + left.length})
        </h3>
        <p className="text-xs text-muted-foreground">
          Ship all is below the last card it ships, so you pass each one first.
        </p>
      </div>
      <ul className="mt-2 space-y-3">
        {gate.cards.map(({ card }) => (
          <li key={card.cardEventId ?? card.id}>{show(card)}</li>
        ))}
      </ul>
      <BenchShipAllBar button="details" />
      {left.length > 0 ? (
        <section aria-label="Not in Ship all" className="mt-4 space-y-3">
          <h4 className="text-sm font-semibold text-foreground">
            Not in Ship all
          </h4>
          <p className="text-xs text-muted-foreground">
            Each says why it is left out. {gate.recheck}
          </p>
          <ul className="space-y-3">
            {left.map(({ card, reason }) => (
              <li key={card.cardEventId ?? card.id}>
                <p
                  className="text-xs text-muted-foreground [overflow-wrap:anywhere]"
                  data-testid="bench-ship-all-left-reason"
                >
                  {reason}
                </p>
                {show(card)}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <p className="mt-3 text-xs text-muted-foreground">
        Typing in this thread ships nothing: the steward acts only on taps.
      </p>
    </section>
  );
}
