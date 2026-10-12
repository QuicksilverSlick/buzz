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
/** Set once a guest belongs to the owner's #general and #welcome-everyone. */
const GUEST_STARTERS_JOINED_STORAGE_KEY = "buzz-guest-starters-joined.v1";
/** Set once a guest has used or dismissed the hint about asking agents. */
const GUEST_AGENT_HINT_DONE_STORAGE_KEY = "buzz-guest-agent-hint-done.v1";

type GuestStorage = Pick<Storage, "getItem" | "removeItem" | "setItem">;
type InviteTransaction = Pick<CommunityOnboardingTransaction, "inviteCode">;

/**
 * Whether this community onboarding joins through an invite. Every invite
 * entry point (an opened invite link, an invite pasted into Join a
 * community, Add a community, or the membership-denied screen) puts the
 * invite code on the transaction at the "claiming" stage, and onboarding only
 * moves past "claiming" once the relay accepts that code.
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

/** A per-identity, per-community storage key. */
function communityScopedKey(prefix: string, pubkey: string, relayUrl: string) {
  return `${prefix}:${encodeURIComponent(
    normalizeCommunityScope(relayUrl),
  )}:${pubkey.trim().toLowerCase()}`;
}

function defaultStorage(): GuestStorage | null {
  return typeof localStorage === "undefined" ? null : localStorage;
}

function readCommunityFlag(
  prefix: string,
  pubkey: string | null | undefined,
  relayUrl: string | null | undefined,
  storage: GuestStorage | null = defaultStorage(),
): boolean {
  if (!pubkey || !relayUrl) return false;
  try {
    return (
      storage?.getItem(communityScopedKey(prefix, pubkey, relayUrl)) === "true"
    );
  } catch {
    return false;
  }
}

/**
 * Best-effort write of a flag whose loss only repeats harmless work: a refused
 * write leaves the flag unset, so the work runs again next time.
 */
function writeCommunityFlag(
  prefix: string,
  pubkey: string,
  relayUrl: string,
  storage: GuestStorage | null = defaultStorage(),
): void {
  try {
    storage?.setItem(communityScopedKey(prefix, pubkey, relayUrl), "true");
  } catch (error) {
    console.warn("Could not save a guest setting for this community.", error);
  }
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
  storage.setItem(
    communityScopedKey(COMMUNITY_GUEST_STORAGE_KEY, pubkey, relayUrl),
    "true",
  );
}

export function isCommunityGuest(
  pubkey: string | null | undefined,
  relayUrl: string | null | undefined,
  storage?: GuestStorage,
): boolean {
  return readCommunityFlag(
    COMMUNITY_GUEST_STORAGE_KEY,
    pubkey,
    relayUrl,
    storage,
  );
}

/**
 * Forget guest mode for this identity and community: the relay lists them as
 * its owner or admin, so the invite that classed them as a guest was only
 * decided because the role lookup didn't answer. A refused removal keeps the
 * record, and the next launch's role check clears it again.
 */
export function clearCommunityGuest(
  pubkey: string,
  relayUrl: string,
  storage: GuestStorage | null = defaultStorage(),
): void {
  try {
    storage?.removeItem(
      communityScopedKey(COMMUNITY_GUEST_STORAGE_KEY, pubkey, relayUrl),
    );
  } catch (error) {
    console.warn("Could not clear guest mode for this community.", error);
  }
}

/** Whether this guest already belongs to the owner's starter channels. */
export function hasJoinedGuestStarterChannels(
  pubkey: string | null | undefined,
  relayUrl: string | null | undefined,
  storage?: GuestStorage,
): boolean {
  return readCommunityFlag(
    GUEST_STARTERS_JOINED_STORAGE_KEY,
    pubkey,
    relayUrl,
    storage,
  );
}

export function markGuestStarterChannelsJoined(
  pubkey: string,
  relayUrl: string,
  storage?: GuestStorage,
): void {
  writeCommunityFlag(
    GUEST_STARTERS_JOINED_STORAGE_KEY,
    pubkey,
    relayUrl,
    storage,
  );
}

/** Whether this guest has used or dismissed the hint about asking agents. */
export function isGuestAgentHintDone(
  pubkey: string | null | undefined,
  relayUrl: string | null | undefined,
  storage?: GuestStorage,
): boolean {
  return readCommunityFlag(
    GUEST_AGENT_HINT_DONE_STORAGE_KEY,
    pubkey,
    relayUrl,
    storage,
  );
}

export function markGuestAgentHintDone(
  pubkey: string,
  relayUrl: string,
  storage?: GuestStorage,
): void {
  writeCommunityFlag(
    GUEST_AGENT_HINT_DONE_STORAGE_KEY,
    pubkey,
    relayUrl,
    storage,
  );
}

const STARTER_CHANNEL_NAMES = new Set([
  STARTER_GENERAL_CHANNEL_NAME,
  STARTER_WELCOME_CHANNEL_NAME,
]);

function isStarterChannelName(channel: Channel) {
  return STARTER_CHANNEL_NAMES.has(channel.name.trim().toLowerCase());
}

/**
 * The community's own #general or #welcome-everyone, the open channels every
 * member is meant to be in. Accepting an invite joins the community, not its
 * channels, so a guest joins these, and only these, once they are in.
 */
export function isGuestStarterChannel(channel: Channel): boolean {
  return (
    channel.channelType === "stream" &&
    channel.visibility === "open" &&
    channel.archivedAt === null &&
    isStarterChannelName(channel)
  );
}

/**
 * The guest's channel list with the starter channels they just joined added.
 * The relay's member list can trail a join that was just sent, so a joined
 * starter may be missing from `channels` for a moment.
 */
export function withJoinedStarterChannels(
  channels: readonly Channel[],
  joinedStarters: readonly Channel[],
): Channel[] {
  const listed = new Set(channels.map((channel) => channel.id));
  return [
    ...channels,
    ...joinedStarters
      .filter((channel) => !listed.has(channel.id))
      .map((channel) => ({ ...channel, isMember: true })),
  ];
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
    null
  );
}

/** The router hash for a guest's landing place: the channel, or Inbox. */
export function guestLandingHash(channel: Channel | null): string {
  return channel ? `/channels/${encodeURIComponent(channel.id)}` : "/";
}
