import { getChannelDetail } from "@/features/channels/lib/channelDescription";
import {
  canManageCommunityMembers,
  type RelayMembershipLookup,
} from "@/shared/api/relayMembers";
import type { Channel } from "@/shared/api/types";

/** One row of the "Channels in this community" list in #welcome-everyone. */
export type WelcomeChannelGuideEntry = {
  description: string | null;
  id: string;
  name: string;
};

/**
 * Whether the viewer keeps the "Create a channel" and "Create an agent"
 * cards in #welcome-everyone. The relay lets any member create channels, so
 * the community's owners and admins stand in for the people who build it;
 * on an open relay with no membership list, everyone does. While the
 * membership is still unknown, the viewer is treated as a guest.
 */
export function canBuildCommunity(
  lookup: RelayMembershipLookup | undefined,
): boolean {
  if (!lookup) return false;
  return !lookup.membershipRequired || canManageCommunityMembers(lookup);
}

/**
 * The channels listed in #welcome-everyone: every channel the viewer can
 * open (open channels, and private ones they belong to), with its
 * description. Direct messages, archived channels and the welcome channel
 * itself are left out. Sorted by name.
 */
export function selectWelcomeChannelGuide(
  channels: readonly Channel[],
  welcomeChannelId: string,
): WelcomeChannelGuideEntry[] {
  return channels
    .filter(
      (channel) =>
        channel.id !== welcomeChannelId &&
        channel.channelType !== "dm" &&
        channel.archivedAt === null &&
        (channel.visibility === "open" || channel.isMember),
    )
    .map((channel) => ({
      description: getChannelDetail(channel),
      id: channel.id,
      name: channel.name,
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
}
