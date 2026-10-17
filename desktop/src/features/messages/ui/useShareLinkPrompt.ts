import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  CHANNEL_MEMBERS_STALE_TIME_MS,
  channelMembersQueryKey,
} from "@/features/channels/rosterFreshness";
import { fetchStructuralAuxForMessages } from "@/features/messages/lib/auxBackfill";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import {
  findLinkedMessageShare,
  type LinkedMessageShare,
} from "@/features/messages/lib/shareLinkedMessage";
import type { useMentions } from "@/features/messages/lib/useMentions";
import { relaySelfQueryKey } from "@/features/moderation/hooks";
import { getRelaySelf } from "@/features/moderation/lib/relaySelf";
import { getChannelMembers } from "@/shared/api/tauriChannels";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { normalizePubkey, truncateNpub } from "@/shared/lib/pubkey";
import {
  loadMessageLink,
  readyMetadata,
} from "@/shared/ui/markdown/useMessageLinkMetadata";

type SharePrompt = LinkedMessageShare & {
  channelId: string;
  draftKey: string | null;
  revision: number;
};

/**
 * The linked message as its channel shows it now: fetched fresh (never the
 * link card's session cache), its latest authorized edit applied and its
 * deletion honored by the timeline's own rules, so a quote never carries
 * text its author removed.
 */
async function fetchMessageForQuote(
  link: ParsedMessageLink,
  relaySelfPubkey: string | null,
) {
  const loaded = await loadMessageLink(link);
  if (loaded.kind !== "ready") return loaded;
  const aux = await fetchStructuralAuxForMessages(link.channelId, [
    loaded.event.id,
  ]);
  const [shown] = formatTimelineMessages(
    [loaded.event, ...aux],
    null,
    undefined,
    null,
    loaded.profile
      ? { [normalizePubkey(loaded.event.pubkey)]: loaded.profile }
      : undefined,
    undefined,
    undefined,
    undefined,
    relaySelfPubkey,
  );
  return shown
    ? readyMetadata(loaded.event, loaded.profile, shown.body)
    : ({ kind: "deleted" } as const);
}

/**
 * Holds a send whose draft links a private message some recipients can't
 * open, until the owner picks "Include quote" or "Send link only". A prompt
 * belongs to one channel, draft and revision; anything else drops it.
 */
export function useShareLinkPrompt({
  channelId,
  draftKey,
  getComposerRevision,
  mentions,
  ownerPubkey,
  resubmit,
}: {
  channelId: string | null;
  draftKey: string | null;
  getComposerRevision: () => number;
  mentions: Pick<
    ReturnType<typeof useMentions>,
    | "extractMentionPersonas"
    | "extractMentionPubkeys"
    | "getMentionDisplayName"
    | "hasResolvedMembers"
    | "memberPubkeys"
  >;
  ownerPubkey: string | null;
  resubmit: (quote: string | null) => void;
}) {
  const { channels } = useChannelNavigation();
  const queryClient = useQueryClient();
  const [prompt, setPrompt] = React.useState<SharePrompt | null>(null);
  // Bumped by every check and dismiss: only the latest check may prompt.
  const request = React.useRef(0);
  const scope = React.useRef({ channelId, draftKey });
  scope.current = { channelId, draftKey };
  const {
    extractMentionPersonas,
    extractMentionPubkeys,
    getMentionDisplayName,
    hasResolvedMembers,
    memberPubkeys,
  } = mentions;

  /** True when the send must wait: for the owner's choice, or a stale check. */
  const check = React.useCallback(
    async (
      content: string,
      addressedAgentPubkeys: readonly string[] = [],
    ): Promise<boolean> => {
      if (!channelId || !hasResolvedMembers) return false;
      const token = ++request.current;
      setPrompt(null); // A new send asks afresh; an older answer never applies.
      const revision = getComposerRevision();
      const share = await findLinkedMessageShare(content, {
        channelId,
        channels,
        recipientPubkeys: new Set(
          [
            ...memberPubkeys,
            ...extractMentionPubkeys(content),
            ...addressedAgentPubkeys,
          ].map(normalizePubkey),
        ),
        newRecipientNames: extractMentionPersonas(content).map(
          (mention) => mention.displayName,
        ),
        ownerPubkey,
        getMemberPubkeys: async (id) =>
          (
            await queryClient.fetchQuery({
              queryKey: channelMembersQueryKey(id),
              queryFn: () => getChannelMembers(id),
              staleTime: CHANNEL_MEMBERS_STALE_TIME_MS,
            })
          ).map((member) => member.pubkey),
        getMessage: async (link) =>
          fetchMessageForQuote(
            link,
            await queryClient
              .fetchQuery({
                queryKey: relaySelfQueryKey,
                queryFn: getRelaySelf,
                staleTime: Number.POSITIVE_INFINITY,
              })
              .catch(() => null),
          ),
        getDisplayName: (pubkey) =>
          getMentionDisplayName(pubkey) ?? truncateNpub(pubkey),
      }).catch(() => null); // A failed lookup sends the link alone: nothing leaks.
      // Switched conversation, typed or dismissed meanwhile: offer nothing,
      // send nothing; the draft stays where it was written.
      if (
        token !== request.current ||
        scope.current.channelId !== channelId ||
        scope.current.draftKey !== draftKey ||
        getComposerRevision() !== revision
      ) {
        return true;
      }
      if (!share) return false;
      setPrompt({ ...share, channelId, draftKey, revision });
      return true;
    },
    [
      channelId,
      channels,
      draftKey,
      extractMentionPersonas,
      extractMentionPubkeys,
      getComposerRevision,
      getMentionDisplayName,
      hasResolvedMembers,
      memberPubkeys,
      ownerPubkey,
      queryClient,
    ],
  );

  const dismiss = React.useCallback(() => {
    request.current += 1;
    setPrompt(null);
  }, []);

  const choose = (includeQuote: boolean) => {
    if (!prompt) return;
    dismiss();
    if (
      prompt.channelId !== scope.current.channelId ||
      prompt.draftKey !== scope.current.draftKey ||
      prompt.revision !== getComposerRevision()
    ) {
      toast.error("The draft changed, so nothing was sent.");
      return;
    }
    resubmit(includeQuote ? prompt.quote : null);
  };

  return {
    check,
    dismiss,
    prompt: prompt
      ? {
          names: prompt.names,
          openChannel: prompt.openChannel,
          quote: prompt.quote,
        }
      : null,
    onIncludeQuote: () => choose(true),
    onSendLinkOnly: () => choose(false),
  };
}
