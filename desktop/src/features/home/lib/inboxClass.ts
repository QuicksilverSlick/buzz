/**
 * What an Inbox row is, for its color, icon and label, and which project it
 * belongs to, for its neutral chip. Types, first match wins: Needs you,
 * Bench, Code, DM, Mention, then Thread (everything else). This only labels
 * rows; the Bench's project tabs never filter the Inbox.
 */

import {
  type BenchCard,
  type BenchSnapshot,
  indexBenchCards,
} from "@/features/bench/lib/benchCards";
import {
  cardWorkbench,
  slugWorkbench,
  workbenchLabel,
} from "@/features/bench/lib/workbenches";
import type { InboxItem } from "@/features/home/lib/inbox";
import { getProjectInboxReference } from "@/features/home/lib/projectInbox";
import { listProjectBoundChannels } from "@/features/projects/lib/projectRelatedChannels";
import type { Project } from "@/features/projects/projectModels";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type InboxItemType =
  | "needs-you"
  | "bench"
  | "code"
  | "dm"
  | "mention"
  | "thread";

export type InboxClass = {
  type: InboxItemType;
  /** The project chip's text; null shows no chip. */
  workbench: string | null;
  /** The Bench card a verified Bench post (or its thread) is. */
  benchCard: BenchCard | null;
};

/** Channel id to project chip, from the app's Projects. */
export type ChannelProjects = ReadonlyMap<string, string>;

/** A workbench's name when the slug or name names one, else the name. */
function projectChip(dtag: string, name: string): string {
  return (slugWorkbench(dtag) ?? slugWorkbench(name))?.label ?? name;
}

/**
 * Every project's home and related channels to that project's chip. A
 * channel bound to two projects keeps the first.
 */
export function channelProjects(
  projects: ReadonlyArray<
    Pick<
      Project,
      | "dtag"
      | "name"
      | "projectChannelId"
      | "relatedChannelIds"
      | "repositories"
    >
  >,
): ChannelProjects {
  const lookup = new Map<string, string>();
  for (const project of projects) {
    const chip = projectChip(project.dtag, project.name);
    for (const { channelId } of listProjectBoundChannels(project)) {
      if (!lookup.has(channelId)) lookup.set(channelId, chip);
    }
  }
  return lookup;
}

/** The repository d-tag of a Buzz Git row ("30617:<owner>:<d-tag>"). */
export function inboxRepoDtag(item: InboxItem): string | null {
  for (const event of item.groupItems) {
    const address = getProjectInboxReference(event)?.repoAddress;
    if (address) return address.split(":").slice(2).join(":");
  }
  return null;
}

const CARD_MARKER = /^bench:card:(.+)@[^@]+$/;

/**
 * The card a row is: one of its events carries the Bench's card marker, the
 * Bench writer key signed it, and it sits in #bench or the card's channel;
 * or the row is the thread of a card the Bench itself posted there. Retired
 * cards count: their old posts are still Bench posts.
 */
function benchCardOf(
  item: InboxItem,
  snapshot: BenchSnapshot | null,
): BenchCard | null {
  if (!snapshot?.writerPubkey) return null;
  const writer = normalizePubkey(snapshot.writerPubkey);
  const inCardChannel = (card: BenchCard, channelId: string | null) =>
    Boolean(channelId) &&
    (channelId === snapshot.channelId || channelId === card.channelId);
  for (const event of [item.item, ...item.groupItems]) {
    if (normalizePubkey(event.pubkey) !== writer) continue;
    const marker = event.tags.find(
      (tag) => tag[0] === "client" && tag[1]?.startsWith("bench:card:"),
    )?.[1];
    const id = marker ? CARD_MARKER.exec(marker)?.[1] : undefined;
    const card = id ? snapshot.items.find((c) => c.id === id) : undefined;
    if (card && inCardChannel(card, event.channelId)) return card;
  }
  const root = indexBenchCards(snapshot).get(item.conversationId);
  return root && item.item.channelId === (root.channelId ?? snapshot.channelId)
    ? root
    : null;
}

function waitsOnOwner(card: BenchCard | null): boolean {
  return (
    card?.kind === "pending" && card.validity === "current" && !card.answer
  );
}

export function classifyInboxItem(
  item: InboxItem,
  benchSnapshot: BenchSnapshot | null,
  channelProject: ChannelProjects,
): InboxClass {
  const benchCard = benchCardOf(item, benchSnapshot);
  const repo = inboxRepoDtag(item);
  const type: InboxItemType =
    item.isActionRequired || waitsOnOwner(benchCard)
      ? "needs-you"
      : benchCard
        ? "bench"
        : repo !== null
          ? "code"
          : item.item.channelType === "dm"
            ? "dm"
            : item.item.category === "mention"
              ? "mention"
              : "thread";
  const cardBench = benchCard ? cardWorkbench(benchCard) : "other";
  const channelId = item.item.channelId;
  const workbench =
    cardBench !== "other"
      ? workbenchLabel(cardBench)
      : repo
        ? projectChip(repo, repo)
        : channelId
          ? (channelProject.get(channelId) ?? null)
          : null;
  return { type, workbench, benchCard };
}
