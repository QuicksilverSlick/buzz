import * as React from "react";

import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import { summarizeMessageLinkContent } from "@/features/messages/lib/messageLinkMetadata";
import { getEventById } from "@/shared/api/tauri";
import { getUserProfile } from "@/shared/api/tauriProfiles";
import type { Profile, RelayEvent } from "@/shared/api/types";
import { truncateNpub } from "@/shared/lib/pubkey";
import { isDefinitiveEventNotFound } from "@/shared/lib/eventLookupError";

const MESSAGE_METADATA_RETRY_DELAY_MS = 750;
const PREVIEWABLE_MESSAGE_KINDS = new Set([9, 40002, 45001, 45003]);

function waitForMessageMetadataRetry(): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, MESSAGE_METADATA_RETRY_DELAY_MS);
  });
}

async function getMessageLinkEvent(messageId: string) {
  try {
    return await getEventById(messageId);
  } catch (error) {
    if (isDefinitiveEventNotFound(error)) throw error;
    await waitForMessageMetadataRetry();
    return getEventById(messageId);
  }
}

type MessageLinkMetadata = {
  author: string;
  content: string;
  createdAt: number;
  pubkey: string;
  snippet: string;
};
type MessageLinkMetadataState =
  | { kind: "idle" }
  | { kind: "loading" }
  | ({ kind: "ready" } & MessageLinkMetadata)
  | { kind: "deleted" }
  | { kind: "unavailable" };

type CachedMessageLinkMetadata =
  | ({ kind: "ready" } & MessageLinkMetadata)
  | { kind: "deleted" }
  | { kind: "unavailable" };

const metadataCache = new Map<string, Promise<CachedMessageLinkMetadata>>();

export function resetMessageLinkMetadataCache() {
  metadataCache.clear();
}

export type LoadedMessageLink =
  | { kind: "ready"; event: RelayEvent; profile: Profile | null }
  | { kind: "deleted" }
  | { kind: "unavailable" };

/** The linked event and its author's profile, uncached (the quote reads it fresh). */
export async function loadMessageLink(
  link: Pick<ParsedMessageLink, "channelId" | "messageId">,
): Promise<LoadedMessageLink> {
  try {
    const event = await getMessageLinkEvent(link.messageId);
    const eventChannelId = event.tags.find((tag) => tag[0] === "h")?.[1];
    if (
      eventChannelId !== link.channelId ||
      !PREVIEWABLE_MESSAGE_KINDS.has(event.kind)
    ) {
      return { kind: "unavailable" };
    }
    const profile = await getUserProfile(event.pubkey).catch(() => null);
    return { kind: "ready", event, profile };
  } catch (error) {
    return isDefinitiveEventNotFound(error)
      ? { kind: "deleted" }
      : { kind: "unavailable" };
  }
}

export function readyMetadata(
  event: RelayEvent,
  profile: Profile | null,
  content = event.content,
): CachedMessageLinkMetadata {
  return {
    kind: "ready",
    author:
      profile?.displayName?.trim() ||
      profile?.nip05Handle?.trim() ||
      truncateNpub(event.pubkey),
    content,
    createdAt: event.created_at,
    pubkey: event.pubkey,
    snippet: summarizeMessageLinkContent(content),
  };
}

function fetchMetadata(
  link: Pick<ParsedMessageLink, "channelId" | "messageId">,
): Promise<CachedMessageLinkMetadata> {
  const key = `${link.channelId}:${link.messageId}`;
  let request = metadataCache.get(key);
  if (!request) {
    request = loadMessageLink(link).then((loaded) =>
      loaded.kind === "ready"
        ? readyMetadata(loaded.event, loaded.profile)
        : loaded,
    );
    metadataCache.set(key, request);
    void request.then((result) => {
      if (result.kind === "unavailable" && metadataCache.get(key) === request) {
        metadataCache.delete(key);
      }
    });
  }
  return request;
}

export function useMessageLinkMetadata(
  link: ParsedMessageLink,
  channelReadable: boolean,
): { state: MessageLinkMetadataState } {
  const [resolved, setResolved] = React.useState<{
    key: string;
    state: CachedMessageLinkMetadata;
  } | null>(null);
  const requestId = React.useRef(0);
  const key = `${link.channelId}:${link.messageId}`;
  React.useEffect(() => {
    requestId.current += 1;
    if (!channelReadable) return;

    const currentRequest = requestId.current;
    const currentLink = {
      channelId: link.channelId,
      messageId: link.messageId,
    };
    void fetchMetadata(currentLink).then((metadata) => {
      if (requestId.current === currentRequest) {
        setResolved({ key, state: metadata });
      }
    });
    return () => {
      requestId.current += 1;
    };
  }, [channelReadable, key, link.channelId, link.messageId]);
  if (!channelReadable) return { state: { kind: "idle" } };
  return {
    state: resolved?.key === key ? resolved.state : { kind: "loading" },
  };
}
