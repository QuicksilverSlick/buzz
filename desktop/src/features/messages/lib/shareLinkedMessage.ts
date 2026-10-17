import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import type { Channel } from "@/shared/api/types";
import { maskMarkdownCode } from "@/shared/lib/mentionBoundaries";
import { normalizePubkey } from "@/shared/lib/pubkey";

import { formatFullDateTime } from "./dateFormatters";
import { type ParsedMessageLink, parseMessageLink } from "./messageLink";
import {
  MESSAGE_URL_PATTERN,
  trimMessageLinkMatch,
} from "./remarkMessageLinks";

// Invisible, and breaks every `@name` and `nostr:` matcher (composer, renderer, SDK).
const WORD_JOINER = String.fromCharCode(0x2060);
// The boundary `getMentionOffsets` accepts before an `@`.
const MENTION_START = /(^|[\s(*_|])@(?=\S)/g;
const NOSTR_REFERENCE = /\bnostr:(?=n(?:pub|profile|event|ote|addr)1)/gi;
// CommonMark ends a line at CR too; U+2028/9 look like one.
const LINE_BREAK = /\r\n|\r|\n|\u2028|\u2029/;

type LinkedMessage =
  | { kind: "ready"; author: string; content: string; createdAt: number }
  | { kind: "deleted" | "unavailable" };

export type LinkedMessageShareContext = {
  channelId: string;
  channels: readonly Channel[];
  /** Everyone this send reaches: members, mentioned and always-addressed agents. */
  recipientPubkeys: ReadonlySet<string>;
  /** Agents this send creates (persona mentions); they're in no channel yet. */
  newRecipientNames?: readonly string[];
  ownerPubkey: string | null;
  getMemberPubkeys: (channelId: string) => Promise<readonly string[]>;
  getMessage: (link: ParsedMessageLink) => Promise<LinkedMessage>;
  getDisplayName: (pubkey: string) => string;
};

export type LinkedMessageShare = {
  names: string[];
  quote: string;
  /** The open channel being written in: everyone who reads it sees the quote. */
  openChannel: string | null;
};

/**
 * Break `@name` mentions and `nostr:` references so a quote can never ping
 * anyone. Code and `@` inside words (emails, URL paths) are left as is:
 * no matcher reads them.
 */
export function neutralizeMentions(text: string): string {
  const masked = maskMarkdownCode(text);
  const breaks = new Set<number>();
  for (const match of masked.matchAll(MENTION_START)) {
    breaks.add(match.index + match[1].length + 1);
  }
  for (const match of masked.matchAll(NOSTR_REFERENCE)) {
    breaks.add(match.index + "nostr".length);
  }
  let result = "";
  let last = 0;
  for (const at of [...breaks].sort((a, b) => a - b)) {
    result += text.slice(last, at) + WORD_JOINER;
    last = at;
  }
  return result + text.slice(last);
}

/** One line, markdown taken literally: a name can't end the quote or style it. */
function inlineText(text: string): string {
  return text
    .split(LINE_BREAK)
    .join(" ")
    .replace(/[\\`*_[\]()<>!~|&]/g, "\\$&");
}

function placeOf(
  channel: Channel,
  link: ParsedMessageLink,
  ownerPubkey: string | null,
): string {
  const place =
    channel.channelType === "dm"
      ? `DM with ${resolveChannelDisplayLabel(channel, ownerPubkey ?? undefined, undefined)}`
      : `#${channel.name}`;
  return link.threadRootId ? `a thread in ${place}` : place;
}

/** A labeled markdown blockquote; attachments keep only their link text. */
export function formatSharedQuote(
  message: { author: string; content: string; createdAt: number },
  place: string,
): string {
  const header = `Quoted from **${inlineText(message.author)}** in ${inlineText(place)}, ${formatFullDateTime(message.createdAt)}:`;
  const body = neutralizeMentions(
    message.content.replace(/!\[([^\]]*)\]\(/g, "[$1]("),
  );
  return [neutralizeMentions(header), "", ...body.split(LINE_BREAK)]
    .map((line) => (line ? `> ${line}` : ">"))
    .join("\n");
}

/** The message links in a draft, as the renderer reads them: no trailing punctuation. */
function messageLinks(content: string): string[] {
  return [
    ...new Set(
      [...content.matchAll(MESSAGE_URL_PATTERN)].map(
        ([match]) => trimMessageLinkMatch(match).value,
      ),
    ),
  ];
}

/**
 * Who this send reaches that can't open the private message links in
 * `content`, and the quote that would show them the text. Null means send as
 * is: the link is public, same-channel, unknown to the owner, or unreadable
 * to them.
 */
export async function findLinkedMessageShare(
  content: string,
  context: LinkedMessageShareContext,
): Promise<LinkedMessageShare | null> {
  const owner = normalizePubkey(context.ownerPubkey ?? "");
  const missing = new Set<string>();
  const quotes: string[] = [];
  const newRecipients = context.newRecipientNames ?? [];
  for (const href of messageLinks(content)) {
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
    const lacking = [...context.recipientPubkeys]
      .map(normalizePubkey)
      .filter((pubkey) => pubkey !== owner && !allowed.has(pubkey));
    if (lacking.length === 0 && newRecipients.length === 0) continue;
    const message = await context.getMessage(link);
    if (message.kind !== "ready") continue;
    for (const pubkey of lacking) missing.add(pubkey);
    quotes.push(
      formatSharedQuote(message, placeOf(channel, link, context.ownerPubkey)),
    );
  }
  if (quotes.length === 0) return null;
  const here = context.channels.find((c) => c.id === context.channelId);
  return {
    names: [
      ...new Set([
        ...[...missing].map(context.getDisplayName),
        ...newRecipients,
      ]),
    ],
    quote: quotes.join("\n\n"),
    openChannel:
      here?.visibility === "open" && here.channelType !== "dm"
        ? here.name
        : null,
  };
}

/**
 * What a send carries once the owner has answered, or null while it waits on
 * them: the draft as is for "Send link only", the draft then the quote for
 * "Include quote", or the draft after `check` finds nothing to ask.
 */
export async function resolveShareSend(
  draft: string,
  choice: { quote: string | null } | null,
  check: (draft: string) => Promise<boolean>,
): Promise<string | null> {
  if (choice) return choice.quote ? `${draft}\n\n${choice.quote}` : draft;
  return (await check(draft)) ? null : draft;
}
