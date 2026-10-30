import { mergeChannelKnownAgentPubkeys } from "@/features/agents/knownAgentPubkeys";
import { isAgentDirectoryReady } from "@/features/agents/lib/agentAutocompleteEligibility";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  DM_NON_MEMBER_MENTION_ERROR,
  DM_THREAD_MEMBERS_LOADING_ERROR,
} from "./dmThreadAgentMentionError";

/**
 * Return the semantic recipients for an outgoing message.
 *
 * Stream messages notify only explicit mentions. A DM addresses every other
 * participant, so it must carry recipient `p` tags even when the composer text
 * contains no `@mention`. Agent harnesses and human notification subscriptions
 * both rely on those tags.
 */
export function messageMentionPubkeys(
  channel: Channel,
  senderPubkey: string,
  explicitMentions: readonly string[] = [],
): string[] {
  const candidates =
    channel.channelType === "dm"
      ? [
          ...explicitMentions,
          ...channel.memberPubkeys,
          ...channel.participantPubkeys,
        ]
      : explicitMentions;
  const sender = normalizePubkey(senderPubkey);

  return [...new Set(candidates.map(normalizePubkey))].filter(
    (pubkey) => pubkey.length > 0 && pubkey !== sender,
  );
}

type DirectoryQuery<T> = { data: readonly T[] | undefined; error: unknown };

/**
 * Recipients for an Inbox reply, which skips useSendMessage.
 *
 * A one-on-one DM tags the other participant, so a plain reply reaches an
 * agent. A group DM tags the other people but only the agents typed in the
 * composer, so one reply does not wake every agent in it. Agents are told
 * apart by the DM's own member list plus the agent directories; a group-DM
 * reply waits for all of them, and a participant missing from the member list
 * is never taken for a person. Throws rather than send untagged.
 */
export function inboxReplyRecipientPubkeys({
  channel,
  senderPubkey,
  explicitMentions,
  members,
  managedAgents,
  relayAgents,
  knownAgentPubkeys,
}: {
  channel: Channel | undefined;
  senderPubkey: string;
  explicitMentions: readonly string[];
  members: DirectoryQuery<{ pubkey: string; role: string; isAgent: boolean }>;
  managedAgents: DirectoryQuery<{ pubkey: string }>;
  relayAgents: DirectoryQuery<{ pubkey: string }>;
  knownAgentPubkeys: ReadonlySet<string>;
}): string[] {
  if (!channel) throw new Error("Open the linked channel to reply.");
  if (channel.channelType !== "dm") {
    return messageMentionPubkeys(channel, senderPubkey, explicitMentions);
  }
  const others = messageMentionPubkeys(channel, senderPubkey);
  if (others.length === 0) throw new Error("Open the linked channel to reply.");
  const sender = normalizePubkey(senderPubkey);
  const mentioned = explicitMentions
    .map(normalizePubkey)
    .filter((pubkey) => pubkey.length > 0 && pubkey !== sender);
  // The relay never shows this DM to a non-participant, and only the DM
  // screen can open a new conversation that includes them.
  if (mentioned.some((pubkey) => !others.includes(pubkey))) {
    throw new Error(DM_NON_MEMBER_MENTION_ERROR);
  }
  if (others.length === 1) return others;

  if (
    !isAgentDirectoryReady(members) ||
    !isAgentDirectoryReady(managedAgents) ||
    !isAgentDirectoryReady(relayAgents) ||
    !members.data
  ) {
    throw new Error(DM_THREAD_MEMBERS_LOADING_ERROR);
  }
  const agents = mergeChannelKnownAgentPubkeys(
    members.data,
    managedAgents.data,
    relayAgents.data,
  );
  const people = new Set(
    members.data
      .map((member) => normalizePubkey(member.pubkey))
      .filter(
        (pubkey) => !agents.has(pubkey) && !knownAgentPubkeys.has(pubkey),
      ),
  );
  return others.filter(
    (pubkey) => mentioned.includes(pubkey) || people.has(pubkey),
  );
}
