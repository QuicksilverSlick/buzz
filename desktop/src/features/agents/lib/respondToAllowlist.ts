/**
 * Pure helpers for the inbound author gate UI.
 *
 * The Rust side is the canonical validator (see
 * `desktop/src-tauri/src/managed_agents/types.rs::validate_respond_to_allowlist`).
 * These helpers exist to give the UI immediate, inline feedback before the
 * round-trip, and to normalize input so the Rust validator sees clean data.
 *
 * Entry pieces may be 64-char hex pubkeys or bech32 `npub1…` strings; both are
 * normalized to the canonical lowercase hex via the shared
 * `parsePubkeyInput`, so npub and hex spellings of the same key dedupe to one
 * entry (users copy npubs from profile/verify surfaces elsewhere in the app).
 */

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { parsePubkeyInput as parseCanonicalPubkey } from "@/shared/lib/nostrUtils";
import {
  canonicalNpub,
  normalizePubkey,
  truncateNpub,
} from "@/shared/lib/pubkey";

export type ParsedAllowlist = {
  /** Successfully parsed entries — lowercase hex, deduplicated, in order. */
  valid: string[];
  /** Entries that failed validation, in their raw form. */
  invalid: string[];
};

/**
 * Parse a free-form pubkey-paste input (one per line, comma-separated, or
 * mixed whitespace) into a normalized allowlist. Matches the splitting
 * pattern used by `ChannelMemberInviteCard` so users have one mental model.
 *
 * - Splits on `/[\s,]+/`.
 * - Accepts 64-char hex (any case) or `npub1…` bech32 per piece, normalizing
 *   to the canonical lowercase hex pubkey.
 * - Deduplicates the canonical form while preserving insertion order.
 */
export function parsePubkeyInput(raw: string): ParsedAllowlist {
  const seen = new Set<string>();
  const valid: string[] = [];
  const invalid: string[] = [];
  for (const piece of raw.split(/[\s,]+/)) {
    const trimmed = piece.trim();
    if (trimmed.length === 0) continue;
    const canonical = parseCanonicalPubkey(trimmed);
    if (canonical === null) {
      invalid.push(trimmed);
      continue;
    }
    if (!seen.has(canonical)) {
      seen.add(canonical);
      valid.push(canonical);
    }
  }
  return { valid, invalid };
}

/**
 * Merge an existing allowlist with newly-added pubkeys, normalizing and
 * deduplicating without reordering existing entries. Both hex and npub
 * spellings normalize to the canonical hex, so the same key cannot enter
 * twice regardless of the form it was added in.
 */
export function mergeAllowlist(existing: string[], add: string[]): string[] {
  const normalize = (pubkey: string): string =>
    parseCanonicalPubkey(pubkey) ?? pubkey.toLowerCase();
  const out = existing.map(normalize);
  const seen = new Set(out);
  for (const candidate of add) {
    const canonical = parseCanonicalPubkey(candidate);
    if (canonical === null || seen.has(canonical)) continue;
    seen.add(canonical);
    out.push(canonical);
  }
  return out;
}

export type AllowlistChip = {
  pubkey: string;
  /** Profile name, then NIP-05, then the managed agent's name, then short npub. */
  label: string;
  /** Full npub for the hover tooltip; null when the key cannot be encoded. */
  title: string | null;
  /** `Craig (npub1y9f…u08a)`, or just the short npub when no name is known. */
  accessibleName: string;
  avatarUrl: string | null;
  isAgent: boolean;
  /** Another selected entry shows the same name, so the key must be visible. */
  ambiguous: boolean;
};

/**
 * Label selected allowlist entries the way message headers do, so the owner
 * can tell who they are removing. A display name is self-asserted: the key
 * stays in the tooltip and accessible name, and is shown inline whenever two
 * entries share a name so an impostor cannot hide behind a duplicate.
 */
export function allowlistChips(
  allowlist: readonly string[],
  profiles: UserProfileLookup | undefined,
  agentNames: ReadonlyMap<string, string>,
): AllowlistChip[] {
  const chips = allowlist.map((pubkey) => {
    const key = normalizePubkey(pubkey);
    const agentName = agentNames.get(key);
    const shortNpub = truncateNpub(pubkey);
    const label = resolveUserLabel({
      pubkey,
      profiles,
      fallbackName: agentName,
    });
    return {
      pubkey,
      label,
      title: canonicalNpub(pubkey),
      accessibleName:
        label === shortNpub ? shortNpub : `${label} (${shortNpub})`,
      avatarUrl: profiles?.[key]?.avatarUrl ?? null,
      isAgent: profiles?.[key]?.isAgent === true || agentName !== undefined,
      ambiguous: false,
    };
  });
  const nameCounts = new Map<string, number>();
  for (const chip of chips) {
    const name = chip.label.trim().toLowerCase();
    nameCounts.set(name, (nameCounts.get(name) ?? 0) + 1);
  }
  for (const chip of chips) {
    chip.ambiguous =
      chip.accessibleName !== chip.label &&
      (nameCounts.get(chip.label.trim().toLowerCase()) ?? 0) > 1;
  }
  return chips;
}

/**
 * Whether a selected entry matches the picker's search text: anywhere in the
 * name, but only as a key prefix, since short text lands inside a random key.
 */
export function allowlistChipMatches(
  chip: AllowlistChip,
  query: string,
): boolean {
  const needle = query.trim().toLowerCase();
  return (
    needle.length === 0 ||
    chip.label.toLowerCase().includes(needle) ||
    (chip.title?.startsWith(needle) ?? false) ||
    chip.pubkey.toLowerCase().startsWith(needle)
  );
}
