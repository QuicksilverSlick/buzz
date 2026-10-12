import type { CommunityOnboardingTransaction } from "@/features/onboarding/communityOnboarding";
import {
  guestLandingHash,
  isInviteJoin,
  joinsAsGuest,
  markCommunityGuest,
  pickGuestLandingChannel,
} from "@/features/onboarding/guestMode";
import { getMyRelayMembershipLookup } from "@/shared/api/relayMembers";
import { getChannels } from "@/shared/api/tauri";
import type { Channel, RelayMemberRole } from "@/shared/api/types";

/**
 * How long guest entry waits on the relay. Past it, the role counts as
 * unknown (the invite decides) and the guest lands in Inbox.
 */
const GUEST_ENTRY_LOOKUP_TIMEOUT_MS = 8_000;

export type GuestEntry = { landingHash: string };

/** Resolves to `fallback` when `promise` rejects or takes longer than `timeoutMs`. */
async function settleWithin<T>(
  promise: Promise<T>,
  fallback: T,
  timeoutMs: number,
  label: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((resolve) => {
        timer = setTimeout(() => {
          console.warn(`${label} timed out.`);
          resolve(fallback);
        }, timeoutMs);
      }),
    ]);
  } catch (error) {
    console.warn(`${label} failed.`, error);
    return fallback;
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** The viewer's role in the active community, or null when the relay keeps no membership list. */
async function loadCommunityRole(): Promise<RelayMemberRole | null> {
  const lookup = await getMyRelayMembershipLookup();
  return lookup.membership?.role ?? null;
}

/** The channels the viewer belongs to, with their latest activity. */
async function loadJoinedChannels(): Promise<Channel[]> {
  const payload = await getChannels(null);
  return (payload.channels ?? []).map((channel) => ({
    ...channel,
    lastMessageAt: payload.lastMessages[channel.id] ?? channel.lastMessageAt,
  }));
}

/** The relay questions guest entry asks; replaced in tests. */
export type GuestEntryLookups = {
  loadChannels: () => Promise<Channel[]>;
  loadRole: () => Promise<RelayMemberRole | null>;
  timeoutMs: number;
};

const RELAY_LOOKUPS: GuestEntryLookups = {
  loadChannels: loadJoinedChannels,
  loadRole: loadCommunityRole,
  timeoutMs: GUEST_ENTRY_LOOKUP_TIMEOUT_MS,
};

/**
 * Decide whether this onboarding enters the active community as a guest and,
 * if so, where the guest lands. Returns null for everyone else, who keep the
 * Welcome experience. Only invite joins ask the relay anything, and every
 * relay question is bounded: an unknown role leaves the invite to decide,
 * and an unknown channel list lands the guest in Inbox.
 */
export async function prepareGuestEntry(
  transaction: Pick<CommunityOnboardingTransaction, "inviteCode"> | null,
  lookups: GuestEntryLookups = RELAY_LOOKUPS,
): Promise<GuestEntry | null> {
  if (!isInviteJoin(transaction)) return null;
  const [role, channels] = await Promise.all([
    settleWithin(
      lookups.loadRole(),
      null,
      lookups.timeoutMs,
      "Reading your community role",
    ),
    settleWithin(
      lookups.loadChannels(),
      [],
      lookups.timeoutMs,
      "Loading your channels",
    ),
  ]);
  if (!joinsAsGuest(transaction, role)) return null;
  return { landingHash: guestLandingHash(pickGuestLandingChannel(channels)) };
}

/**
 * Commit a prepared guest entry: remember guest mode for this identity and
 * community, then point the router at the landing place before the app
 * mounts. Callers mark onboarding complete afterwards, so a refused storage
 * write throws here and leaves onboarding open to retry.
 */
export function commitGuestEntry(
  entry: GuestEntry,
  pubkey: string,
  relayUrl: string,
): void {
  markCommunityGuest(pubkey, relayUrl);
  window.location.hash = entry.landingHash;
}
