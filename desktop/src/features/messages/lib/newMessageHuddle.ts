import { buildHuddleChannelName } from "@/features/huddle/lib/huddleChannelName";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type StartNewMessageHuddleOptions = {
  currentPubkey?: string;
  recipients: readonly { pubkey: string; isAgent: boolean }[];
  openDirectMessage: () => Promise<Channel | null>;
  /** Resolves false when the user has left, so the huddle must not start. */
  showDirectMessage: (directMessage: Channel) => Promise<boolean>;
  startHuddle: (
    parentChannelId: string,
    memberPubkeys: string[],
    channelName?: string,
  ) => Promise<void>;
};

/**
 * The New Message screen has no channel until something opens the DM, and a
 * huddle needs a parent channel. Open (or reuse) the DM on demand, show it,
 * then start the huddle there, so no throwaway first message is needed.
 * Agent recipients join as huddle members, as they do from a DM's header.
 */
export async function startNewMessageHuddle({
  currentPubkey,
  recipients,
  openDirectMessage,
  showDirectMessage,
  startHuddle,
}: StartNewMessageHuddleOptions): Promise<boolean> {
  const directMessage = await openDirectMessage();
  if (!directMessage || !(await showDirectMessage(directMessage))) {
    return false;
  }

  await startHuddle(
    directMessage.id,
    recipients
      .filter((recipient) => recipient.isAgent)
      .map((recipient) => normalizePubkey(recipient.pubkey)),
    buildHuddleChannelName({ channel: directMessage, currentPubkey }),
  );
  return true;
}
