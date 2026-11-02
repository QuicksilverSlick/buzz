/**
 * The Bench's project tabs: one per project in the registry the snapshot
 * carries (~/.dreamforge/projects.json, read by bench/projects.rs), each
 * sub-product right after its parent, else the built-in list; plus All and
 * Other. A card's tab comes from its own project, area or writer. Nothing
 * here changes the drop schema; it only reads what drops already say.
 */

import type {
  BenchCard,
  BenchCards,
  BenchProject,
  BenchSnapshot,
} from "@/features/bench/lib/benchCards";
import { benchSections } from "@/features/bench/lib/benchScreen";
import {
  fallbackProjectColor,
  PROJECT_COLORS,
  type ProjectColor,
  projectColorOf,
} from "@/features/projects/lib/projectColors";
import { projectDtagFromName } from "@/features/projects/projectCreation";
import type { Project } from "@/features/projects/projectModels";
import { PRODUCT_NAME } from "@/shared/constants/brand";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** A registry slug, or "other". */
export type WorkbenchId = string;
export type BenchTab = "all" | WorkbenchId;

export type Workbench = {
  id: string;
  label: string;
  /** A slug matches when it equals one of these or starts with one plus
   * "-"; the longest match wins, so "dreamforge-talos" is Talos's. */
  slugs: readonly string[];
  /** Writer folders whose cards land here when project and area don't say. */
  writers: readonly string[];
  /** A sub-product's top-level project (Talos under Dreamforge). */
  parent?: string;
};

/** The tabs while the registry is missing or invalid (projects.rs built_in). */
export const BUILT_IN_WORKBENCHES: readonly Workbench[] = [
  {
    id: "reset",
    label: "28 Day Reset",
    slugs: ["28-day-reset", "reset"],
    writers: ["orchestrator"],
  },
  {
    id: "dreamforge",
    label: PRODUCT_NAME,
    slugs: ["dreamforge", "buzz"],
    writers: ["claude-ai", "claude", "upstream-sync"],
  },
];

// ponytail: one registry per app, set from each bench_cards read; pass it
// down instead if two Benches ever share a window.
let current: readonly Workbench[] = BUILT_IN_WORKBENCHES;
let adopted = "[]";

/** The project tabs, in tab order. */
export function workbenches(): readonly Workbench[] {
  return current;
}

/**
 * Takes the registry a snapshot carries; with none (missing, invalid, or the
 * Bench off) the built-in tabs. The writers stay the built-in list's for each
 * slug until the registry names them (P3). Unchanged, nothing moves.
 */
export function adoptBenchProjects(cards: BenchCards | null | undefined): void {
  const projects = (cards?.enabled && cards.projects) || [];
  const key = JSON.stringify(projects);
  if (key === adopted) return;
  adopted = key;
  const bench = (p: BenchProject): Workbench => ({
    id: p.slug,
    label: p.label,
    slugs: [p.slug, ...p.aliases],
    writers: BUILT_IN_WORKBENCHES.find((w) => w.id === p.slug)?.writers ?? [],
    ...(p.parent ? { parent: p.parent } : {}),
  });
  current = projects.length
    ? projects
        .filter((p) => !p.parent)
        .flatMap((p) => [p, ...projects.filter((c) => c.parent === p.slug)])
        .map(bench)
    : BUILT_IN_WORKBENCHES;
}

export type BenchTabEntry = { id: BenchTab; label: string; parent?: string };

/** All, the projects, then Other. */
export function benchTabs(): readonly BenchTabEntry[] {
  return [
    { id: "all", label: "All" },
    ...current.map(({ id, label, parent }) => ({ id, label, parent })),
    { id: "other", label: "Other" },
  ];
}

/** The tab strip is sized for this many projects (design section 8). */
export const INLINE_PROJECT_TABS = 6;

/**
 * Past INLINE_PROJECT_TABS the rest wait in More, those that need the owner
 * first. All, Other and the open tab always show.
 */
export function splitBenchTabs(
  tabs: readonly BenchTabEntry[],
  open: BenchTab,
  counts: Record<BenchTab, WorkbenchCount> | null,
): { inline: BenchTabEntry[]; more: BenchTabEntry[] } {
  const needs = (t: BenchTabEntry) => counts?.[t.id]?.needsYou ?? 0;
  const inline: BenchTabEntry[] = [];
  const more: BenchTabEntry[] = [];
  let projects = 0;
  for (const t of tabs) {
    const fixed = t.id === "all" || t.id === "other";
    if (!fixed) projects += 1;
    (fixed || t.id === open || projects <= INLINE_PROJECT_TABS
      ? inline
      : more
    ).push(t);
  }
  return { inline, more: more.sort((a, b) => needs(b) - needs(a)) };
}

/**
 * The workbench a slug or a display name names: "28-day-reset", "reset-q4"
 * and "28 Day Reset" are the Reset; "resetting" and "bench" are nothing.
 */
export function slugWorkbench(
  slug: string | null | undefined,
): Workbench | null {
  const s = projectDtagFromName(slug ?? "");
  let best: Workbench | null = null;
  let length = 0;
  for (const w of s ? current : []) {
    for (const known of w.slugs) {
      if ((s === known || s.startsWith(`${known}-`)) && known.length > length) {
        best = w;
        length = known.length;
      }
    }
  }
  return best;
}

/** The workbench a project's d-tag, else its name, names. */
export function projectWorkbench(
  project: Pick<Project, "dtag" | "name">,
): Workbench | null {
  return slugWorkbench(project.dtag) ?? slugWorkbench(project.name);
}

/** Address order: a stable tie-break that a republish never changes. */
export function compareProjectIds(
  left: Pick<Project, "id">,
  right: Pick<Project, "id">,
): number {
  return left.id < right.id ? -1 : left.id > right.id ? 1 : 0;
}

type ColorProjects = ReadonlyArray<
  Pick<Project, "color" | "dtag" | "id" | "legacy" | "name" | "owner">
>;

/**
 * A workbench's color: the viewer's own project for it, else one of their
 * agents' (which they can recolor too); else the color a project with its
 * first slug gets, or, when another tab already shows that, the first color
 * no tab shows (so Talos, with no app project yet, never looks like Reset).
 * Earlier tabs keep theirs, and past the palette colors repeat.
 */
// ponytail: tabs^2 x projects per call, fine for the 6 to 12 tabs the strip is
// sized for; a per-render color map if the registry nears its 50 projects.
export function workbenchColor(
  workbench: Workbench,
  projects: ColorProjects,
  viewerPubkey: string | undefined,
  viewerAgentPubkeys: ReadonlySet<string> = new Set(),
): ProjectColor {
  const own = (w: Workbench) =>
    ownProjectColor(w, projects, viewerPubkey, viewerAgentPubkeys);
  const color = own(workbench);
  if (color) return color;
  const shown = new Set(current.flatMap((w) => own(w)?.id ?? []));
  for (const w of current) {
    if (own(w)) continue;
    const slug = fallbackProjectColor(w.slugs[0]);
    const pick = shown.has(slug.id)
      ? (PROJECT_COLORS.find((c) => !shown.has(c.id)) ?? slug)
      : slug;
    if (w.id === workbench.id) return pick;
    shown.add(pick.id);
  }
  return fallbackProjectColor(workbench.slugs[0]);
}

/**
 * The project that colors a workbench: a d-tag equal to a slug (in slug
 * order) beats one that only starts with one or names it, an announced project
 * beats a repository-only one, then address order. Never a stranger's project,
 * and never recency: a recolor moves nothing.
 */
function ownProjectColor(
  workbench: Workbench,
  projects: ColorProjects,
  viewerPubkey: string | undefined,
  viewerAgentPubkeys: ReadonlySet<string>,
): ProjectColor | null {
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : undefined;
  const whose = (project: (typeof projects)[number]) => {
    const owner = normalizePubkey(project.owner);
    return owner === viewer ? 0 : viewerAgentPubkeys.has(owner) ? 1 : -1;
  };
  const rank = (project: (typeof projects)[number]) => {
    const exact = workbench.slugs.indexOf(project.dtag);
    return (
      (exact < 0 ? workbench.slugs.length : exact) * 2 +
      (project.legacy ? 1 : 0)
    );
  };
  const [project] = projects
    .filter(
      (candidate) =>
        whose(candidate) >= 0 &&
        projectWorkbench(candidate)?.id === workbench.id,
    )
    .sort(
      (a, b) =>
        whose(a) - whose(b) || rank(a) - rank(b) || compareProjectIds(a, b),
    );
  return project ? projectColorOf(project) : null;
}

/** The card's project, else its area, else its writer; else Other. */
export function cardWorkbench(
  card: Pick<BenchCard, "area" | "project" | "writer">,
): WorkbenchId {
  return (
    (
      slugWorkbench(card.project) ??
      slugWorkbench(card.area) ??
      current.find((w) => w.writers.includes(card.writer))
    )?.id ?? "other"
  );
}

/**
 * Why the Other tab's cards are there and how they get a tab: "3 cards name
 * no project: fathom (2), your-desktop (1)."
 */
export function otherReason(
  cards: ReadonlyArray<Pick<BenchCard, "area" | "project" | "validity">>,
): string | null {
  const live = cards.filter((card) => card.validity === "current");
  if (live.length === 0) return null;
  const names = new Map<string, number>();
  for (const { area, project } of live) {
    names.set(project ?? area, (names.get(project ?? area) ?? 0) + 1);
  }
  const list = [...names]
    .sort((a, b) => b[1] - a[1])
    .map(([name, count]) => `${name} (${count})`)
    .join(", ");
  const lead = live.length === 1 ? "1 card names" : `${live.length} cards name`;
  return `${lead} no project: ${list}. A project's alias in projects.json gives them a tab.`;
}

export function workbenchLabel(id: WorkbenchId): string {
  return current.find((w) => w.id === id)?.label ?? "Other";
}

/** The snapshot as one tab sees it; All is the whole Bench. */
export function workbenchSnapshot(
  snapshot: BenchSnapshot,
  tab: BenchTab,
): BenchSnapshot {
  return tab === "all"
    ? snapshot
    : {
        ...snapshot,
        items: snapshot.items.filter((card) => cardWorkbench(card) === tab),
      };
}

export type WorkbenchCount = { cards: number; needsYou: number };

/** Current cards and the "Needs you" section's size, per tab. */
export function workbenchCounts(
  snapshot: BenchSnapshot,
  viewer?: string,
): Record<BenchTab, WorkbenchCount> {
  const counts = {} as Record<BenchTab, WorkbenchCount>;
  for (const { id } of benchTabs()) {
    const view = workbenchSnapshot(snapshot, id);
    counts[id] = {
      cards: view.items.filter((card) => card.validity === "current").length,
      needsYou: benchSections(view, viewer).needsYou.length,
    };
  }
  return counts;
}
