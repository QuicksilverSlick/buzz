/**
 * Pure logic for the Bench screen and its sidebar badge: which section each
 * item belongs in, in the order the pinned #bench board uses (item.rs
 * render_board), and how many items need the owner.
 */

import {
  type BenchCard,
  type BenchCards,
  type BenchSeverity,
  type BenchSnapshot,
  enabledBenchSnapshot,
} from "@/features/bench/lib/benchCards";

export type BenchSections = {
  /** Pending, current and unanswered: by severity, then order. */
  needsYou: BenchCard[];
  /** Pending and current with an answer: oldest answer first. */
  answered: BenchCard[];
  /** Current decisions: by area, then order. */
  decided: BenchCard[];
  /** Current status items: by area, then order. */
  status: BenchCard[];
  /** Stale or superseded items. The board folds them; so does the screen. */
  retiring: number;
};

const SEVERITY_RANK: Record<BenchSeverity, number> = {
  critical: 0,
  attention: 1,
  info: 2,
  good: 3,
};
const UNRATED_RANK = 4;

/** item.rs severity_rank: critical first, no severity last. */
export function benchSeverityRank(severity: string | null): number {
  return severity && Object.hasOwn(SEVERITY_RANK, severity)
    ? SEVERITY_RANK[severity as BenchSeverity]
    : UNRATED_RANK;
}

/** Code-unit order, like Rust's String ordering for the ASCII ids. */
function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function byKeys(...keys: Array<(card: BenchCard) => number | string>) {
  return (a: BenchCard, b: BenchCard) => {
    for (const key of keys) {
      const x = key(a);
      const y = key(b);
      const order =
        typeof x === "number" && typeof y === "number"
          ? x - y
          : compareText(String(x), String(y));
      if (order !== 0) return order;
    }
    return 0;
  };
}

const byNeed = byKeys(
  (card) => benchSeverityRank(card.severity),
  (card) => card.order,
  (card) => card.id,
);
const byAnswerTime = byKeys(
  (card) => card.answer?.answeredAt ?? 0,
  (card) => card.id,
);
const byArea = byKeys(
  (card) => card.area,
  (card) => card.order,
  (card) => card.id,
);

const EMPTY_SECTIONS: BenchSections = {
  needsYou: [],
  answered: [],
  decided: [],
  status: [],
  retiring: 0,
};

export function benchSections(
  snapshot: BenchSnapshot | null | undefined,
): BenchSections {
  if (!snapshot) return EMPTY_SECTIONS;
  const sections: BenchSections = {
    needsYou: [],
    answered: [],
    decided: [],
    status: [],
    retiring: 0,
  };
  for (const card of snapshot.items) {
    if (card.validity !== "current") {
      sections.retiring += 1;
    } else if (card.kind === "pending") {
      (card.answer ? sections.answered : sections.needsYou).push(card);
    } else if (card.kind === "decision") {
      sections.decided.push(card);
    } else if (card.kind === "status") {
      sections.status.push(card);
    }
  }
  sections.needsYou.sort(byNeed);
  sections.answered.sort(byAnswerTime);
  sections.decided.sort(byArea);
  sections.status.sort(byArea);
  return sections;
}

/**
 * The sidebar badge: null while the Bench is not enabled, otherwise the size
 * of the screen's "Needs you" section (the board's "needs you" count).
 */
export function benchNeedsYouCount(
  cards: BenchCards | null | undefined,
): number | null {
  const snapshot = enabledBenchSnapshot(cards);
  return snapshot ? benchSections(snapshot).needsYou.length : null;
}

/** The Bench's last tick (RFC 3339) in unix seconds; null before the first. */
export function benchLastTickSeconds(lastTick: string): number | null {
  if (!lastTick) return null;
  const ms = Date.parse(lastTick);
  return Number.isFinite(ms) ? Math.floor(ms / 1_000) : null;
}
