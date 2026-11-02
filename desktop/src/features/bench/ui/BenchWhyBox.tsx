import * as React from "react";

import {
  type BenchCard as BenchCardData,
  BENCH_REASON_MAX,
  benchAsksWhy,
} from "@/features/bench/lib/benchCards";
import { sendBenchReply } from "@/features/bench/useBenchCards";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

/** One item's Why? box: open after a hold-type tap, until sent or skipped. */
type BenchWhy = {
  /** The card tapped: the reason goes on that card only. */
  cardEventId: string;
  /** The tapped option the reason is for. */
  option: number;
  text: string;
  phase: "editing" | "sending" | "sent";
  error: string | null;
};

// Keyed by the Bench item id, outside React: the Bench screen moves an
// answered card to another section, which remounts it, and a card that
// closes or is replaced keeps its id, so an unsent reason never goes silently.
// ponytail: entries live for the app session, one per tapped item; prune
// them if that ever matters.
const whys = new Map<string, BenchWhy>();
const listeners = new Set<() => void>();

function setWhy(itemId: string, why: BenchWhy | null) {
  if (why) whys.set(itemId, why);
  else whys.delete(itemId);
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Tests: close every box. */
export function clearBenchWhys() {
  whys.clear();
}

/**
 * After an accepted option click: a tap on a hold-type option opens the box
 * (a retap of the same one keeps what was typed); any other click (another
 * option, an undo) closes it. The tap never waits on the box.
 */
export function benchWhyAfterClick(
  card: BenchCardData,
  cardEventId: string,
  target: number | null,
) {
  const label = target === null ? undefined : card.options[target];
  if (target === null || label === undefined || !benchAsksWhy(label)) {
    setWhy(card.id, null);
    return;
  }
  const prev = whys.get(card.id);
  const kept =
    prev?.phase === "editing" &&
    prev.option === target &&
    prev.cardEventId === cardEventId;
  setWhy(card.id, {
    cardEventId,
    option: target,
    text: kept ? prev.text : "",
    phase: "editing",
    error: null,
  });
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * The steward kit adds the reason to the card's PR as a comment. Its card ids
 * name the repo by its short name (`hold-reset-<pr>-...`). Only reset and talos
 * are known private: buzz is public, and a repo new to the lane gets the
 * warning until it is added here.
 */
function prCommentHint(card: BenchCardData): string {
  if (card.writer !== "steward") return "";
  return /^steward\/(?:hold|stack)-(?:reset|talos)-\d/.test(card.id)
    ? "It's added to the PR as a comment. "
    : "It's added to the PR as a comment; treat it as public. ";
}

/**
 * The optional one-line reason after a hold-type tap. Send posts it as the
 * owner's reply in the card's thread (`bench_reply`), which the Bench tells
 * the card's writer; Skip (or Escape) sends nothing more. Send waits until
 * the card records the tap, so the reason always goes beside that answer,
 * never instead of it. It says where the reason goes, stays as a sent line
 * once it went, and says so if the card changed before it could go.
 */
export function BenchWhyBox({
  card,
  live,
}: {
  card: BenchCardData;
  /** The owner can still answer this card here. */
  live: boolean;
}) {
  const why = React.useSyncExternalStore(
    subscribe,
    () => whys.get(card.id) ?? null,
  );
  const inputRef = React.useRef<HTMLInputElement>(null);
  const hintId = React.useId();
  const here = why !== null && live && why.cardEventId === card.cardEventId;
  const editing = here && why.phase === "editing";
  React.useEffect(() => {
    if (!editing) return;
    // Opened by a tap on this card, or this card remounted in another
    // section: never take focus from anywhere else.
    const active = document.activeElement;
    const host = inputRef.current?.closest("[data-bench-item]");
    if (!active || active === document.body || host?.contains(active)) {
      inputRef.current?.focus({ preventScroll: true });
    }
  }, [editing]);

  if (!why) return null;
  if (!here) {
    // Typed, never sent, and the card closed or was replaced.
    const lost = why.phase === "editing" ? why.text.trim() : "";
    if (!lost) return null;
    return (
      <div className="mt-2 text-xs text-muted-foreground" role="status">
        <p className="[overflow-wrap:anywhere]">
          {`This card changed before your reason went, so it was not sent: "${lost}"`}
        </p>
        <Button
          className="mt-1"
          onClick={() => setWhy(card.id, null)}
          size="sm"
          type="button"
          variant="ghost"
        >
          Dismiss
        </Button>
      </div>
    );
  }
  const told = card.repliesTold === true;
  if (why.phase === "sent") {
    return (
      <p
        className="mt-1.5 text-xs text-muted-foreground [overflow-wrap:anywhere]"
        data-testid="bench-card-why-sent"
        role="status"
      >
        {told
          ? `Your reason went to ${card.writer} as a reply on this card.`
          : "Your reason is a reply on this card."}
      </p>
    );
  }
  const sending = why.phase === "sending";
  const reason = why.text.trim();
  // The tick recorded this tap as the owner's: a reason sent earlier would
  // reach the writer as a reply instead of an answer, or beside another one.
  const recorded =
    card.recorded === why.option && (card.answer?.byOwner ?? true);
  // Back to the tapped option, so focus never falls to the page.
  const tapped = () =>
    inputRef.current
      ?.closest("[data-bench-item]")
      ?.querySelector<HTMLElement>(`[data-bench-option="${why.option}"]`);
  const close = () => {
    const back = tapped();
    setWhy(card.id, null);
    back?.focus();
  };
  // Only the attempt it started settles: a later tap may have closed it.
  const settle = (next: Partial<BenchWhy>) => {
    const now = whys.get(card.id);
    if (now?.phase === "sending" && now.option === why.option) {
      setWhy(card.id, { ...now, ...next });
    }
  };
  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!reason || sending || !recorded) return;
    const back = tapped();
    const box = inputRef.current?.closest("form");
    setWhy(card.id, { ...why, phase: "sending", error: null });
    sendBenchReply({
      itemId: card.id,
      cardEventId: why.cardEventId,
      text: reason,
    }).then(
      () => {
        settle({ phase: "sent" });
        const active = document.activeElement;
        if (!active || active === document.body || box?.contains(active)) {
          back?.focus();
        }
      },
      (error: unknown) => settle({ phase: "editing", error: errorText(error) }),
    );
  };
  return (
    <form className="mt-2" data-testid="bench-card-why" onSubmit={submit}>
      <div className="flex gap-1.5">
        <Input
          aria-describedby={hintId}
          aria-label="Why? (optional)"
          className="h-8 min-w-0 flex-1"
          disabled={sending}
          maxLength={BENCH_REASON_MAX}
          onChange={(event) =>
            setWhy(card.id, {
              ...why,
              text: event.target.value,
              error: null,
            })
          }
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            // The box owns this Escape, not the app's mark-as-read.
            event.preventDefault();
            close();
          }}
          placeholder="Why? (optional)"
          ref={inputRef}
          spellCheck
          value={why.text}
        />
        <Button
          disabled={!reason || sending || !recorded}
          size="sm"
          title={
            !recorded
              ? "Waiting for the card to record your tap"
              : reason
                ? undefined
                : "Type a reason to send, or skip"
          }
          type="submit"
        >
          {sending ? "Sending..." : "Send"}
        </Button>
        <Button
          disabled={sending}
          onClick={close}
          size="sm"
          type="button"
          variant="ghost"
        >
          Skip
        </Button>
      </div>
      <p
        className="mt-1 text-xs text-muted-foreground [overflow-wrap:anywhere]"
        id={hintId}
      >
        {told
          ? `Sent to ${card.writer} as a reply on this card.`
          : `Posted as a reply on this card; no agent is told for ${card.writer}.`}{" "}
        {prCommentHint(card)}
        {recorded
          ? "Skip, and only your tap goes."
          : "Send opens once the card records your tap."}
      </p>
      {why.error ? (
        <p
          className="mt-1 text-xs text-destructive [overflow-wrap:anywhere]"
          role="alert"
        >
          Couldn't send the reason: {why.error}
        </p>
      ) : null}
    </form>
  );
}
