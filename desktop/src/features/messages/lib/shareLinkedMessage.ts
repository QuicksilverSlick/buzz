import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import { formatFullDateTime } from "./dateFormatters";
import { type ParsedMessageLink, parseMessageLink } from "./messageLink";

/** The "Copy link" format, as the composer writes it back out (bare or in `<>`/`()`). */
const MESSAGE_LINK_PATTERN = /buzz:\/\/message\?[^\s<>)\]]+/g;
// Invisible, and breaks every `@name` and `nostr:` matcher (composer, renderer, SDK).
const WORD_JOINER = String.fromCharCode(0x2060);

type LinkedMessage =
  | { kind: "ready"; author: string; content: string; createdAt: number }
  | { kind: "deleted" | "unavailable" };

export type LinkedMessageShareContext = {
  channelId: string;
  channels: readonly Channel[];
  /** Everyone in the conversation being written in, agents included. */
  memberPubkeys: ReadonlySet<string>;
  ownerPubkey: string | null;
  getMemberPubkeys: (channelId: string) => Promise<readonly string[]>;
  getMessage: (link: ParsedMessageLink) => Promise<LinkedMessage>;
  getDisplayName: (pubkey: string) => string;
};

/** Break `@name` and `nostr:` references so a quote can never ping anyone. */
export function neutralizeMentions(text: string): string {
  return text
    .replaceAll("@", `@${WORD_JOINER}`)
    .replace(/(nostr):/gi, `$1${WORD_JOINER}:`);
}

function placeOf(channel: Channel, link: ParsedMessageLink): string {
  const place =
    channel.channelType === "dm"
      ? `DM with ${channel.name}`
      : `#${channel.name}`;
  return link.threadRootId ? `a thread in ${place}` : place;
}

/** A labeled markdown blockquote; attachments keep only their link text. */
export function formatSharedQuote(
  message: { author: string; content: string; createdAt: number },
  place: string,
): string {
  const body = message.content.replace(/!\[([^\]]*)\]\(/g, "[$1](");
  const lines = [
    `Quoted from **${message.author}** in ${place}, ${formatFullDateTime(message.createdAt)}:`,
    "",
    ...body.split(/\r?\n/),
  ];
  return neutralizeMentions(
    lines.map((line) => (line ? `> ${line}` : ">")).join("\n"),
  );
}

/**
 * Who in this conversation can't open the private message links in `content`,
 * and the quote that would show them the text. Null means send as is: the
 * link is public, same-channel, unknown to the owner, or unreadable to them.
 */
export async function findLinkedMessageShare(
  content: string,
  context: LinkedMessageShareContext,
): Promise<{ names: string[]; quote: string } | null> {
  const owner = normalizePubkey(context.ownerPubkey ?? "");
  const missing = new Set<string>();
  const quotes: string[] = [];
  for (const href of new Set(content.match(MESSAGE_LINK_PATTERN))) {
    const parsed = parseMessageLink(href);
    if (!parsed.ok || parsed.value.channelId === context.channelId) continue;
    const link = parsed.value;
    const channel = context.channels.find((c) => c.id === link.channelId);
    if (
      !channel?.isMember ||
      (channel.visibility === "open" && channel.channelType !== "dm")
    ) {
      continue;
    }
    const allowed = new Set(
      (await context.getMemberPubkeys(channel.id)).map(normalizePubkey),
    );
    const lacking = [...context.memberPubkeys].filter(
      (pubkey) => pubkey !== owner && !allowed.has(pubkey),
    );
    if (lacking.length === 0) continue;
    const message = await context.getMessage(link);
    if (message.kind !== "ready") continue;
    for (const pubkey of lacking) missing.add(pubkey);
    quotes.push(formatSharedQuote(message, placeOf(channel, link)));
  }
  if (quotes.length === 0) return null;
  return {
    names: [...missing].map(context.getDisplayName),
    quote: quotes.join("\n\n"),
  };
}
