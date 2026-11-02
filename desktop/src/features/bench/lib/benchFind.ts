/**
 * Bench cards for global search, read from the steward kit's ids and links
 * (hold-<repo>-<pr>-<head12>-<base12>, <repo>-launch-needed-<sha>,
 * <repo>-train); a card it can't read still matches on its text.
 */

import {
  type BenchCard,
  type BenchSnapshot,
  benchCardRepo,
  searchText,
} from "@/features/bench/lib/benchCards";
import { parsePrQuery, prMentionScore } from "@/features/search/lib/prNumber";

const HOLD = /^steward\/hold-(buzz|reset|talos)-(\d{1,7})-[0-9a-f]{12}/;
const WORD = /[^\p{L}\p{N}]+/u;

/** The PRs a card is about: its id, PR links, launch item, receipt lines. */
export function benchCardPrs(card: BenchCard): string[] {
  const all = (re: RegExp) => Array.from(card.body.matchAll(re), (m) => m[1]);
  return [
    HOLD.exec(card.id)?.[2],
    /(?:^|\/)pr-(\d{1,7})$/.exec(card.id)?.[1],
    ...card.links.map(
      ({ url }) => /\/pull\/(\d{1,7})(?:[/?#]|$)/.exec(url)?.[1],
    ),
    ...all(/\b(?:buzz|reset|talos):pr-(\d{1,7})\b/g),
    ...(card.id.endsWith("-train") ? all(/^- (\d{1,7})[ :]/gm) : []),
  ].filter((pr): pr is string => Boolean(pr));
}

/** Every pending hold card is red by the kit's definition. */
export function benchCardTier(card: BenchCard): "red" | null {
  return HOLD.test(card.id) && card.kind === "pending" ? "red" : null;
}

/** As the Bench's Needs you shows it to the owner: no one else asked first. */
export const benchNeedsYou = (card: BenchCard) =>
  card.kind === "pending" && !card.answer && card.answerers.length === 0;

/** Needs you, then other pending, then decisions and status. */
const rank = (card: BenchCard) =>
  benchNeedsYou(card) ? 0 : card.kind === "pending" ? 1 : 2;

/**
 * What names a card, split into words: its id, title, project, pull links
 * and "receipt" for one. Word queries look only here, so "it" or "send"
 * never match every card's options or body.
 */
function nameWords(card: BenchCard): string[] {
  const receipt = card.request || card.id.endsWith("-train") ? "receipt" : "";
  return [
    card.id,
    card.title,
    card.area,
    receipt,
    ...card.links.map((l) => l.url),
  ]
    .join(" ")
    .toLowerCase()
    .split(WORD);
}

/**
 * Current cards for a global search, best first. A PR query ranks cards about
 * that PR (and repo) first, then cards naming it, then (a bare number only)
 * cards with it as a whole number; for other queries each word must start a
 * word of the card's names, title matches first. Ties: needs you, newest, id.
 */
export function findBenchCards(
  snapshot: BenchSnapshot | null,
  query: string,
  limit = 5,
): BenchCard[] {
  if (!snapshot || !query.trim()) return [];
  const pr = parsePrQuery(query);
  const q = query.trim().toLowerCase();
  const words = q.split(WORD).filter(Boolean);
  if (!pr && !words.length) return [];
  const score = (card: BenchCard): number => {
    if (!pr) {
      const names = nameWords(card);
      if (!words.every((w) => names.some((n) => n.startsWith(w)))) return 0;
      return card.title.toLowerCase().includes(q) ? 2 : 1;
    }
    const repo = benchCardRepo(card);
    const sameRepo = !pr.repo || repo === pr.repo;
    if (sameRepo && benchCardPrs(card).includes(pr.number)) return 3;
    if (!sameRepo && repo) return 0;
    const mention = prMentionScore(searchText(card), pr.number);
    return mention === 2 || !pr.explicit ? mention : 0;
  };
  return snapshot.items
    .filter((card) => card.validity === "current")
    .map((card) => ({ card, score: score(card) }))
    .filter(({ score }) => score > 0)
    .sort(
      (a, b) =>
        b.score - a.score ||
        rank(a.card) - rank(b.card) ||
        (Date.parse(b.card.updatedAt) || 0) -
          (Date.parse(a.card.updatedAt) || 0) ||
        a.card.id.localeCompare(b.card.id),
    )
    .slice(0, limit)
    .map(({ card }) => card);
}
