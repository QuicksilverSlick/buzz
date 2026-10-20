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
  compareProjectIds,
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
  | "id"
  | "legacy"
  | "name"
  | "owner"
  | "projectChannelId"
  | "relatedChannelIds"
  | "repositories"
>;

/**
 * Every project's channels and repositories to its chip: its workbench's
 * when it names one, else its own name and color. A project claims only
 * repositories its owner owns or maintains (as `buildProjectReadModels`
 * does). A channel or repository in two projects goes to the viewer's own
 * first, an announced one before a repository-only one, then address order:
 * never recency, so a recolor or someone else's newer project moves nothing.
 */
export function inboxProjects(
  projects: ReadonlyArray<ChipProject>,
  viewerPubkey: string | undefined,
  viewerAgentPubkeys?: ReadonlySet<string>,
): InboxProjects {
  const viewer = viewerPubkey?.toLowerCase();
  const workbenches = Object.fromEntries(
    WORKBENCHES.map((w) => [
      w.id,
      {
        label: w.label,
        color: workbenchColor(w, projects, viewerPubkey, viewerAgentPubkeys),
      },
    ]),
  ) as Record<Workbench["id"], ProjectChip>;
  const rank = (project: ChipProject) =>
    (project.owner.toLowerCase() === viewer ? 0 : 2) + (project.legacy ? 1 : 0);
  const chips = new Map<string, ProjectChip>();
  for (const project of [...projects].sort(
    (a, b) => rank(a) - rank(b) || compareProjectIds(a, b),
  )) {
    const workbench = projectWorkbench(project);
    const chip = workbench
      ? workbenches[workbench.id]
      : { label: project.name, color: projectColorOf(project) };
    const repositories = project.repositories.filter(
      (repository) =>
        repository.owner === project.owner ||
        repository.maintainers?.includes(project.owner),
    );
    const keys = [
      ...listProjectBoundChannels({ ...project, repositories }).map(
        (bound) => bound.channelId,
      ),
      ...repositories.map((repository) => repository.repoAddress),
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
 * The card an event is the post of: it carries the Bench's card marker, the
 * Bench writer key signed it, and it sits in #bench or the card's channel.
 */
function markedCard(
  event: InboxItem["item"],
  snapshot: BenchSnapshot | null,
): BenchCard | null {
  if (
    !snapshot?.writerPubkey ||
    normalizePubkey(event.pubkey) !== normalizePubkey(snapshot.writerPubkey)
  ) {
    return null;
  }
  const marker = event.tags.find(
    (tag) => tag[0] === "client" && tag[1]?.startsWith("bench:card:"),
  )?.[1];
  const id = marker ? CARD_MARKER.exec(marker)?.[1] : undefined;
  const card = id ? snapshot.items.find((c) => c.id === id) : undefined;
  return card &&
    event.channelId &&
    (event.channelId === snapshot.channelId ||
      event.channelId === card.channelId)
    ? card
    : null;
}

/**
 * The card a row is: one of its events is the card's post (`markedCard`),
 * or the row is the thread of a card the Bench itself posted there. Retired
 * cards count: their old posts are still Bench posts.
 */
function benchCardOf(
  item: InboxItem,
  snapshot: BenchSnapshot | null,
): BenchCard | null {
  if (!snapshot?.writerPubkey) return null;
  for (const event of [item.item, ...item.groupItems]) {
    const card = markedCard(event, snapshot);
    if (card) return card;
  }
  const root = indexBenchCards(snapshot).get(item.conversationId);
  return root && item.item.channelId === (root.channelId ?? snapshot.channelId)
    ? root
    : null;
}

/**
 * The row's card when the row shows the card's own post; a reply in its
 * thread is not the card, whatever marker it carries.
 */
export function inboxCardPost(
  item: InboxItem,
  snapshot: BenchSnapshot | null,
): BenchCard | null {
  return markedCard(item.item, snapshot);
}

/**
 * The card waits on the owner, and the row is its live post or thread: an
 * older post of the same card no longer takes an answer.
 */
function waitsOnOwner(card: BenchCard | null, item: InboxItem): boolean {
  const live = card?.cardEventId;
  return (
    card?.kind === "pending" &&
    card.validity === "current" &&
    !card.answer &&
    !!live &&
    (item.conversationId === live ||
      [item.item, ...item.groupItems].some((event) => event.id === live))
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
    item.isActionRequired || waitsOnOwner(benchCard, item)
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
