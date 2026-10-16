/**
 * The Bench's project tabs: which workbench (28 Day Reset, Dreamforge, or
 * Other) a card belongs to, read from the card's own project, area or writer.
 * Nothing here changes the drop schema; it only reads what drops already say.
 */

import type { BenchCard, BenchSnapshot } from "@/features/bench/lib/benchCards";
import { benchSections } from "@/features/bench/lib/benchScreen";
import { projectDtagFromName } from "@/features/projects/projectCreation";
import { PRODUCT_NAME } from "@/shared/constants/brand";

export type WorkbenchId = "reset" | "dreamforge" | "other";
export type BenchTab = "all" | WorkbenchId;

export type Workbench = {
  id: Exclude<WorkbenchId, "other">;
  label: string;
  /** A slug matches when it equals one of these or starts with one plus "-". */
  slugs: readonly string[];
  /** Writer folders whose cards land here when project and area don't say. */
  writers: readonly string[];
};

export const WORKBENCHES: readonly Workbench[] = [
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

export const BENCH_TABS: ReadonlyArray<{ id: BenchTab; label: string }> = [
  { id: "all", label: "All" },
  ...WORKBENCHES.map(({ id, label }) => ({ id, label })),
  { id: "other", label: "Other" },
];

/**
 * The workbench a slug or a display name names: "28-day-reset", "reset-q4"
 * and "28 Day Reset" are the Reset; "resetting" and "bench" are nothing.
 */
export function slugWorkbench(
  slug: string | null | undefined,
): Workbench | null {
  const s = projectDtagFromName(slug ?? "");
  if (!s) return null;
  return (
    WORKBENCHES.find((w) =>
      w.slugs.some((known) => s === known || s.startsWith(`${known}-`)),
    ) ?? null
  );
}

/** The card's project, else its area, else its writer; else Other. */
export function cardWorkbench(
  card: Pick<BenchCard, "area" | "project" | "writer">,
): WorkbenchId {
  return (
    (
      slugWorkbench(card.project) ??
      slugWorkbench(card.area) ??
      WORKBENCHES.find((w) => w.writers.includes(card.writer))
    )?.id ?? "other"
  );
}

export function workbenchLabel(id: WorkbenchId): string {
  return WORKBENCHES.find((w) => w.id === id)?.label ?? "Other";
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
  for (const { id } of BENCH_TABS) {
    const view = workbenchSnapshot(snapshot, id);
    counts[id] = {
      cards: view.items.filter((card) => card.validity === "current").length,
      needsYou: benchSections(view, viewer).needsYou.length,
    };
  }
  return counts;
}
