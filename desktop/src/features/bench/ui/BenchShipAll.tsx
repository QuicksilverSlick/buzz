import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  type BenchCard,
  benchPlainView,
} from "@/features/bench/lib/benchCards";
import {
  type ShipAllButton,
  type ShipAllCard,
  formatShipAllTime,
  runShipAll,
  shipAllGate,
  shipAllPick,
  shipAllRecorded,
  shipAllStillOk,
  shipAllTaps,
  shipAllTrains,
} from "@/features/bench/lib/benchShipAll";
import {
  readBenchSnapshot,
  useBenchAnswer,
  useBenchSnapshot,
} from "@/features/bench/useBenchCards";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";

/** What happens after the taps (design 1.2): the kit's own timings. */
const AFTER_TAPS = [
  "The steward reads a tap once it has stood 2 minutes, at its next tick (every 15 minutes).",
  "Trains then leave one at a time: up to 5 pull requests on buzz and Talos, 6 on Reset.",
  "Reset goes live about 35-45 minutes after a train leaves. A buzz desktop change also waits for a launch check.",
  "Each card closes when its pull request ships. To take one back, click its Ship it again.",
];

/** One run at a time across every bar, so no card is tapped twice at once. */
let running = false;

type Run =
  | { phase: "idle" }
  | { phase: "confirm"; cards: ShipAllCard[] }
  | { phase: "running"; total: number; step: number }
  | { phase: "done"; recorded: number; at: number; error: string | null };

/** "risk medium, undo easy", from the card's own plain fields. */
function riskUndo(card: BenchCard): string {
  const parts = benchPlainView(card)
    .risk.filter((r) => r.label !== "Reach")
    .map((r) => `${r.label.toLowerCase()} ${r.text.toLowerCase()}`);
  return parts.length > 0 ? parts.join(", ") : "risk and undo not stated";
}

/** Give me details: opens the digest's thread, as a card's Open thread does. */
function BenchShipAllDetailsButton({
  digest,
  channelId,
}: {
  digest: BenchCard;
  channelId: string | null;
}) {
  const { goChannel } = useAppNavigation();
  const channel = digest.channelId ?? channelId;
  const messageId = digest.cardEventId;
  if (!channel || !messageId) {
    return (
      <span className="self-center text-muted-foreground">
        Give me details opens once the digest is on the Bench.
      </span>
    );
  }
  return (
    <Button
      onClick={() => void goChannel(channel, { messageId })}
      size="sm"
      type="button"
      variant="outline"
    >
      Give me details
    </Button>
  );
}

/**
 * Ship all for one button: the digest card's (no read-first card) or Give me
 * details' (every waiting card). It taps the cards the steward's check
 * clears; otherwise it is off and says why and when that re-checks. Yes taps
 * each card's own Ship it in the steward's order, reading the Bench again
 * before each, and waits for its Recorded line before the next.
 */
export function BenchShipAllBar({
  button,
  digest,
}: {
  button: ShipAllButton;
  /** The digest card, for its Give me details. */
  digest?: BenchCard;
}) {
  const snapshot = useBenchSnapshot();
  const now = useNow(15_000);
  const queryClient = useQueryClient();
  const { mutateAsync: answer } = useBenchAnswer();
  const [run, setRun] = React.useState<Run>({ phase: "idle" });
  const reasonId = React.useId();
  if (!snapshot) return null;
  const gate = shipAllGate(snapshot, button, now);
  const { candidates, left } = shipAllPick(snapshot);
  // With none on this button, its reason already says they are all read first.
  const later = gate.left.filter((l) => l.readFirst).length;
  const readFirst =
    button === "digest" && gate.cards.length > 0
      ? candidates.filter((c) => c.readFirst).length + later
      : 0;
  // While it is off, its reasons already name this button's own cards.
  const others = left.length + (gate.enabled ? gate.left.length - later : 0);

  const ship = async (confirmed: ShipAllCard[]) => {
    if (running) {
      setRun({
        phase: "done",
        recorded: 0,
        at: Date.now(),
        error: "Another Ship all is running: wait for it to finish.",
      });
      return;
    }
    running = true;
    const taps = shipAllTaps(confirmed);
    setRun({ phase: "running", total: taps.length, step: 0 });
    try {
      const result = await runShipAll(taps, {
        answer,
        // Only what he was shown, and only while each is still waiting and clear.
        stillOk: async (tap) =>
          shipAllStillOk(
            await readBenchSnapshot(queryClient),
            tap,
            button,
            Date.now(),
          ),
        isRecorded: async (tap) =>
          shipAllRecorded(await readBenchSnapshot(queryClient), tap),
        onStep: (step) =>
          setRun({ phase: "running", total: taps.length, step }),
        sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      });
      const { recorded, error } = result;
      const rest =
        recorded + 1 < taps.length ? " The rest were not tapped." : "";
      const message =
        error &&
        `Stopped at ${recorded + 1} of ${taps.length}: ${error}${rest}`;
      setRun({ phase: "done", recorded, at: Date.now(), error: message });
      // Also outside this bar, which a new digest can replace mid-run.
      if (message) toast.error(message);
      else toast.success(`Recorded Ship it on ${recorded} of ${taps.length}.`);
    } finally {
      running = false;
    }
  };

  const busy = run.phase === "confirm" || run.phase === "running";
  const trains = shipAllTrains(snapshot);
  return (
    <section
      aria-label="Ship all"
      className="mt-2.5 space-y-1.5 text-xs"
      data-testid={`bench-ship-all-${button}`}
    >
      <div className="flex flex-wrap gap-1.5">
        <Button
          aria-describedby={gate.enabled ? undefined : reasonId}
          disabled={!gate.enabled || busy}
          onClick={() => setRun({ phase: "confirm", cards: gate.cards })}
          size="sm"
          type="button"
        >
          Ship all {gate.cards.length}
        </Button>
        {digest ? (
          <BenchShipAllDetailsButton
            channelId={snapshot.channelId}
            digest={digest}
          />
        ) : null}
      </div>
      {gate.enabled ? null : (
        <p
          className="text-muted-foreground [overflow-wrap:anywhere]"
          data-testid="bench-ship-all-reason"
          id={reasonId}
        >
          {gate.reasons.join(" ")}
        </p>
      )}
      {readFirst > 0 ? (
        <p className="text-muted-foreground">
          {readFirst === 1 ? "1 more is" : `${readFirst} more are`} read first
          (lane:hold, money, sign-in or database), so{" "}
          {readFirst === 1 ? "it ships" : "they ship"} from Give me details.
        </p>
      ) : null}
      {button === "digest" && others > 0 ? (
        <p className="text-muted-foreground">
          {others} more can't go on Ship all now: Give me details says why.
        </p>
      ) : null}
      {run.phase === "confirm" ? (
        <fieldset
          className="space-y-1.5 rounded-md border border-border p-2"
          data-testid="bench-ship-all-confirm"
        >
          <legend className="font-medium text-foreground">
            Ship these {run.cards.length}?
          </legend>
          <ol className="list-decimal space-y-0.5 pl-4 text-foreground">
            {run.cards.map((c) => (
              <li className="[overflow-wrap:anywhere]" key={c.card.id}>
                {c.card.title} · {riskUndo(c.card)}
                {c.readFirst ? " · read first" : ""}
              </li>
            ))}
          </ol>
          <p className="text-muted-foreground">
            One tap each, in this order, at the commit each card shows. A stack
            ships bottom first.
          </p>
          <div className="flex gap-1.5">
            <Button
              onClick={() => void ship(run.cards)}
              size="sm"
              type="button"
            >
              Yes, ship {run.cards.length}
            </Button>
            <Button
              autoFocus
              onClick={() => setRun({ phase: "idle" })}
              size="sm"
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
          </div>
        </fieldset>
      ) : null}
      <div aria-live="polite" data-testid="bench-ship-all-status">
        {run.phase === "running" ? (
          <p className="text-muted-foreground">
            Recording {run.step + 1} of {run.total}...
          </p>
        ) : null}
        {run.phase === "done" && run.recorded > 0 ? (
          <div className="space-y-0.5 text-muted-foreground">
            <p className="font-medium text-foreground">
              Recorded Ship it on {run.recorded}{" "}
              {run.recorded === 1 ? "card" : "cards"} at{" "}
              {formatShipAllTime(run.at)}.
            </p>
            {AFTER_TAPS.map((line) => (
              <p key={line}>{line}</p>
            ))}
            <p>Trains now: {trains.length ? trains.join("; ") : "none"}.</p>
          </div>
        ) : null}
        {run.phase === "done" && run.error ? (
          <p className="text-destructive [overflow-wrap:anywhere]" role="alert">
            {run.error}
          </p>
        ) : null}
      </div>
    </section>
  );
}
