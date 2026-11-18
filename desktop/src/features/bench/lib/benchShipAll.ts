/**
 * Ship all on the steward's digest (Desktop/Buzz/steward-digest-ship-all-design-
 * 2026-11-15.md, Russell's answers of 15 Nov 2026). Ship all taps each waiting
 * hold or stack card's own Ship it, one `bench_answer` at a time, so the kit
 * verifies every tap alone as today. It taps only the cards that both this
 * picks (the same rules as the kit's lane/shipall.mjs) and the steward's
 * fresh check (`steward/ship-all-check`, a status item the kit drops each
 * tick) lists as clear, card by card; every other waiting card says why.
 */

import {
  type BenchCard,
  type BenchSnapshot,
  benchCreatedSeconds,
} from "@/features/bench/lib/benchCards";

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
const OVER = / cards wait, more than one check can list: /;
/** Under two ticks: one slow tick (232 s, 15 Nov) must not turn it off. */
const STALE_MS = 25 * 60_000;
const FUTURE_MS = 2 * 60_000;

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

export type ShipAllGate = {
  /** What this button taps, in the steward's order; while the check can't be used, what it would. */
  cards: ShipAllCard[];
  enabled: boolean;
  /** Why it is off, and when that clears; empty when enabled. */
  reasons: string[];
  /** This button's other waiting cards, each with why it is not tapped. */
  left: ShipAllLeftOut[];
  /** When the steward checks again, once its check is in use. */
  recheck: string;
};

/**
 * What one button taps now: the cards both picked here and listed clear for
 * it by a readable steward check at most 25 minutes old (their ids hold
 * their commits), in the check's order. A card answered since the check just
 * drops out; one the check holds, or has not seen yet, is left out with why.
 */
export function shipAllGate(
  snapshot: Pick<BenchSnapshot, "items">,
  button: ShipAllButton,
  now: number,
): ShipAllGate {
  const { candidates } = shipAllPick(snapshot);
  const mine = candidates
    .filter((c) => button === "details" || !c.readFirst)
    .sort(
      (a, b) =>
        (benchCreatedSeconds(a.card) ?? 0) - (benchCreatedSeconds(b.card) ?? 0),
    );
  const off = (reason: string) => ({
    cards: mine,
    enabled: false,
    reasons: [reason],
    left: [],
    recheck: "",
  });
  if (mine.length === 0) {
    return off(
      candidates.length > 0
        ? "Every waiting card is read first (lane:hold, money, sign-in or database): open Give me details to read and ship them there."
        : "Nothing for this button waits on your tap.",
    );
  }
  const check = readShipAllCheck(snapshot);
  const still = "Each card's own Ship it still works.";
  if (check === "missing") {
    return off(
      `The steward's check is not on the Bench yet: it checks every 15 minutes. ${still}`,
    );
  }
  if (check === "unreadable") {
    return off(
      `The steward's check could not be read. ${still} It checks every 15 minutes; if this stays, see #steward.`,
    );
  }
  const at = formatShipAllTime(check.checked);
  if (now - check.checked > STALE_MS || check.checked - now > FUTURE_MS) {
    return off(
      `The steward's last check (${at}) is too old to trust. It checks every 15 minutes; if this stays, the tick may be failing (see #steward). ${still}`,
    );
  }
  const recheck =
    check.next > now
      ? `It checks again by ${formatShipAllTime(check.next)}.`
      : "It checks again at its next tick.";
  const cards = check.cards
    .filter((l) => button === "details" || l.digest)
    .flatMap((l) => mine.filter((c) => c.card.id === l.id));
  const left = mine
    .filter((c) => !cards.includes(c))
    .map(({ card }): ShipAllLeftOut => {
      const held = check.held.get(card.id);
      if (held) return { card, reason: held };
      if (check.cards.some((l) => l.id === card.id)) {
        return {
          card,
          reason:
            "It is read first now (lane:hold was added after its card): ship it from Give me details.",
          readFirst: true,
        };
      }
      // The kit's one way to name no card while some wait: too many to list.
      const over = check.whys[button].find((w) => OVER.test(w));
      return {
        card,
        reason: over ?? `New since the steward's check at ${at}.`,
      };
    });
  if (cards.length === 0) {
    const shown = left.slice(0, 3).map((l) => l.reason);
    const more = left.length - shown.length;
    return {
      cards,
      enabled: false,
      reasons: [...shown, ...(more > 0 ? [`And ${more} more.`] : []), recheck],
      left,
      recheck,
    };
  }
  return { cards, enabled: true, reasons: [], left, recheck };
}

export type ShipAllTap = {
  itemId: string;
  cardEventId: string;
  /** 0-based: the card's recommended Ship it. */
  option: number;
  title: string;
};

export function shipAllTaps(cards: readonly ShipAllCard[]): ShipAllTap[] {
  return cards.map(({ card }) => ({
    itemId: card.id,
    cardEventId: card.cardEventId ?? "",
    option: (card.recommend ?? 1) - 1,
    title: card.title,
  }));
}

/**
 * Taps each card in order with `answer` (bench_answer), then waits for its
 * Recorded line before the next; the first failure stops the run, so no
 * card goes before one under it. `stillOk` reads the Bench again before each
 * tap, so an answer or reply he gave meanwhile, or a new check that holds
 * the card, is never overwritten with Ship it.
 */
export async function runShipAll(
  taps: readonly ShipAllTap[],
  {
    answer,
    stillOk,
    isRecorded,
    onStep,
    sleep,
    pollMs = 1_000,
    timeoutMs = 15_000,
  }: {
    answer: (tap: ShipAllTap) => Promise<unknown>;
    stillOk: (tap: ShipAllTap) => Promise<boolean>;
    isRecorded: (tap: ShipAllTap) => Promise<boolean>;
    onStep: (index: number) => void;
    sleep: (ms: number) => Promise<void>;
    pollMs?: number;
    timeoutMs?: number;
  },
): Promise<{ recorded: number; error: string | null }> {
  for (const [i, tap] of taps.entries()) {
    onStep(i);
    if (!(await stillOk(tap).catch(() => false))) {
      return {
        recorded: i,
        error: `${tap.title}: not tapped. You answered or wrote on it meanwhile, or the steward's check no longer clears it.`,
      };
    }
    try {
      await answer(tap);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      return { recorded: i, error: `${tap.title}: couldn't save: ${message}` };
    }
    for (let polls = 0; !(await isRecorded(tap).catch(() => false)); polls++) {
      if (polls * pollMs >= timeoutMs) {
        return {
          recorded: i,
          error: `${tap.title}: its Ship it was sent, but the Bench has not recorded it after ${timeoutMs / 1_000} s. It may still record: check its card, and click its Ship it to take it back.`,
        };
      }
      await sleep(pollMs);
    }
  }
  return { recorded: taps.length, error: null };
}

/** Whether a fresh snapshot still has this card waiting and clear on this button. */
export function shipAllStillOk(
  snapshot: Pick<BenchSnapshot, "items"> | null,
  tap: ShipAllTap,
  button: ShipAllButton,
  now: number,
): boolean {
  const gate = snapshot && shipAllGate(snapshot, button, now);
  return (
    !!gate?.enabled &&
    gate.cards.some(
      (c) => c.card.id === tap.itemId && c.card.cardEventId === tap.cardEventId,
    )
  );
}

/** Whether a fresh snapshot shows the owner's Ship it recorded on the card. */
export function shipAllRecorded(
  snapshot: Pick<BenchSnapshot, "items"> | null,
  tap: ShipAllTap,
): boolean {
  const card = snapshot?.items.find((i) => i.id === tap.itemId);
  return (
    card?.cardEventId === tap.cardEventId &&
    card.recorded === tap.option &&
    (card.answer?.byOwner ?? true)
  );
}

/** The titles of the steward's trains still on their way to live. */
export function shipAllTrains(
  snapshot: Pick<BenchSnapshot, "items">,
): string[] {
  return snapshot.items
    .filter(
      (i) =>
        i.writer === "steward" &&
        /^steward\/(?:reset|talos|buzz)-train$/.test(i.id) &&
        i.validity === "current" &&
        !i.title.endsWith(": live"),
    )
    .map((i) => i.title);
}
