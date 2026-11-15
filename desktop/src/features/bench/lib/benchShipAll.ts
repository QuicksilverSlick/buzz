/**
 * Ship all on the steward's digest (Desktop/Buzz/steward-digest-ship-all-design-
 * 2026-11-15.md, Russell's answers of 15 Nov 2026). Ship all taps each waiting
 * hold or stack card's own Ship it, one `bench_answer` at a time, so the kit
 * verifies every tap alone as today. It taps only the cards that both this
 * picks (the same rules as the kit's lane/shipall.mjs) and the steward's
 * fresh check (`steward/ship-all-check`, a status item the kit drops each
 * tick) lists as clear, card by card; every other waiting card says why.
 */

import type { BenchCard, BenchSnapshot } from "@/features/bench/lib/benchCards";

export type ShipAllButton = "digest" | "details";

/** The kit's digestCard ids: the morning's, then a daytime `-2` or `-3`. */
const DIGEST_ID = /^steward\/digest-\d{4}-\d{2}-\d{2}(?:-\d{1,2})?$/;
const HOLD_ID =
  /^steward\/hold-(reset|talos|buzz)-(\d{1,7})-([0-9a-f]{12})-([0-9a-f]{12})$/;
const STACK_ID =
  /^steward\/stack-(reset|talos|buzz)-(\d{1,7})-(\d{1,7})-([0-9a-f]{12})$/;
const SHIP_ACTION =
  /^Ship pull request (\d{1,7}) of ([A-Za-z0-9-]+\/[A-Za-z0-9.-]+) at commit ([0-9a-f]{40}) on top of [A-Za-z0-9./-]{1,80} at commit ([0-9a-f]{40})$/;
const STACK_ACTION =
  /^Ship as one stack the pull requests (\d{1,7}(?:, \d{1,7}){0,4}) of ([A-Za-z0-9-]+\/[A-Za-z0-9.-]+), list ([0-9a-f]{64})$/;
/** K1's marker: any other title reads as read first (lane:hold, money, sign-in or database). */
const TAP_TITLE =
  /^(buzz|reset|talos) (hotfix )?(PR \d{1,7}|stack \d{1,7}-\d{1,7}) waits for your tap: /;
/** lane/repos.json: the repo an action names, by its short name. */
const REPO_SHORT: Record<string, string> = {
  "QuicksilverSlick/Metabolic-reset": "reset",
  "QuicksilverSlick/dreamforge-talos": "talos",
  "QuicksilverSlick/buzz": "buzz",
};
const SHIP_LABEL = "Ship it";

export const SHIP_ALL_CHECK_ID = "steward/ship-all-check";
const CHECK_HEAD =
  "Ship all check v1: the steward's check for the desktop's Ship all buttons. It ships nothing.";
const ISO = "(\\d{4}-\\d\\d-\\d\\dT\\d\\d:\\d\\d:\\d\\d\\.\\d{3}Z)";
const CHECKED = new RegExp(`^checked: ${ISO}$`);
const NEXT = new RegExp(`^next: ${ISO}$`);
const CARD_LINE =
  /^card: (steward\/(?:hold|stack)-[a-z0-9-]{1,64}) (digest details|details)$/;
const HELD_LINE =
  /^held: (steward\/(?:hold|stack)-[a-z0-9-]{1,64}) (.{1,200})$/;

export function isStewardDigest(card: BenchCard): boolean {
  return (
    card.writer === "steward" &&
    card.kind === "pending" &&
    !card.request &&
    DIGEST_ID.test(card.id)
  );
}

/** A card Ship all can tap; `prs` as "reset 906", one per PR it ships. */
export type ShipAllCard = {
  card: BenchCard;
  prs: string[];
  readFirst: boolean;
};
export type ShipAllLeftOut = {
  card: BenchCard;
  reason: string;
  /** The steward's check reads it as read first now (a lane:hold added later). */
  readFirst?: boolean;
};

/** The PRs a card's id and action agree on, or null when they don't. */
function boundPrs(card: BenchCard): string[] | null {
  const hold = HOLD_ID.exec(card.id);
  if (hold) {
    const a = SHIP_ACTION.exec(card.action ?? "");
    const ok =
      a &&
      REPO_SHORT[a[2]] === hold[1] &&
      a[1] === hold[2] &&
      a[3].startsWith(hold[3]) &&
      a[4].startsWith(hold[4]);
    return ok ? [`${hold[1]} ${hold[2]}`] : null;
  }
  const stack = STACK_ID.exec(card.id);
  const a = STACK_ACTION.exec(card.action ?? "");
  if (!stack || !a) return null;
  const prs = a[1].split(", ");
  const ok =
    REPO_SHORT[a[2]] === stack[1] &&
    prs[0] === stack[2] &&
    prs.at(-1) === stack[3] &&
    a[3].startsWith(stack[4]);
  return ok ? prs.map((n) => `${stack[1]} ${n}`) : null;
}

/**
 * Every hold or stack card waiting on the owner's tap, split into what Ship
 * all can tap (the kit's candidates) and what it leaves out, with why.
 */
export function shipAllPick(snapshot: Pick<BenchSnapshot, "items">): {
  candidates: ShipAllCard[];
  left: ShipAllLeftOut[];
} {
  const picked: ShipAllCard[] = [];
  const left: ShipAllLeftOut[] = [];
  for (const card of snapshot.items) {
    const waiting =
      card.writer === "steward" &&
      card.kind === "pending" &&
      card.validity === "current" &&
      !card.request &&
      !card.answer &&
      /^steward\/(?:hold|stack)-/.test(card.id);
    if (!waiting) continue;
    const prs = boundPrs(card);
    const out = (reason: string) => left.push({ card, reason });
    if (card.reply) out("You wrote on its card: answer it there.");
    else if (!card.cardEventId) {
      out(
        card.waitsFor === "slot"
          ? "Not on the Bench yet (it waits for a card slot). It posts as answered cards free their slots, about 15 minutes after each answer."
          : "Not on the Bench yet: answer it on its card once it posts.",
      );
    } else if (
      !prs ||
      card.recommend === null ||
      card.options[card.recommend - 1] !== SHIP_LABEL
    ) {
      out(
        "Its card does not name the commits Ship all checks: answer it on its card.",
      );
    } else {
      picked.push({ card, prs, readFirst: !TAP_TITLE.test(card.title) });
    }
  }
  // Two cards naming one PR: one is being replaced, so neither is tapped.
  const count = new Map<string, number>();
  for (const pr of picked.flatMap((c) => c.prs)) {
    count.set(pr, (count.get(pr) ?? 0) + 1);
  }
  const candidates: ShipAllCard[] = [];
  for (const c of picked) {
    const twice = c.prs.find((pr) => count.get(pr) !== 1);
    if (!twice) candidates.push(c);
    else {
      const [repo, n] = twice.split(" ");
      left.push({
        card: c.card,
        reason: `Two open cards for ${repo} pull request ${n}: the steward closes one at its next tick.`,
      });
    }
  }
  return { candidates, left };
}

export type ShipAllCheck = {
  checked: number;
  next: number;
  clear: Record<ShipAllButton, boolean>;
  /** The clear cards, in the order Ship all taps them; `digest`: on the digest card's too. */
  cards: Array<{ id: string; digest: boolean }>;
  /** Each card the steward keeps off Ship all this tick, with why. */
  held: Map<string, string>;
  whys: Record<ShipAllButton, string[]>;
};

/**
 * The steward's check, line by line in the kit's order (README, Ship all's
 * check); any other line makes it unreadable. "missing" when the snapshot
 * holds no such item from the steward.
 */
export function readShipAllCheck(
  snapshot: Pick<BenchSnapshot, "items">,
): ShipAllCheck | "missing" | "unreadable" {
  const item = snapshot.items.find(
    (i) =>
      i.id === SHIP_ALL_CHECK_ID &&
      i.writer === "steward" &&
      i.kind === "status",
  );
  if (!item) return "missing";
  const lines = item.body.split("\n");
  const checked = CHECKED.exec(lines[1] ?? "")?.[1];
  const next = NEXT.exec(lines[2] ?? "")?.[1];
  const digest = /^digest: (clear|not clear)$/.exec(lines[3] ?? "")?.[1];
  const details = /^details: (clear|not clear)$/.exec(lines[4] ?? "")?.[1];
  if (lines[0] !== CHECK_HEAD || !checked || !next || !digest || !details) {
    return "unreadable";
  }
  const check: ShipAllCheck = {
    checked: Date.parse(checked),
    next: Date.parse(next),
    clear: { digest: digest === "clear", details: details === "clear" },
    cards: [],
    held: new Map(),
    whys: { digest: [], details: [] },
  };
  // Sections in order: cards, held, then the digest's whys, then the details'.
  let section = 0;
  for (const line of lines.slice(5)) {
    const card = CARD_LINE.exec(line);
    const held = HELD_LINE.exec(line);
    const why = /^why (digest|details): (.{1,200})$/.exec(line);
    const at = card ? 0 : held ? 1 : why?.[1] === "digest" ? 2 : why ? 3 : -1;
    if (at < section) return "unreadable";
    section = at;
    if (card) check.cards.push({ id: card[1], digest: card[2] !== "details" });
    else if (held) check.held.set(held[1], held[2]);
    else if (why) check.whys[why[1] as ShipAllButton].push(why[2]);
  }
  const ids = new Set(check.cards.map((c) => c.id));
  const named = ids.size + check.held.size;
  for (const id of check.held.keys()) ids.add(id);
  if (ids.size !== named || !Number.isFinite(check.checked)) {
    return "unreadable";
  }
  return Number.isFinite(check.next) ? check : "unreadable";
}

const HHMM = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
});
export function formatShipAllTime(ms: number): string {
  return HHMM.format(new Date(ms));
}
