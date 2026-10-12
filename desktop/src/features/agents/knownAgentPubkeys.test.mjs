import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeKnownAgentPubkeys,
  mergeOwnedAgentPubkeys,
  ownsAnyAgent,
} from "./knownAgentPubkeys.ts";

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
    [[], [{ ownerPubkey: " OWNER " }], "owner", true],
    [undefined, [{ ownerPubkey: "owner" }], "OWNER", true],
    [[], [{ ownerPubkey: "somebody-else" }], "owner", false],
    [[], [{ ownerPubkey: null }], "owner", false],
    [[], [{ ownerPubkey: "owner" }], null, false],
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
