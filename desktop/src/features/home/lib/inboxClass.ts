/**
 * What an Inbox row is, for its icon and label, and which project it
 * belongs to, for its chip and stripe in that project's color. Types, first
 * match wins: Needs you, Bench, Code, DM, Mention, then Thread (everything
 * else). This only labels rows; the Bench's project tabs never filter the
 * Inbox.
 */

import {
  type BenchCard,
  type BenchSnapshot,
  indexBenchCards,
} from "@/features/bench/lib/benchCards";
import {
  WORKBENCHES,
  type Workbench,
  cardWorkbench,
  projectWorkbench,
  slugWorkbench,
  workbenchColor,
} from "@/features/bench/lib/workbenches";
import type { InboxItem } from "@/features/home/lib/inbox";
import { getProjectInboxReference } from "@/features/home/lib/projectInbox";
import {
  fallbackProjectColor,
  type ProjectColor,
  projectColorOf,
} from "@/features/projects/lib/projectColors";
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

export type ProjectChip = { label: string; color: ProjectColor };

export type InboxClass = {
  type: InboxItemType;
  /** The project chip's text; null shows no chip. */
  workbench: string | null;
  /** The chip's project color, also the row's stripe; null with no chip. */
  color: ProjectColor | null;
  /** The Bench card a verified Bench post (or its thread) is. */
  benchCard: BenchCard | null;
};

/** The app's Projects as Inbox chips. */
export type InboxProjects = {
  /** Keyed by channel id (home, related, repository) and repo address. */
  chips: ReadonlyMap<string, ProjectChip>;
  workbenches: Readonly<Record<Workbench["id"], ProjectChip>>;
};

type ChipProject = Pick<
  Project,
  | "color"
  | "dtag"
  | "legacy"
  | "name"
  | "projectChannelId"
  | "relatedChannelIds"
  | "repositories"
  | "repositoryAddresses"
>;

/**
 * Every project's channels and repositories to its chip: its workbench's
 * when it names one, else its own name and color. A channel or repository
 * in two projects keeps the first.
 */
export function inboxProjects(
  projects: ReadonlyArray<ChipProject>,
): InboxProjects {
  const workbenches = Object.fromEntries(
    WORKBENCHES.map((w) => [
      w.id,
      { label: w.label, color: workbenchColor(w, projects) },
    ]),
  ) as Record<Workbench["id"], ProjectChip>;
  const chips = new Map<string, ProjectChip>();
  for (const project of projects) {
    const workbench = projectWorkbench(project);
    const chip = workbench
      ? workbenches[workbench.id]
      : { label: project.name, color: projectColorOf(project) };
    const keys = [
      ...listProjectBoundChannels(project).map((bound) => bound.channelId),
      ...project.repositoryAddresses,
    ];
    for (const key of keys) if (!chips.has(key)) chips.set(key, chip);
  }
  return { chips, workbenches };
}

/** The repository address of a Buzz Git row ("30617:<owner>:<d-tag>"). */
function inboxRepoAddress(item: InboxItem): string | null {
  for (const event of item.groupItems) {
    const address = getProjectInboxReference(event)?.repoAddress;
    if (address) return address;
  }
  return null;
}

function addressDtag(address: string): string {
  return address.split(":").slice(2).join(":");
}

/** The repository d-tag of a Buzz Git row. */
export function inboxRepoDtag(item: InboxItem): string | null {
  const address = inboxRepoAddress(item);
  return address ? addressDtag(address) : null;
}

/** A repository no project lists: its workbench's chip, else its d-tag. */
function repoChip(address: string, projects: InboxProjects): ProjectChip {
  const dtag = addressDtag(address);
  const workbench = slugWorkbench(dtag);
  return workbench
    ? projects.workbenches[workbench.id]
    : { label: dtag, color: fallbackProjectColor(dtag) };
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
  projects: InboxProjects,
): InboxClass {
  const benchCard = benchCardOf(item, benchSnapshot);
  const repo = inboxRepoAddress(item);
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
  const chip =
    cardBench !== "other"
      ? projects.workbenches[cardBench]
      : repo
        ? (projects.chips.get(repo) ?? repoChip(repo, projects))
        : channelId
          ? (projects.chips.get(channelId) ?? null)
          : null;
  return {
    type,
    workbench: chip?.label ?? null,
    color: chip?.color ?? null,
    benchCard,
  };
}
