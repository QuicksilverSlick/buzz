/**
 * PR numbers as whole tokens, shared by global search and the Bench search:
 * "#792" names PR 792 and never 7928, #1792, 792d3edd or 0.5.792.
 */

export type RepoShort = "buzz" | "reset" | "talos";

export const REPO_SHORT: Record<string, RepoShort> = {
  buzz: "buzz",
  reset: "reset",
  "metabolic-reset": "reset",
  talos: "talos",
  "dreamforge-talos": "talos",
};

/** `explicit`: "#", "pr"/"pull" or a repo was typed, not a bare number. */
export type PrQuery = {
  number: string;
  repo: RepoShort | null;
  explicit: boolean;
};

/**
 * "#792", "PR 792", "pr792", "pr-792", "PR #792", "pull/792", "792",
 * "buzz#792", "talos/12" and "reset pr 792"; null for anything else. A repo
 * and a number with only a space between ("reset 14") stay words.
 */
export function parsePrQuery(query: string): PrQuery | null {
  const q = query.trim().toLowerCase().replace(/\s+/g, " ");
  const m =
    /^(?:(buzz|reset|talos|metabolic-reset|dreamforge-talos)(?:\s*[#:/]\s*|\s+(?=pr|pull)))?(?:(pr|pull)\s*[-#/]?\s*|(#))?(\d{1,7})$/.exec(
      q,
    );
  return m
    ? {
        number: m[4],
        repo: m[1] ? REPO_SHORT[m[1]] : null,
        explicit: Boolean(m[1] || m[2] || m[3]),
      }
    : null;
}

// Not followed by more of a word or number: never 7928, 792d3edd or 792.5.
const END = "(?!\\.?[0-9a-z])";

/**
 * 2: the text names PR `n` (#n, PR n, pr-n, pull/n, "pull request n");
 * 1: `n` is a whole-number token; 0: neither. `n` is 1-7 ASCII digits.
 */
export function prMentionScore(text: string, n: string): 0 | 1 | 2 {
  const t = text.toLowerCase();
  if (
    new RegExp(
      `(?:#|\\bpr\\s*[-#]?\\s*|\\bpull(?:/|\\s+request\\s+))${n}${END}`,
    ).test(t)
  ) {
    return 2;
  }
  return new RegExp(`(?<![0-9a-z.,])${n}${END}`).test(t) ? 1 : 0;
}
