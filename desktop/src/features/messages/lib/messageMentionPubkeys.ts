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

type DmRecipientInput = {
  channel: Channel;
  senderPubkey: string;
  explicitMentions: readonly string[];
  members: DirectoryQuery<{ pubkey: string; role: string; isAgent: boolean }>;
  managedAgents: DirectoryQuery<{ pubkey: string }>;
  relayAgents: DirectoryQuery<{ pubkey: string }>;
  knownAgentPubkeys: ReadonlySet<string>;
};

/**
 * Recipients for a send that tells a group DM's agents apart.
 *
 * Streams and one-on-one DMs get messageMentionPubkeys unchanged, so a plain
 * message in a 1:1 DM still reaches its agent. A group DM tags the other
 * people and the typed mentions but no untyped agent, so one message does not
 * wake every agent in it. Agents are told apart by the DM's own member list
 * plus the agent directories; a group-DM send waits for all of them, and a
 * participant missing from the member list is never taken for a person.
 * Throws rather than send untagged.
 */
export function dmRecipientPubkeys({
  channel,
  senderPubkey,
  explicitMentions,
  members,
  managedAgents,
  relayAgents,
  knownAgentPubkeys,
}: DmRecipientInput): string[] {
  const recipients = messageMentionPubkeys(
    channel,
    senderPubkey,
    explicitMentions,
  );
  if (
    channel.channelType !== "dm" ||
    messageMentionPubkeys(channel, senderPubkey).length <= 1
  ) {
    return recipients;
  }

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
  const mentioned = new Set(explicitMentions.map(normalizePubkey));
  return recipients.filter(
    (pubkey) => mentioned.has(pubkey) || people.has(pubkey),
  );
}

/**
 * Recipients for an Inbox reply, which skips useSendMessage: the
 * dmRecipientPubkeys rule, refusing a DM that is not loaded or a mention of
 * someone outside it.
 */
export function inboxReplyRecipientPubkeys({
  channel,
  ...input
}: Omit<DmRecipientInput, "channel"> & {
  channel: Channel | undefined;
}): string[] {
  if (!channel) throw new Error("Open the linked channel to reply.");
  if (channel.channelType === "dm") {
    const others = messageMentionPubkeys(channel, input.senderPubkey);
    if (others.length === 0) {
      throw new Error("Open the linked channel to reply.");
    }
    const sender = normalizePubkey(input.senderPubkey);
    // The relay never shows this DM to a non-participant, and only the DM
    // screen can open a new conversation that includes them.
    if (
      input.explicitMentions
        .map(normalizePubkey)
        .some(
          (pubkey) =>
            pubkey.length > 0 && pubkey !== sender && !others.includes(pubkey),
        )
    ) {
      throw new Error(DM_NON_MEMBER_MENTION_ERROR);
    }
  }
  return dmRecipientPubkeys({ channel, ...input });
}
