import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeKnownAgentPubkeys,
  mergeOwnedAgentPubkeys,
  ownsAnyAgent,
} from "./knownAgentPubkeys.ts";
import { WELCOME_TEAM_ID } from "@/features/onboarding/welcomeGuide";

const MANAGED =
  "1111111111111111111111111111111111111111111111111111111111111111";
const RELAY =
  "2222222222222222222222222222222222222222222222222222222222222222";

test("mergesTrustedSources", () => {
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED }],
    [{ pubkey: RELAY }],
  );

  assert.deepEqual([...merged].sort(), [MANAGED, RELAY].sort());
});

test("undefinedSources_yieldEmptySet", () => {
  const merged = mergeKnownAgentPubkeys(undefined, undefined);

  assert.equal(merged.size, 0);
});

test("normalisesCaseAndWhitespace_dedupingAcrossSources", () => {
  // The same agent appearing in multiple sources with different casing /
  // stray whitespace must collapse to one normalised entry, so membership
  // checks against normalizePubkey output always hit.
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED.toUpperCase() }],
    [{ pubkey: ` ${MANAGED}` }],
  );

  assert.deepEqual([...merged], [MANAGED]);
});

test("owned agents include managed and profile-declared agents", () => {
  const merged = mergeOwnedAgentPubkeys(
    [{ pubkey: MANAGED }],
    {
      [RELAY]: { ownerPubkey: " owner " },
      other: { ownerPubkey: "somebody-else" },
    },
    "OWNER",
  );

  assert.deepEqual([...merged].sort(), [MANAGED, RELAY].sort());
});

test("owned agents exclude agents controlled by somebody else", () => {
  const merged = mergeOwnedAgentPubkeys(
    undefined,
    { [RELAY]: { ownerPubkey: "somebody-else" } },
    "owner",
  );

  assert.equal(merged.size, 0);
});

test("ownsAnyAgent counts managed agents and relay agents owned by the viewer", () => {
  const cases = [
    [[{ pubkey: MANAGED }], [], "owner", true],
    [[{ pubkey: MANAGED }], undefined, null, true],
    [[], [{ pubkey: RELAY, ownerPubkey: " OWNER " }], "owner", true],
    [undefined, [{ pubkey: RELAY, ownerPubkey: "owner" }], "OWNER", true],
    [[], [{ pubkey: RELAY, ownerPubkey: "somebody-else" }], "owner", false],
    [[], [{ pubkey: RELAY, ownerPubkey: null }], "owner", false],
    [[], [{ pubkey: RELAY, ownerPubkey: "owner" }], null, false],
    [undefined, undefined, "owner", false],
  ];

  for (const [managed, relay, viewer, expected] of cases) {
    assert.equal(
      ownsAnyAgent(managed, relay, viewer),
      expected,
      JSON.stringify({ managed, relay, viewer }),
    );
  }
});

test("ownsAnyAgent leaves out the starter team onboarding gives everyone", () => {
  const starter = { pubkey: MANAGED, teamId: WELCOME_TEAM_ID };
  const cases = [
    // The starter as both a managed agent and a relay agent the viewer owns.
    [
      [starter],
      [{ pubkey: MANAGED.toUpperCase(), ownerPubkey: "owner" }],
      false,
    ],
    [[starter], [], false],
    [[starter, { pubkey: RELAY, teamId: null }], [], true],
    [[starter], [{ pubkey: RELAY, ownerPubkey: "owner" }], true],
  ];

  for (const [managed, relay, expected] of cases) {
    assert.equal(
      ownsAnyAgent(managed, relay, "owner", WELCOME_TEAM_ID),
      expected,
      JSON.stringify({ managed, relay }),
    );
  }
  // Without a starter team, every managed agent counts.
  assert.equal(ownsAnyAgent([starter], [], "owner"), true);
});
