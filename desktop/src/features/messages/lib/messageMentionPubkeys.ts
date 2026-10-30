import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

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

/**
 * Recipients for a message sent outside the DM screen (Inbox reply, Wave).
 *
 * A one-on-one DM tags the other participant, so a plain reply reaches an
 * agent. A group DM tags the other people but only the agents typed in the
 * composer, so one reply does not wake every agent in it. Returns null when
 * the DM's participants are not loaded: the caller must refuse rather than
 * send untagged.
 */
export function directMessageRecipientPubkeys(
  channel: Channel,
  senderPubkey: string,
  explicitMentions: readonly string[],
  isAgentPubkey: (pubkey: string) => boolean,
): string[] | null {
  if (channel.channelType !== "dm") {
    return messageMentionPubkeys(channel, senderPubkey, explicitMentions);
  }
  const others = messageMentionPubkeys(channel, senderPubkey);
  if (others.length === 0) {
    return null;
  }
  const addressed =
    others.length === 1 ? others : others.filter((pk) => !isAgentPubkey(pk));
  const sender = normalizePubkey(senderPubkey);
  return [
    ...new Set([...explicitMentions.map(normalizePubkey), ...addressed]),
  ].filter((pubkey) => pubkey.length > 0 && pubkey !== sender);
}
