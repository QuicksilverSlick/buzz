import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  CHANNEL_MEMBERS_STALE_TIME_MS,
  channelMembersQueryKey,
} from "@/features/channels/rosterFreshness";
import { findLinkedMessageShare } from "@/features/messages/lib/shareLinkedMessage";
import type { useMentions } from "@/features/messages/lib/useMentions";
import { getChannelMembers } from "@/shared/api/tauriChannels";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { truncateNpub } from "@/shared/lib/pubkey";
import { fetchMessageLinkMetadata } from "@/shared/ui/markdown/useMessageLinkMetadata";

type SharePrompt = { names: string[]; quote: string; revision: number };

/**
 * Holds a send whose draft links a private message some recipients can't
 * open, until the owner picks "Include quote" or "Send link only".
 */
export function useShareLinkPrompt({
  channelId,
  getComposerRevision,
  mentions,
  ownerPubkey,
  resubmit,
}: {
  channelId: string | null;
  getComposerRevision: () => number;
  mentions: Pick<
    ReturnType<typeof useMentions>,
    "getMentionDisplayName" | "hasResolvedMembers" | "memberPubkeys"
  >;
  ownerPubkey: string | null;
  resubmit: (quote: string | null) => void;
}) {
  const { channels } = useChannelNavigation();
  const queryClient = useQueryClient();
  const [prompt, setPrompt] = React.useState<SharePrompt | null>(null);
  const { getMentionDisplayName, hasResolvedMembers, memberPubkeys } = mentions;

  /** True when the send must wait for the owner's choice. */
  const check = React.useCallback(
    async (content: string): Promise<boolean> => {
      if (!channelId || !hasResolvedMembers) return false;
      const revision = getComposerRevision();
      const share = await findLinkedMessageShare(content, {
        channelId,
        channels,
        memberPubkeys,
        ownerPubkey,
        getMemberPubkeys: async (id) =>
          (
            await queryClient.fetchQuery({
              queryKey: channelMembersQueryKey(id),
              queryFn: () => getChannelMembers(id),
              staleTime: CHANNEL_MEMBERS_STALE_TIME_MS,
            })
          ).map((member) => member.pubkey),
        getMessage: fetchMessageLinkMetadata,
        getDisplayName: (pubkey) =>
          getMentionDisplayName(pubkey) ?? truncateNpub(pubkey),
      }).catch(() => null); // A failed lookup sends the link alone: nothing leaks.
      if (!share) return false;
      setPrompt({ ...share, revision });
      return true;
    },
    [
      channelId,
      channels,
      getComposerRevision,
      getMentionDisplayName,
      hasResolvedMembers,
      memberPubkeys,
      ownerPubkey,
      queryClient,
    ],
  );

  const choose = (includeQuote: boolean) => {
    if (!prompt) return;
    setPrompt(null);
    if (prompt.revision !== getComposerRevision()) return;
    resubmit(includeQuote ? prompt.quote : null);
  };

  return {
    check,
    dismiss: React.useCallback(() => setPrompt(null), []),
    names: prompt?.names ?? null,
    onIncludeQuote: () => choose(true),
    onSendLinkOnly: () => choose(false),
  };
}
