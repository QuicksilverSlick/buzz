/** Steward cards and the kit's Ship all check as a Bench snapshot shows them. */

export const RESET = "QuicksilverSlick/Metabolic-reset";
export const ANSWER = {
  option: 0,
  answeredAt: 1,
  by: "",
  byName: null,
  byOwner: true,
  source: "tap",
};
export const REPLY = {
  eventId: "r",
  by: "",
  byName: null,
  role: null,
  at: 1,
  hhmm: "",
};
const OPTIONS = ["Ship it", "Send it back", "Keep holding"];
const BASE = "b".repeat(40);

export function stewardItem(id, overrides = {}) {
  return {
    id,
    writer: "steward",
    kind: "pending",
    title: id,
    body: "",
    area: "28-day-reset",
    severity: "attention",
    options: OPTIONS,
    state: null,
    decidedBy: null,
    deadline: null,
    links: [],
    order: 0,
    validity: "current",
    cardEventId: "e".repeat(64),
    answer: null,
    recorded: null,
    updatedAt: "2026-11-15T18:00:00Z",
    channelId: null,
    project: null,
    scoped: false,
    needs: "you",
    answerers: [],
    ownerName: null,
    recommend: 1,
    recommender: null,
    reply: null,
    request: null,
    plain: { risk: "medium", undo: "easy" },
    ...overrides,
  };
}

/** A reset hold card as the kit raises it; `sha` seeds its head commit. */
export function hold(pr, { sha = "a", readFirst = false, ...overrides } = {}) {
  const head = sha.repeat(40);
  const wait = readFirst ? "waits for you, read first" : "waits for your tap";
  return stewardItem(
    `steward/hold-reset-${pr}-${head.slice(0, 12)}-${BASE.slice(0, 12)}`,
    {
      title: `reset PR ${pr} ${wait}: Change ${pr}`,
      action: `Ship pull request ${pr} of ${RESET} at commit ${head} on top of main at commit ${BASE}`,
      cardEventId: String(pr).padStart(64, "e"),
      ...overrides,
    },
  );
}

export function stack(prs, { digest = "c".repeat(64), ...overrides } = {}) {
  const span = `${prs[0]}-${prs.at(-1)}`;
  return stewardItem(`steward/stack-reset-${span}-${digest.slice(0, 12)}`, {
    title: `reset stack ${span} waits for your tap: ${prs.length} pull requests`,
    action: `Ship as one stack the pull requests ${prs.join(", ")} of ${RESET}, list ${digest}`,
    ...overrides,
  });
}

/**
 * The kit's check item; `listed` as [card, on the digest button too?], the
 * clear ones; `held` as [card, why].
 */
export function shipAllCheck(
  listed,
  {
    at = Date.now(),
    digest = "clear",
    details = "clear",
    held = [],
    whys = [],
  } = {},
) {
  const iso = (ms) => new Date(ms).toISOString();
  return stewardItem("steward/ship-all-check", {
    kind: "status",
    state: "active",
    options: [],
    recommend: null,
    cardEventId: null,
    title: "Ship all check",
    body: [
      "Ship all check v1: the steward's check for the desktop's Ship all buttons. It ships nothing.",
      `checked: ${iso(at)}`,
      `next: ${iso(at + 15 * 60_000)}`,
      `digest: ${digest}`,
      `details: ${details}`,
      ...listed.map(
        ([c, both]) => `card: ${c.id} ${both ? "digest " : ""}details`,
      ),
      ...held.map(([c, why]) => `held: ${c.id} ${why}`),
      ...whys,
    ].join("\n"),
  });
}
