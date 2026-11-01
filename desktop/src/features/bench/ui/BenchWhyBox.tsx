import * as React from "react";

import {
  type BenchCard as BenchCardData,
  BENCH_REASON_MAX,
  benchAsksWhy,
} from "@/features/bench/lib/benchCards";
import { sendBenchReply } from "@/features/bench/useBenchCards";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

/** One card's Why? box: open after a hold-type tap, until sent or skipped. */
type BenchWhy = {
  /** The tapped option the reason is for. */
  option: number;
  text: string;
  phase: "editing" | "sending" | "sent";
  error: string | null;
};

// Keyed by the card's event id, outside React: the Bench screen moves an
// answered card to another section, which remounts it, and an open box (or
// a send in flight) must survive that.
// ponytail: entries live for the app session, one per tapped card; prune
// them if that ever matters.
const whys = new Map<string, BenchWhy>();
const listeners = new Set<() => void>();

function setWhy(cardEventId: string, why: BenchWhy | null) {
  if (why) whys.set(cardEventId, why);
  else whys.delete(cardEventId);
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * After an accepted option click: a tap on a hold-type option opens a fresh
 * box; any other click (another option, an undo) closes it. The tap never
 * waits on the box.
 */
export function benchWhyAfterClick(
  card: BenchCardData,
  cardEventId: string,
  target: number | null,
) {
  const label = target === null ? undefined : card.options[target];
  setWhy(
    cardEventId,
    target !== null && label !== undefined && benchAsksWhy(label)
      ? { option: target, text: "", phase: "editing", error: null }
      : null,
  );
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * The optional one-line reason after a hold-type tap. Send posts it as the
 * owner's reply in the card's thread (`bench_reply`), which the Bench tells
 * the card's writer; Skip (or Escape) sends nothing more. It says where the
 * reason goes, and stays as a sent line once it went.
 */
export function BenchWhyBox({
  card,
  cardEventId,
}: {
  card: BenchCardData;
  cardEventId: string;
}) {
  const why = React.useSyncExternalStore(
    subscribe,
    () => whys.get(cardEventId) ?? null,
  );
  const inputRef = React.useRef<HTMLInputElement>(null);
  const editing = why?.phase === "editing";
  React.useEffect(() => {
    if (!editing) return;
    // Opened by a tap on this card, or this card remounted in another
    // section: never take focus from anywhere else.
    const active = document.activeElement;
    const here = inputRef.current?.closest("[data-bench-item]");
    if (!active || active === document.body || here?.contains(active)) {
      inputRef.current?.focus({ preventScroll: true });
    }
  }, [editing]);

  if (!why) return null;
  if (why.phase === "sent") {
    return (
      <p
        className="mt-1.5 text-xs text-muted-foreground [overflow-wrap:anywhere]"
        data-testid="bench-card-why-sent"
      >
        Your reason went to {card.writer} as a reply on this card.
      </p>
    );
  }
  const sending = why.phase === "sending";
  const reason = why.text.trim();
  // Only the attempt it started settles: a later tap may have closed it.
  const settle = (next: Partial<BenchWhy>) => {
    const now = whys.get(cardEventId);
    if (now?.phase === "sending" && now.option === why.option) {
      setWhy(cardEventId, { ...now, ...next });
    }
  };
  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!reason || sending) return;
    setWhy(cardEventId, { ...why, phase: "sending", error: null });
    sendBenchReply({ itemId: card.id, cardEventId, text: reason }).then(
      () => settle({ phase: "sent" }),
      (error: unknown) => settle({ phase: "editing", error: errorText(error) }),
    );
  };
  return (
    <form
      aria-label="Why? (optional)"
      className="mt-2"
      data-testid="bench-card-why"
      onSubmit={submit}
    >
      <div className="flex gap-1.5">
        <Input
          aria-label="Why? (optional)"
          className="h-8 min-w-0 flex-1"
          disabled={sending}
          maxLength={BENCH_REASON_MAX}
          onChange={(event) =>
            setWhy(cardEventId, {
              ...why,
              text: event.target.value,
              error: null,
            })
          }
          onKeyDown={(event) => {
            if (event.key === "Escape") setWhy(cardEventId, null);
          }}
          placeholder="Why? (optional)"
          ref={inputRef}
          spellCheck
          value={why.text}
        />
        <Button
          disabled={!reason || sending}
          size="sm"
          title={reason ? undefined : "Type a reason to send, or skip"}
          type="submit"
        >
          {sending ? "Sending..." : "Send"}
        </Button>
        <Button
          disabled={sending}
          onClick={() => setWhy(cardEventId, null)}
          size="sm"
          type="button"
          variant="ghost"
        >
          Skip
        </Button>
      </div>
      <p className="mt-1 text-xs text-muted-foreground [overflow-wrap:anywhere]">
        Sent to {card.writer} as a reply on this card. Skip, and only your tap
        goes.
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
