import type { CommunityOnboardingTransaction } from "@/features/onboarding/communityOnboarding";
import {
  guestLandingHash,
  isGuestStarterChannel,
  isInviteJoin,
  joinsAsGuest,
  markCommunityGuest,
  markGuestStarterChannelsJoined,
  pickGuestLandingChannel,
  withJoinedStarterChannels,
} from "@/features/onboarding/guestMode";
import { getMyRelayMembershipLookup } from "@/shared/api/relayMembers";
import {
  getChannels,
  getOpenChannelDirectory,
  joinChannel,
} from "@/shared/api/tauri";
import type { Channel, RelayMemberRole } from "@/shared/api/types";

/**
 * How long guest entry waits on each relay question. Past it, the role counts
 * as unknown (the invite decides), the starter channels are joined once the
 * app opens instead, and an unknown channel list lands the guest in Inbox.
 */
const GUEST_ENTRY_LOOKUP_TIMEOUT_MS = 8_000;

export type GuestEntry = {
  landingHash: string;
  /** The owner's #general and #welcome-everyone were joined during entry. */
  startersJoined: boolean;
};

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

/**
 * Join the community's existing open #general and #welcome-everyone, the way
 * Block's phone app does after an invite. Accepting an invite adds a person
 * to the community but to none of its channels. Creates nothing: a community
 * whose owner removed these channels stays without them. Resolves to the
 * starter channels, all joined, and rejects when any join fails.
 */
export async function joinGuestStarterChannels(): Promise<Channel[]> {
  const starters = (await getOpenChannelDirectory()).filter(
    isGuestStarterChannel,
  );
  await Promise.all(
    starters
      .filter((channel) => !channel.isMember)
      .map((channel) => joinChannel(channel.id)),
  );
  return starters.map((channel) => ({ ...channel, isMember: true }));
}

/** The relay questions guest entry asks; replaced in tests. */
export type GuestEntryLookups = {
  joinStarterChannels: () => Promise<Channel[]>;
  loadChannels: () => Promise<Channel[]>;
  loadRole: () => Promise<RelayMemberRole | null>;
  timeoutMs: number;
};

const RELAY_LOOKUPS: GuestEntryLookups = {
  joinStarterChannels: joinGuestStarterChannels,
  loadChannels: loadJoinedChannels,
  loadRole: loadCommunityRole,
  timeoutMs: GUEST_ENTRY_LOOKUP_TIMEOUT_MS,
};

/**
 * Join the starter channels, then read the channels the viewer belongs to.
 * Each step has its own time budget, so a slow directory scan still leaves
 * time to read the channels and land somewhere.
 */
async function loadGuestChannels(lookups: GuestEntryLookups) {
  const starters = await settleWithin<Channel[] | null>(
    lookups.joinStarterChannels(),
    null,
    lookups.timeoutMs,
    "Joining the community's starter channels",
  );
  const channels = await settleWithin(
    lookups.loadChannels(),
    [],
    lookups.timeoutMs,
    "Loading your channels",
  );
  return {
    channels: withJoinedStarterChannels(channels, starters ?? []),
    startersJoined: starters !== null,
  };
}

/**
 * Decide whether this onboarding enters the active community as a guest and,
 * if so, where the guest lands. Returns null for everyone else, who keep the
 * Welcome experience. Only invite joins ask the relay anything, and every
 * relay question is bounded: an unknown role leaves the invite to decide,
 * and an unknown channel list lands the guest in Inbox.
 *
 * The starter channels are joined while the role is read. Joining creates
 * nothing, and an owner or admin who turns out not to be a guest joins them
 * during their own setup anyway.
 */
export async function prepareGuestEntry(
  transaction: Pick<CommunityOnboardingTransaction, "inviteCode"> | null,
  lookups: GuestEntryLookups = RELAY_LOOKUPS,
): Promise<GuestEntry | null> {
  if (!isInviteJoin(transaction)) return null;
  const [role, landing] = await Promise.all([
    settleWithin(
      lookups.loadRole(),
      null,
      lookups.timeoutMs,
      "Reading your community role",
    ),
    loadGuestChannels(lookups),
  ]);
  if (!joinsAsGuest(transaction, role)) return null;
  return {
    landingHash: guestLandingHash(pickGuestLandingChannel(landing.channels)),
    startersJoined: landing.startersJoined,
  };
}

/**
 * Commit a prepared guest entry: remember guest mode for this identity and
 * community, then point the router at the landing place before the app
 * mounts. Callers mark onboarding complete afterwards, so a refused storage
 * write throws here and leaves onboarding open to retry. When the starter
 * channels could not be joined yet, the app tries again each time it opens
 * this community until it can.
 */
export function commitGuestEntry(
  entry: GuestEntry,
  pubkey: string,
  relayUrl: string,
): void {
  markCommunityGuest(pubkey, relayUrl);
  if (entry.startersJoined) markGuestStarterChannelsJoined(pubkey, relayUrl);
  window.location.hash = entry.landingHash;
}
