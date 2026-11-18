import assert from "node:assert/strict";
import test from "node:test";

import {
  allowlistChipMatches,
  allowlistChips,
  allowlistReachabilityText,
  allowlistWithoutSharedChannel,
  mergeAllowlist,
  parsePubkeyInput,
} from "./respondToAllowlist.ts";

const HEX_A = "a".repeat(64);
const HEX_B = "b".repeat(64);
const HEX_A_UPPER = "A".repeat(64);
// Handoff vector, round-trip verified with nostr-tools nip19.
const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const HEX_NPUB =
  "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

test("parsePubkeyInput splits on commas, whitespace, and newlines", () => {
  const input = `${HEX_A}, ${HEX_B}\n${HEX_A_UPPER}`;
  const result = parsePubkeyInput(input);
  assert.deepEqual(result.valid, [HEX_A, HEX_B]);
  assert.deepEqual(result.invalid, []);
});

test("parsePubkeyInput lowercases and dedupes", () => {
  const result = parsePubkeyInput(`${HEX_A_UPPER} ${HEX_A}`);
  assert.deepEqual(result.valid, [HEX_A]);
});

test("parsePubkeyInput surfaces invalid entries separately", () => {
  const result = parsePubkeyInput(`notgood ${HEX_A} ${"z".repeat(64)}`);
  assert.deepEqual(result.valid, [HEX_A]);
  assert.deepEqual(result.invalid, ["notgood", "z".repeat(64)]);
});

test("parsePubkeyInput accepts npub entries, normalizes to hex, and dedupes across spellings", () => {
  // One invalid npub-shaped token proves classification at this seam; the
  // full codec negative matrix lives at the shared parser (nostrUtils).
  const corrupt = `${HEX_NPUB.slice(0, -2)}qq`;
  const result = parsePubkeyInput(
    `${HEX_NPUB} ${HEX} ${HEX.toUpperCase()} ${corrupt}`,
  );
  assert.deepEqual(result.valid, [HEX]);
  assert.deepEqual(result.invalid, [corrupt]);
});

test("parsePubkeyInput rejects wrong-length entries", () => {
  const shortHex = "a".repeat(63);
  const longHex = "a".repeat(65);
  const result = parsePubkeyInput(`${shortHex} ${longHex}`);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, [shortHex, longHex]);
});

test("parsePubkeyInput handles empty and whitespace-only input", () => {
  assert.deepEqual(parsePubkeyInput("").valid, []);
  assert.deepEqual(parsePubkeyInput("   \n\t  ").valid, []);
});

test("mergeAllowlist preserves existing order and appends new", () => {
  const merged = mergeAllowlist([HEX_A], [HEX_B]);
  assert.deepEqual(merged, [HEX_A, HEX_B]);
});

test("mergeAllowlist dedupes case-insensitively", () => {
  const merged = mergeAllowlist([HEX_A], [HEX_A_UPPER]);
  assert.deepEqual(merged, [HEX_A]);
});

test("mergeAllowlist skips invalid additions silently", () => {
  // Invalid additions are caller-validated; merge ignores them defensively.
  const merged = mergeAllowlist([HEX_A], ["not-hex", HEX_B]);
  assert.deepEqual(merged, [HEX_A, HEX_B]);
});

test("mergeAllowlist normalizes npub additions to canonical hex and dedupes", () => {
  assert.deepEqual(mergeAllowlist([HEX_A], [HEX_NPUB]), [HEX_A, HEX]);
  assert.deepEqual(mergeAllowlist([HEX], [HEX_NPUB]), [HEX]);
});

const HEX_SHORT_NPUB = "npub1a2d…yp60";
const NO_AGENTS = new Map();
const profile = (displayName, extra = {}) => ({
  displayName,
  avatarUrl: null,
  nip05Handle: null,
  ...extra,
});
const chipFor = (profiles, agentNames = NO_AGENTS) =>
  allowlistChips([HEX], profiles, agentNames)[0];

test("allowlistChips shows a known profile's display name", () => {
  const chip = chipFor({ [HEX]: profile("Craig", { avatarUrl: "a.png" }) });
  assert.equal(chip.label, "Craig");
  assert.equal(chip.accessibleName, `Craig (${HEX_SHORT_NPUB})`);
  assert.equal(chip.avatarUrl, "a.png");
  assert.equal(chip.isAgent, false);
});

test("allowlistChips falls back to the short npub for an unknown key", () => {
  const chip = chipFor({});
  assert.equal(chip.label, HEX_SHORT_NPUB);
  assert.equal(chip.accessibleName, HEX_SHORT_NPUB);
  assert.equal(chip.ambiguous, false);
});

test("allowlistChips tooltip carries the full npub", () => {
  assert.equal(chipFor(undefined).title, HEX_NPUB);
  assert.equal(
    allowlistChips(["not-a-key"], undefined, NO_AGENTS)[0].title,
    null,
  );
});

test("allowlistChips names a managed agent with no relay profile", () => {
  const agents = new Map([[HEX, "Reset Worker"]]);
  const chip = chipFor({}, agents);
  assert.equal(chip.label, "Reset Worker");
  assert.equal(chip.isAgent, true);
  assert.equal(
    chipFor({ [HEX]: profile("Relay Name") }, agents).label,
    "Relay Name",
  );
});

test("allowlistChips shows the key when two entries share a name", () => {
  const [real, impostor, other] = allowlistChips(
    [HEX, HEX_A, HEX_B],
    {
      [HEX]: profile("Craig"),
      [HEX_A]: profile(" craig "),
      [HEX_B]: profile("Alice"),
    },
    NO_AGENTS,
  );
  assert.equal(real.ambiguous, true);
  assert.equal(impostor.ambiguous, true);
  assert.equal(other.ambiguous, false);
});

test("allowlistChipMatches finds a selected entry by name or key prefix", () => {
  const chip = chipFor({ [HEX]: profile("Craig") });
  assert.equal(allowlistChipMatches(chip, ""), true);
  assert.equal(allowlistChipMatches(chip, " cRAi "), true);
  assert.equal(allowlistChipMatches(chip, HEX_NPUB.slice(0, 12)), true);
  assert.equal(allowlistChipMatches(chip, HEX.slice(0, 10)), true);
  assert.equal(allowlistChipMatches(chip, "alice"), false);
  // Short text lands inside a random key; only a key prefix may match.
  assert.equal(allowlistChipMatches(chip, HEX.slice(20, 24)), false);
  assert.equal(allowlistChipMatches(chip, HEX_NPUB.slice(20, 24)), false);
});

const AGENT = "c".repeat(64);
const HEX_D = "d".repeat(64);

function channel(channelType, memberPubkeys, archivedAt = null) {
  return { channelType, archivedAt, memberPubkeys };
}

test("allowlistWithoutSharedChannel counts a shared stream and a shared forum", () => {
  const channels = [
    channel("stream", [AGENT, HEX_A]),
    channel("forum", [HEX_B, AGENT]),
  ];
  assert.deepEqual(
    allowlistWithoutSharedChannel([HEX_A, HEX_B], AGENT, channels),
    [],
  );
});

test("allowlistWithoutSharedChannel ignores shared DMs, group or 1:1", () => {
  // A partner can only mention their own agents in a DM on Desktop.
  const channels = [
    channel("dm", [AGENT, HEX_A, HEX_B]),
    channel("dm", [AGENT, HEX_D]),
  ];
  assert.deepEqual(
    allowlistWithoutSharedChannel([HEX_A, HEX_B, HEX_D], AGENT, channels),
    [HEX_A, HEX_B, HEX_D],
  );
});

test("allowlistWithoutSharedChannel ignores a shared archived channel", () => {
  const channels = [channel("stream", [AGENT, HEX_A], "2026-11-01T00:00:00Z")];
  assert.deepEqual(allowlistWithoutSharedChannel([HEX_A], AGENT, channels), [
    HEX_A,
  ]);
});

test("allowlistWithoutSharedChannel flags people only in channels without the agent", () => {
  const channels = [
    channel("stream", [HEX_A, HEX_B]),
    channel("stream", [AGENT, HEX_B]),
  ];
  assert.deepEqual(
    allowlistWithoutSharedChannel([HEX_A, HEX_B], AGENT, channels),
    [HEX_A],
  );
});

test("allowlistWithoutSharedChannel flags everyone when the agent is in no channel", () => {
  const channels = [channel("stream", [HEX_A, HEX_B])];
  assert.deepEqual(
    allowlistWithoutSharedChannel([HEX_A, HEX_B], AGENT, channels),
    [HEX_A, HEX_B],
  );
  assert.deepEqual(allowlistWithoutSharedChannel([HEX_A], AGENT, []), [HEX_A]);
});

test("allowlistWithoutSharedChannel matches keys across case", () => {
  const channels = [channel("stream", [AGENT.toUpperCase(), HEX_A])];
  assert.deepEqual(
    allowlistWithoutSharedChannel([HEX_A_UPPER], AGENT, channels),
    [],
  );
});

test("allowlistWithoutSharedChannel keeps input spelling and order", () => {
  const channels = [channel("stream", [AGENT, HEX_B])];
  assert.deepEqual(
    allowlistWithoutSharedChannel(
      [HEX_D, HEX_A_UPPER, HEX_B],
      AGENT.toUpperCase(),
      channels,
    ),
    [HEX_D, HEX_A_UPPER],
  );
});

test("allowlistReachabilityText names nobody, one, two, or a few and a count", () => {
  const tail =
    "in a channel with this agent yet, so there's nowhere for them to reach it. Add them to one of its channels.";
  assert.equal(allowlistReachabilityText([]), null);
  assert.equal(allowlistReachabilityText(["Chris"]), `Chris isn't ${tail}`);
  assert.equal(
    allowlistReachabilityText(["Chris", "Craig"]),
    `Chris and Craig aren't ${tail}`,
  );
  assert.equal(
    allowlistReachabilityText(["A", "B", "C"]),
    `A, B and C aren't ${tail}`,
  );
  assert.equal(
    allowlistReachabilityText(["A", "B", "C", "D", "E"]),
    `A, B and 3 others aren't ${tail}`,
  );
});
