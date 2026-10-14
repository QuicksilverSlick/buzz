/**
 * Pure logic for Bench decision cards in #bench and bound project channels:
 * the wire types of the `bench_cards` snapshot, the guard that decides which
 * timeline message is a live Bench card, who may answer it, chip/label
 * mapping, the link filter, and the reducer that tracks a click until the
 * Bench records it.
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
export type BenchNeeds = "you" | "delegable";
export type BenchAnswerRole = "owner" | "decider";
/** Who may answer a scoped card besides the owner's own keys. */
export type BenchAnswerer = {
  pk: string;
  name: string;
  role: BenchAnswerRole;
  agent: boolean;
};
/** 0-based option; answeredAt is the Bench clock in unix seconds. */
export type BenchAnswer = {
  option: number;
  answeredAt: number;
  /** The answerer's hex; "" on answers from before named answerers. */
  by: string;
  /** A named answerer; null for the owner's own keys. */
  byName: string | null;
  /** From the owner's keys, so the owner's click on it undoes it. */
  byOwner: boolean;
  source: "tap" | "reply";
};
/** The newest "Other": a free-text reply in the card's thread. */
export type BenchReply = {
  eventId: string;
  by: string;
  byName: string | null;
  role: BenchAnswerRole | null;
  at: number;
  hhmm: string;
};

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
  /** RFC 3339: when the Bench took this drop. */
  updatedAt: string;
  /** The live card's kind 9 id (a receipt's kept card); null when none. */
  cardEventId: string | null;
  answer: BenchAnswer | null;
  /** 0-based option the card text shows as Recorded; null = unanswered. */
  recorded: number | null;
  /** The card's channel; null for #bench. */
  channelId: string | null;
  project: string | null;
  /** Has the For and Recommended lines and Other. */
  scoped: boolean;
  needs: BenchNeeds;
  answerers: BenchAnswerer[];
  ownerName: string | null;
  /** 1-based. */
  recommend: number | null;
  recommender: string | null;
  reply: BenchReply | null;
  /** A receipt: the question as it closed; this card is its outcome. */
  request: BenchCard | null;
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
  /** The row sits in #bench or a card's channel, so the snapshot should
   * stay fresh. */
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

type BenchIndex = {
  cards: ReadonlyMap<string, BenchCard>;
  /** Bound channels that hold a card; #bench is the snapshot's own. */
  channels: ReadonlySet<string>;
};

const cardIndexCache = new WeakMap<BenchSnapshot, BenchIndex>();

function benchIndex(snapshot: BenchSnapshot): BenchIndex {
  const cached = cardIndexCache.get(snapshot);
  if (cached) return cached;
  const cards = new Map<string, BenchCard>();
  const channels = new Set<string>();
  const ambiguous = new Set<string>();
  for (const item of snapshot.items) {
    const id = item.cardEventId;
    if (!id) continue;
    if (item.channelId) channels.add(item.channelId);
    if (cards.has(id)) ambiguous.add(id);
    else cards.set(id, item);
  }
  for (const id of ambiguous) cards.delete(id);
  const index = { cards, channels };
  cardIndexCache.set(snapshot, index);
  return index;
}

/**
 * Live cards by their kind 9 event id. An id claimed by two items maps to
 * neither: the guard would rather render the message as plain chat.
 */
export function indexBenchCards(
  snapshot: BenchSnapshot | null | undefined,
): ReadonlyMap<string, BenchCard> {
  return snapshot ? benchIndex(snapshot).cards : new Map();
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
 * The guard. A message renders as a Bench card only when its id is a live
 * card's event id, it is in that card's own channel, and the Bench writer
 * key signed it. Everything else renders exactly as before.
 */
export function benchRowState(
  cards: BenchCards | null | undefined,
  message: BenchMessageRef,
): BenchRowState {
  const snapshot = enabledBenchSnapshot(cards);
  const channelId = message.channelId;
  const inCardChannel =
    snapshot &&
    (isBenchChannel(snapshot, channelId) ||
      (channelId && benchIndex(snapshot).channels.has(channelId)));
  if (!snapshot || !inCardChannel) return NOT_A_BENCH_ROW;
  if (!isBenchCandidate(message) || !snapshot.writerPubkey) {
    return { inBenchChannel: true, match: null };
  }
  const card = indexBenchCards(snapshot).get(message.id);
  const signer = normalizePubkey(message.signerPubkey ?? "");
  const signedByWriter = signer === normalizePubkey(snapshot.writerPubkey);
  const inItsChannel = (card?.channelId ?? snapshot.channelId) === channelId;
  return {
    inBenchChannel: true,
    match:
      card && inItsChannel && signedByWriter
        ? { card, ownerPubkey: snapshot.ownerPubkey }
        : null,
  };
}

function sameKey(a: string, b: string): boolean {
  return normalizePubkey(a) === normalizePubkey(b);
}

/**
 * Whether `pubkey` may answer the card: one of the owner's keys, or a named
 * answerer that is a human or answers a delegable card. item.rs `eligible`;
 * answer_vectors.json pins both.
 */
export function benchEligible(
  card: Pick<BenchCard, "answerers" | "needs">,
  pubkey: string,
  ownerKeys: readonly string[],
): boolean {
  return (
    ownerKeys.some((key) => sameKey(key, pubkey)) ||
    card.answerers.some(
      (a) => sameKey(a.pk, pubkey) && (!a.agent || card.needs === "delegable"),
    )
  );
}

/** "Craig", "Craig or Dana", "Craig, Dana or Quicksilver". */
function orList(names: readonly string[]): string {
  return names.length < 2
    ? (names[0] ?? "")
    : `${names.slice(0, -1).join(", ")} or ${names[names.length - 1]}`;
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
  // Only the Bench screen shows an item without a live card: an answered one
  // whose card was retired, or one the Bench has yet to (re)post.
  if (!card.cardEventId) {
    return readOnly(
      card.answer
        ? "This card is no longer live."
        : "Waiting for the Bench to post this card in #bench.",
    );
  }
  if (card.validity === "superseded") {
    return readOnly("Superseded: this card is being retired.");
  }
  if (card.validity === "suspected-stale") {
    return readOnly("May be stale: this card is being retired.");
  }
  if (!ownerPubkey) return readOnly("The Bench has no owner yet.");
  // bench_answer signs with the owner's key, so only the owner answers here.
  if (viewerPubkey !== undefined && !sameKey(viewerPubkey, ownerPubkey)) {
    if (benchEligible(card, viewerPubkey, [ownerPubkey])) {
      return readOnly("Answer from your phone.");
    }
    const names = card.answerers.map((a) => a.name);
    if (names.length === 0) {
      return readOnly("Only the Bench owner can answer this card.");
    }
    const owner = card.ownerName ?? "the owner";
    return readOnly(
      `For ${orList(names)}. Only ${orList([...names, owner])} can answer.`,
    );
  }
  return { interactive: true, readOnlyReason: null };
}

export type BenchChipVariant = "destructive" | "warning" | "info" | "success";

const SEVERITY_CHIPS: Record<
  BenchSeverity,
  { label: string; variant: BenchChipVariant }
> = {
  // An escalation to the owner; "Critical" is kept for real incidents.
  critical: { label: "Your decision", variant: "destructive" },
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

const ROLE_LABELS: Record<BenchAnswerRole, string> = {
  owner: "Owner",
  decider: "Decider",
};

/** "For Craig · Decider"; null when only the owner answers. */
export function benchForLabel(card: BenchCard): string | null {
  if (card.answerers.length === 0) return null;
  const who = card.answerers.map(
    (a) => `${a.name} · ${lookup(ROLE_LABELS, a.role) ?? a.role}`,
  );
  return `For ${who.join(", ")}`;
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

/** `byOwner`: the recorded answer came from the owner's own keys. */
export type BenchAnswerAction =
  | {
      type: "click";
      option: number;
      recorded: number | null;
      byOwner: boolean;
    }
  | { type: "commandFailed"; attempt: number; message: string }
  | { type: "timedOut"; attempt: number }
  | { type: "recorded"; recorded: number | null; byOwner: boolean };

export const INITIAL_BENCH_ANSWER_STATE: BenchAnswerState = {
  attempt: 0,
  phase: "idle",
  clicked: null,
  target: null,
  error: null,
};

/**
 * Clicking the owner's own recorded option undoes it; any other option, or
 * someone else's recorded one, answers as the owner.
 */
export function answerTarget(
  option: number,
  recorded: number | null,
  byOwner: boolean,
): number | null {
  return option === recorded && byOwner ? null : option;
}

/** A clear lands once the owner's answer is gone; another may then stand. */
function settle(
  state: BenchAnswerState,
  recorded: number | null,
  byOwner: boolean,
): BenchAnswerState {
  const landed =
    state.target === null
      ? recorded === null || !byOwner
      : recorded === state.target && byOwner;
  return state.phase !== "idle" && landed
    ? { ...state, phase: "idle", clicked: null, target: null, error: null }
    : state;
}

export function benchAnswerReducer(
  state: BenchAnswerState,
  action: BenchAnswerAction,
): BenchAnswerState {
  switch (action.type) {
    case "recorded":
      return settle(state, action.recorded, action.byOwner);
    case "click": {
      const base = settle(state, action.recorded, action.byOwner);
      if (base.phase === "saving") return base;
      return {
        attempt: base.attempt + 1,
        phase: "saving",
        clicked: action.option,
        target: answerTarget(action.option, action.recorded, action.byOwner),
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
  /** The writer's recommendation. */
  recommended: boolean;
  pending: boolean;
  disabled: boolean;
};

/**
 * The option the card shows as answered: what the live card's text records,
 * or, once the card is retired or deleted (the Bench forgets `recorded` then
 * and keeps `answer`), the Bench's answer. Such a card is never interactive,
 * so the reducer still settles on `recorded` alone.
 */
function shownOption(card: BenchCard): number | null {
  return (
    card.recorded ?? (card.cardEventId ? null : (card.answer?.option ?? null))
  );
}

export function benchOptionViews(
  card: BenchCard,
  state: BenchAnswerState,
  interactive: boolean,
): BenchOptionView[] {
  const saving = state.phase === "saving";
  const shown = shownOption(card);
  return card.options.map((text, index) => {
    const pending = saving && state.clicked === index;
    return {
      index,
      number: String(index + 1),
      text,
      selected: shown === index,
      recommended: card.recommend === index + 1,
      pending,
      disabled: !interactive || (saving && !pending),
    };
  });
}

/**
 * Who answered, as the card text names them (item.rs shown_name): a named
 * answerer, else the owner. Null on a legacy card, whose text names no one.
 */
function shownName(card: BenchCard, byName: string | null): string | null {
  return card.scoped ? (byName ?? card.ownerName ?? "the owner") : null;
}

function dotted(...parts: Array<string | null>): string {
  return parts.filter((part) => part !== null).join(" · ");
}

/**
 * "Recorded: 2. Ship it · 14:05" ("· Craig · 14:05" on a scoped card), or
 * null when nothing is recorded.
 */
export function benchRecordedLine(
  card: BenchCard,
  formatTime: (unixSeconds: number) => string,
): string | null {
  const recorded = shownOption(card);
  if (recorded === null || recorded < 0) return null;
  const text = card.options[recorded];
  if (text === undefined) return null;
  const line = `Recorded: ${recorded + 1}. ${text}`;
  const a = card.answer;
  return a?.option === recorded
    ? dotted(line, shownName(card, a.byName), formatTime(a.answeredAt))
    : line;
}

export type BenchReceipt = {
  /** The question as it closed. */
  request: BenchCard;
  /** "Answered: 2. Hold · Craig · 14:05", an Other reply, or none. */
  answeredLine: string;
  /** "Outcome · Done · 14:20". */
  outcomeLine: string;
};

/** A closed card (item.rs render_receipt): question, answer, outcome. */
export function benchReceipt(
  card: BenchCard,
  formatTime: (unixSeconds: number) => string,
): BenchReceipt | null {
  const request = card.request;
  if (!request) return null;
  const a = request.answer;
  const option = a ? request.options[a.option] : undefined;
  const reply = request.reply;
  let answered = "no answer recorded";
  if (a && option !== undefined) {
    answered = dotted(
      `${a.option + 1}. ${option}`,
      shownName(request, a.byName),
      formatTime(a.answeredAt),
    );
  } else if (reply) {
    answered = dotted(
      "Other, in this thread",
      shownName(request, reply.byName),
      formatTime(reply.at),
    );
  }
  const at = benchDeadlineSeconds(card.updatedAt);
  return {
    request,
    answeredLine: `Answered: ${answered}`,
    outcomeLine: dotted(
      "Outcome",
      card.kind === "status" ? benchStateLabel(card.state) : null,
      at === null ? null : formatTime(at),
    ),
  };
}

/** The keycaps the Bench seeds and reads as answers (item.rs OPTION_EMOJI). */
const BENCH_OPTION_KEYCAPS = ["1️⃣", "2️⃣", "3️⃣", "4️⃣"];

/**
 * Whether a reaction is one of the card's answer keycaps: the pills phones
 * tap, which the desktop card already shows as its option buttons. A
 * receipt shows its question's.
 */
export function isBenchAnswerKeycap(card: BenchCard, emoji: string): boolean {
  const index = BENCH_OPTION_KEYCAPS.indexOf(emoji);
  return index !== -1 && index < (card.request ?? card).options.length;
}

/** RFC 3339 (a deadline, updatedAt) to unix seconds; null if unparseable. */
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
