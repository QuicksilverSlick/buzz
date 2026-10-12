import { isEphemeralChannel } from "@/features/channels/lib/ephemeralChannel";
import type { CommunityOnboardingTransaction } from "@/features/onboarding/communityOnboarding";
import {
  isStarterWelcomeChannel,
  isWelcomeChannel,
  STARTER_GENERAL_CHANNEL_NAME,
  STARTER_WELCOME_CHANNEL_NAME,
} from "@/features/onboarding/welcome";
import type { Channel, RelayMemberRole } from "@/shared/api/types";

/**
 * Guest mode is how the app treats someone who joins an existing community
 * through an invite. The community's owner has already set up its channels
 * and agents, so a guest gets no private Welcome channel, no Welcome Team
 * (which would need the guest's own AI provider) and no kickoff, and lands in
 * a channel the owner put them in. Owners and people who create a community
 * keep the Welcome experience.
 */

const COMMUNITY_GUEST_STORAGE_KEY = "buzz-community-guest.v1";

type GuestStorage = Pick<Storage, "getItem" | "setItem">;
type InviteTransaction = Pick<CommunityOnboardingTransaction, "inviteCode">;

/**
 * Whether this community onboarding joins through an invite. Every invite
 * entry point (an opened invite link, an invite pasted into Join a
 * community, Add a community, or the membership-denied screen) starts the
 * transaction with the invite code, and onboarding only moves past
 * "claiming" once the relay accepts that code.
 */
export function isInviteJoin(
  transaction: InviteTransaction | null | undefined,
): boolean {
  return Boolean(transaction?.inviteCode?.trim());
}

/**
 * Whether an onboarding enters the community as a guest: it joins through an
 * invite, and the relay does not list the person as the community's owner or
 * admin (someone reconnecting with an invite link keeps their Welcome
 * experience). `role` is null when the relay could not say, and then the
 * invite decides.
 */
export function joinsAsGuest(
  transaction: InviteTransaction | null | undefined,
  role: RelayMemberRole | null,
): boolean {
  return isInviteJoin(transaction) && role !== "owner" && role !== "admin";
}

function normalizeCommunityScope(relayUrl: string) {
  return relayUrl.trim().replace(/\/+$/, "").toLowerCase();
}

function communityGuestStorageKey(pubkey: string, relayUrl: string) {
  return `${COMMUNITY_GUEST_STORAGE_KEY}:${encodeURIComponent(
    normalizeCommunityScope(relayUrl),
  )}:${pubkey.trim().toLowerCase()}`;
}

/**
 * Remember that this identity is a guest of this community, so later launches
 * never create the private Welcome channel or the Welcome Team for it. Throws
 * when storage refuses the write, so onboarding stays open to retry instead
 * of finishing without the record.
 */
export function markCommunityGuest(
  pubkey: string,
  relayUrl: string,
  storage: GuestStorage = localStorage,
): void {
  storage.setItem(communityGuestStorageKey(pubkey, relayUrl), "true");
}

export function isCommunityGuest(
  pubkey: string | null | undefined,
  relayUrl: string | null | undefined,
  storage?: GuestStorage,
): boolean {
  if (!pubkey || !relayUrl) return false;
  try {
    const store =
      storage ?? (typeof localStorage === "undefined" ? null : localStorage);
    return (
      store?.getItem(communityGuestStorageKey(pubkey, relayUrl)) === "true"
    );
  } catch {
    return false;
  }
}

const STARTER_CHANNEL_NAMES = new Set([
  STARTER_GENERAL_CHANNEL_NAME,
  STARTER_WELCOME_CHANNEL_NAME,
]);

function isStarterChannelName(channel: Channel) {
  return STARTER_CHANNEL_NAMES.has(channel.name.trim().toLowerCase());
}

function lastActivity(channel: Channel) {
  const time = channel.lastMessageAt ? Date.parse(channel.lastMessageAt) : NaN;
  return Number.isNaN(time) ? Number.NEGATIVE_INFINITY : time;
}

/**
 * Where a guest lands when onboarding finishes: the channel they belong to
 * that the owner set up for them (any joined channel other than #general and
 * #welcome-everyone, the most recently active first), else #welcome-everyone,
 * else nothing, which means Inbox. Direct messages, archived and temporary
 * channels, and a private Welcome channel are never a landing place.
 */
export function pickGuestLandingChannel(
  channels: readonly Channel[],
): Channel | null {
  const usable = channels.filter(
    (channel) =>
      channel.channelType !== "dm" &&
      channel.archivedAt === null &&
      !isEphemeralChannel(channel) &&
      !isWelcomeChannel(channel),
  );
  let landing: Channel | null = null;
  for (const channel of usable) {
    if (!channel.isMember || isStarterChannelName(channel)) continue;
    if (!landing || lastActivity(channel) > lastActivity(landing)) {
      landing = channel;
    }
  }
  return (
    landing ??
    usable.find(
      (channel) => isStarterWelcomeChannel(channel) && channel.isMember,
    ) ??
    usable.find(isStarterWelcomeChannel) ??
    null
  );
}

/** The router hash for a guest's landing place: the channel, or Inbox. */
export function guestLandingHash(channel: Channel | null): string {
  return channel ? `/channels/${encodeURIComponent(channel.id)}` : "/";
}
