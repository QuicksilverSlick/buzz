/**
 * Pure logic for Bench decision cards in #bench: the wire types of the
 * `bench_cards` snapshot, the guard that decides which timeline message is a
 * live Bench card, chip/label mapping, the link filter, and the reducer that
 * tracks a click until the Bench records it.
 *
 * Everything an agent wrote (title, body, options, link labels) stays plain
 * text: callers render it as React text, never markdown or HTML.
 */

import { normalizePubkey } from "@/shared/lib/pubkey";

export type BenchSeverity = "critical" | "attention" | "info" | "good";
export type BenchItemKind = "pending" | "decision" | "status";
export type BenchStatusState =
  | "done"
  | "in-progress"
  | "active"
  | "planned"
  | "not-started"
  | "blocked"
  | "inactive";
export type BenchValidity = "current" | "suspected-stale" | "superseded";
export type BenchLink = { label: string; url: string };

export type BenchCard = {
  id: string;
  writer: string;
  kind: BenchItemKind;
  title: string;
  body: string;
  area: string;
  severity: BenchSeverity | null;
  options: string[];
  state: BenchStatusState | null;
  decidedBy: string | null;
  deadline: string | null;
  links: BenchLink[];
  order: number;
  validity: BenchValidity;
  /** The live card's kind 9 id; null when the item has no live card. */
  cardEventId: string | null;
  /** 0-based option; answeredAt is the Bench clock in unix seconds. */
  answer: { option: number; answeredAt: number } | null;
  /** 0-based option the card text shows as Recorded; null = unanswered. */
  recorded: number | null;
};

export type BenchSnapshot = {
  enabled: true;
  /** "" before the Bench's first tick. */
  channelId: string;
  writerPubkey: string;
  ownerPubkey: string | null;
  lastError: string | null;
  lastTick: string;
  items: BenchCard[];
};

export type BenchCards = { enabled: false } | BenchSnapshot;

export type BenchCardMatch = {
  card: BenchCard;
  ownerPubkey: string | null;
};

/** What a timeline row needs from the snapshot. */
export type BenchRowState = {
  /** The row sits in #bench, so the snapshot should stay fresh. */
  inBenchChannel: boolean;
  match: BenchCardMatch | null;
};

export type BenchMessageRef = {
  channelId: string | null | undefined;
  id: string;
  kind?: number;
  pending?: boolean;
  /** Raw event signer, not a relay-delegated display author. */
  signerPubkey?: string;
};

const KIND_STREAM_MESSAGE = 9;
const NOT_A_BENCH_ROW: BenchRowState = { inBenchChannel: false, match: null };

const cardIndexCache = new WeakMap<
  BenchSnapshot,
  ReadonlyMap<string, BenchCard>
>();

/**
 * Live cards by their kind 9 event id. An id claimed by two items maps to
 * neither: the guard would rather render the message as plain chat.
 */
export function indexBenchCards(
  snapshot: BenchSnapshot | null | undefined,
): ReadonlyMap<string, BenchCard> {
  if (!snapshot) return new Map();
  const cached = cardIndexCache.get(snapshot);
  if (cached) return cached;
  const index = new Map<string, BenchCard>();
  const ambiguous = new Set<string>();
  for (const item of snapshot.items) {
    const id = item.cardEventId;
    if (!id) continue;
    if (index.has(id)) ambiguous.add(id);
    else index.set(id, item);
  }
  for (const id of ambiguous) index.delete(id);
  cardIndexCache.set(snapshot, index);
  return index;
}

export function enabledBenchSnapshot(
  cards: BenchCards | null | undefined,
): BenchSnapshot | null {
  return cards?.enabled === true ? cards : null;
}

export function isBenchChannel(
  snapshot: BenchSnapshot | null | undefined,
  channelId: string | null | undefined,
): boolean {
  return Boolean(
    snapshot && snapshot.channelId !== "" && channelId === snapshot.channelId,
  );
}

/** Whether a row could be a Bench card at all, before reading the snapshot. */
export function isBenchCandidate(message: BenchMessageRef): boolean {
  return (
    Boolean(message.channelId) &&
    Boolean(message.signerPubkey) &&
    message.pending !== true &&
    (message.kind === undefined || message.kind === KIND_STREAM_MESSAGE)
  );
}

/**
 * The guard. A message renders as a Bench card only when it is in the
 * snapshot's channel, its id is a live card's event id, and the Bench writer
 * key signed it. Everything else renders exactly as before.
 */
export function benchRowState(
  cards: BenchCards | null | undefined,
  message: BenchMessageRef,
): BenchRowState {
  const snapshot = enabledBenchSnapshot(cards);
  if (!snapshot || !isBenchChannel(snapshot, message.channelId)) {
    return NOT_A_BENCH_ROW;
  }
  if (!isBenchCandidate(message) || !snapshot.writerPubkey) {
    return { inBenchChannel: true, match: null };
  }
  const card = indexBenchCards(snapshot).get(message.id);
  const signer = normalizePubkey(message.signerPubkey ?? "");
  const signedByWriter = signer === normalizePubkey(snapshot.writerPubkey);
  return {
    inBenchChannel: true,
    match:
      card && signedByWriter
        ? { card, ownerPubkey: snapshot.ownerPubkey }
        : null,
  };
}

export type BenchCardMode = {
  /** Option buttons answer the card. */
  interactive: boolean;
  /** Why a pending card's options are read-only. */
  readOnlyReason: string | null;
};

export function benchCardMode(
  card: BenchCard,
  ownerPubkey: string | null,
  viewerPubkey: string | undefined,
): BenchCardMode {
  if (card.kind !== "pending") {
    return { interactive: false, readOnlyReason: null };
  }
  const readOnly = (readOnlyReason: string) => ({
    interactive: false,
    readOnlyReason,
  });
  if (!card.cardEventId) return readOnly("This card is no longer live.");
  if (card.validity === "superseded") {
    return readOnly("Superseded: this card is being retired.");
  }
  if (card.validity === "suspected-stale") {
    return readOnly("May be stale: this card is being retired.");
  }
  if (!ownerPubkey) return readOnly("The Bench has no owner yet.");
  if (
    viewerPubkey !== undefined &&
    normalizePubkey(viewerPubkey) !== normalizePubkey(ownerPubkey)
  ) {
    return readOnly("Only the Bench owner can answer this card.");
  }
  return { interactive: true, readOnlyReason: null };
}

export type BenchChipVariant = "destructive" | "warning" | "info" | "success";

const SEVERITY_CHIPS: Record<
  BenchSeverity,
  { label: string; variant: BenchChipVariant }
> = {
  critical: { label: "Critical", variant: "destructive" },
  attention: { label: "Attention", variant: "warning" },
  info: { label: "Info", variant: "info" },
  good: { label: "Good", variant: "success" },
};

export function benchSeverityChip(
  severity: string | null | undefined,
): { label: string; variant: BenchChipVariant } | null {
  return severity && Object.hasOwn(SEVERITY_CHIPS, severity)
    ? SEVERITY_CHIPS[severity as BenchSeverity]
    : null;
}

const KIND_LABELS: Record<BenchItemKind, string> = {
  pending: "Pending",
  decision: "Decision",
  status: "Status",
};

const STATE_LABELS: Record<BenchStatusState, string> = {
  done: "Done",
  "in-progress": "In progress",
  active: "Active",
  planned: "Planned",
  "not-started": "Not started",
  blocked: "Blocked",
  inactive: "Inactive",
};

const VALIDITY_LABELS: Record<BenchValidity, string | null> = {
  current: null,
  "suspected-stale": "May be stale",
  superseded: "Superseded",
};

function lookup<K extends string>(
  table: Record<K, string | null>,
  key: string | null | undefined,
): string | null {
  return key && Object.hasOwn(table, key) ? table[key as K] : null;
}

export function benchKindLabel(kind: string): string | null {
  return lookup(KIND_LABELS, kind);
}

export function benchStateLabel(state: string | null): string | null {
  return lookup(STATE_LABELS, state);
}

export function benchValidityLabel(validity: string): string | null {
  return lookup(VALIDITY_LABELS, validity);
}

export type SafeBenchLink = BenchLink & { scheme: "https" | "buzz" };

/**
 * Only https (without credentials) and buzz:// URLs survive; anything with
 * whitespace or control characters, or any other scheme (javascript:, data:,
 * http:, file:) is dropped. The Bench validates links too; this is the
 * renderer's own check.
 */
export function safeBenchLinks(
  links: readonly BenchLink[] | null | undefined,
): SafeBenchLink[] {
  const out: SafeBenchLink[] = [];
  for (const link of links ?? []) {
    if (typeof link?.url !== "string" || typeof link.label !== "string") {
      continue;
    }
    const label = link.label.trim();
    const scheme = linkScheme(link.url);
    if (label && scheme) out.push({ label, url: link.url, scheme });
  }
  return out;
}

function linkScheme(url: string): SafeBenchLink["scheme"] | null {
  // Printable ASCII only (no spaces, controls or look-alike Unicode), and
  // like the Bench itself no '@' or '\', which URL parsers read as
  // credentials or as '/'.
  if (!/^[\x21-\x7e]+$/.test(url) || /[@\\]/.test(url)) return null;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.username || parsed.password) return null;
  // The raw text must already name the host: `https:host` and `https:///host`
  // parse, but only after the parser repairs them.
  if (parsed.protocol === "https:" && /^https:\/\/[^/]/.test(url)) {
    return parsed.hostname ? "https" : null;
  }
  if (parsed.protocol === "buzz:" && url.startsWith("buzz://")) return "buzz";
  return null;
}

/** How long a click waits for the snapshot to show it recorded. */
export const BENCH_SAVE_TIMEOUT_MS = 15_000;
export const BENCH_SAVE_TIMEOUT_MESSAGE =
  "Not recorded after 15 s. The Bench may be offline; try again.";

export type BenchAnswerState = {
  /** Bumped on every accepted click, so a late timer or reply is ignored. */
  attempt: number;
  phase: "idle" | "saving" | "failed";
  /** The option button that was clicked. */
  clicked: number | null;
  /** The `recorded` value the click is waiting for (null = cleared). */
  target: number | null;
  error: string | null;
};

export type BenchAnswerAction =
  | { type: "click"; option: number; recorded: number | null }
  | { type: "commandFailed"; attempt: number; message: string }
  | { type: "timedOut"; attempt: number }
  | { type: "recorded"; recorded: number | null };

export const INITIAL_BENCH_ANSWER_STATE: BenchAnswerState = {
  attempt: 0,
  phase: "idle",
  clicked: null,
  target: null,
  error: null,
};

/** Clicking the recorded option undoes it; any other option answers. */
export function answerTarget(
  option: number,
  recorded: number | null,
): number | null {
  return option === recorded ? null : option;
}

function settle(
  state: BenchAnswerState,
  recorded: number | null,
): BenchAnswerState {
  return state.phase !== "idle" && recorded === state.target
    ? { ...state, phase: "idle", clicked: null, target: null, error: null }
    : state;
}

export function benchAnswerReducer(
  state: BenchAnswerState,
  action: BenchAnswerAction,
): BenchAnswerState {
  switch (action.type) {
    case "recorded":
      return settle(state, action.recorded);
    case "click": {
      const base = settle(state, action.recorded);
      if (base.phase === "saving") return base;
      return {
        attempt: base.attempt + 1,
        phase: "saving",
        clicked: action.option,
        target: answerTarget(action.option, action.recorded),
        error: null,
      };
    }
    case "commandFailed":
    case "timedOut": {
      if (state.phase !== "saving" || state.attempt !== action.attempt) {
        return state;
      }
      return {
        ...state,
        phase: "failed",
        error:
          action.type === "timedOut"
            ? BENCH_SAVE_TIMEOUT_MESSAGE
            : `Couldn't save: ${action.message}`,
      };
    }
  }
}

export type BenchOptionView = {
  index: number;
  /** "1", "2", ... like the phone's keycaps. */
  number: string;
  text: string;
  selected: boolean;
  pending: boolean;
  disabled: boolean;
};

export function benchOptionViews(
  card: BenchCard,
  state: BenchAnswerState,
  interactive: boolean,
): BenchOptionView[] {
  const saving = state.phase === "saving";
  return card.options.map((text, index) => {
    const pending = saving && state.clicked === index;
    return {
      index,
      number: String(index + 1),
      text,
      selected: card.recorded === index,
      pending,
      disabled: !interactive || (saving && !pending),
    };
  });
}

/** "Recorded: 2. Ship it · 14:05", or null when nothing is recorded. */
export function benchRecordedLine(
  card: BenchCard,
  formatTime: (unixSeconds: number) => string,
): string | null {
  const recorded = card.recorded;
  if (recorded === null || recorded < 0) return null;
  const text = card.options[recorded];
  if (text === undefined) return null;
  const line = `Recorded: ${recorded + 1}. ${text}`;
  return card.answer?.option === recorded
    ? `${line} · ${formatTime(card.answer.answeredAt)}`
    : line;
}

/** RFC 3339 deadline to unix seconds; null when it does not parse. */
export function benchDeadlineSeconds(deadline: string | null): number | null {
  if (!deadline) return null;
  const ms = Date.parse(deadline);
  return Number.isFinite(ms) ? Math.floor(ms / 1_000) : null;
}

type IntervalApi = {
  setInterval: (callback: () => void, ms: number) => unknown;
  clearInterval: (id: unknown) => void;
};

/**
 * One timer for any number of subscribers. React Query gives every observer
 * its own refetchInterval timer, so N rows polling one key would fetch N
 * times per interval; rows retain this instead. Each tick calls the oldest
 * live subscriber's refetch, and only while `isVisible()`.
 */
export function createPollGroup(
  intervalMs: number,
  isVisible: () => boolean,
  timers: IntervalApi = {
    setInterval: (callback, ms) => globalThis.setInterval(callback, ms),
    clearInterval: (id) =>
      globalThis.clearInterval(id as ReturnType<typeof setInterval>),
  },
) {
  const subscribers = new Set<{ refetch: () => void }>();
  let timer: unknown = null;
  return {
    retain(refetch: () => void): () => void {
      const subscriber = { refetch };
      subscribers.add(subscriber);
      if (timer === null) {
        timer = timers.setInterval(() => {
          if (!isVisible()) return;
          subscribers.values().next().value?.refetch();
        }, intervalMs);
      }
      return () => {
        subscribers.delete(subscriber);
        if (subscribers.size === 0 && timer !== null) {
          timers.clearInterval(timer);
          timer = null;
        }
      };
    },
    get size() {
      return subscribers.size;
    },
  };
}
