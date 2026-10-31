/**
 * Global search over what the app already holds for its other screens:
 * Inbox rows and the agent library. Pure: each hit names where it opens;
 * the dialog navigates.
 */

import { getLibraryPersonas } from "@/features/agents/lib/catalog";
import {
  formatInboxTypeLabel,
  type InboxItem,
} from "@/features/home/lib/inbox";
import type { InboxClass } from "@/features/home/lib/inboxClass";
import {
  filterInboxItems,
  matchesInboxAllView,
} from "@/features/home/lib/inboxViewHelpers";
import type { ProjectColor } from "@/features/projects/lib/projectColors";
import { matchesProjectsSearch } from "@/features/projects/lib/projectsSearch";
import { parsePrQuery, prMentionScore } from "@/features/search/lib/prNumber";
import type { AgentPersona } from "@/shared/api/personaTypes";

export type SurfaceKey = "inbox" | "agent-library";

export type SurfaceTarget =
  | { to: "inbox"; item: string }
  | { to: "persona"; personaId: string };

export type SurfaceHit = {
  surface: SurfaceKey;
  id: string;
  /** Inbox type or "persona": the row's icon and label. */
  kind: string;
  title: string;
  meta: string | null;
  chips: string[];
  color: ProjectColor | null;
  target: SurfaceTarget;
};

type Values = ReadonlyArray<string | null | undefined>;
/** A hit and the text it is found by. */
type Candidate = SurfaceHit & { text: Values };

/**
 * PR-shaped queries match whole tokens ("#792" never 7928 or 792d3edd);
 * anything else matches every word, as the Projects search does.
 */
function matchesLocal(query: string, values: Values): boolean {
  // shortcut: a repo in the query ("reset#792") is ignored, as the Messages
  // section does; check pr.repo if rows that name another repo get noisy.
  const pr = parsePrQuery(query);
  if (!pr) return matchesProjectsSearch(query, values);
  const text = values.filter(Boolean).join("\n");
  return prMentionScore(text, pr.number) >= (pr.explicit ? 2 : 1);
}

/** The first `limit` hits that match, one per id. */
function take<T>(
  items: Iterable<T>,
  query: string,
  limit: number,
  toHit: (item: T) => Candidate | null,
): SurfaceHit[] {
  const hits = new Map<string, SurfaceHit>();
  for (const item of items) {
    if (hits.size >= limit) break;
    const hit = toHit(item);
    if (hit && !hits.has(hit.id) && matchesLocal(query, hit.text)) {
      hits.set(hit.id, hit);
    }
  }
  return [...hits.values()];
}

// Rows truncate it.
const firstLine = (text: string) => text.trim().split("\n")[0] ?? "";

/**
 * Only rows the Inbox's All view lists, so each one opens a row; a Bench
 * card's post is left to the Bench cards section.
 */
export function inboxHits(
  items: InboxItem[],
  classify: (item: InboxItem) => InboxClass,
  ownedAgents: ReadonlySet<string>,
  query: string,
  limit = 4,
): SurfaceHit[] {
  const listed = filterInboxItems(items).filter((item) =>
    matchesInboxAllView(item, ownedAgents),
  );
  return take(listed, query, limit, (item) => {
    const { benchCard, color, type, workbench } = classify(item);
    if (benchCard) return null;
    const { channelLabel, id, preview, senderLabel, subject } = item;
    return {
      surface: "inbox",
      id,
      kind: type,
      title: firstLine(preview) || subject,
      meta: formatInboxTypeLabel(item),
      chips: workbench ? [workbench] : [],
      color,
      target: { to: "inbox", item: id },
      text: [
        subject,
        preview,
        senderLabel,
        channelLabel,
        item.categoryLabel,
        ...item.mentionNames,
      ],
    };
  });
}

/**
 * The Agents screen's library (active personas), by name and public
 * description only: never the prompt, env or commands.
 */
export function personaHits(
  personas: readonly AgentPersona[],
  query: string,
  limit = 3,
): SurfaceHit[] {
  return take(
    getLibraryPersonas(personas),
    query,
    limit,
    (persona): Candidate => ({
      surface: "agent-library",
      id: persona.id,
      kind: "persona",
      title: persona.displayName,
      meta: persona.description,
      chips: [persona.model, persona.runtime].flatMap((chip) =>
        chip ? [chip] : [],
      ),
      color: null,
      target: { to: "persona", personaId: persona.id },
      text: [persona.displayName, persona.description],
    }),
  );
}
