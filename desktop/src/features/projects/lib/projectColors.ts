import type { Project } from "@/features/projects/projectModels";

/**
 * The eight project colors. A project saves one as its `color` tag; one
 * without a known id gets a fixed color from its d-tag. Text is the 800 in
 * light and the 300 in dark, 4.5:1 on the default themes (pinned by the
 * test); the stripe and the dot are the 500. Class names stay written out
 * so Tailwind finds them. Color is never alone: the name always shows.
 */
export type ProjectColor = {
  id: string;
  label: string;
  text: string;
  stripe: string;
  dot: string;
};

export const PROJECT_COLOR_TAG = "color";

export const PROJECT_COLORS: readonly ProjectColor[] = [
  {
    id: "blue",
    label: "Blue",
    text: "text-blue-800 dark:text-blue-300",
    stripe: "border-l-blue-500",
    dot: "bg-blue-500",
  },
  {
    id: "violet",
    label: "Violet",
    text: "text-violet-800 dark:text-violet-300",
    stripe: "border-l-violet-500",
    dot: "bg-violet-500",
  },
  {
    id: "fuchsia",
    label: "Fuchsia",
    text: "text-fuchsia-800 dark:text-fuchsia-300",
    stripe: "border-l-fuchsia-500",
    dot: "bg-fuchsia-500",
  },
  {
    id: "rose",
    label: "Rose",
    text: "text-rose-800 dark:text-rose-300",
    stripe: "border-l-rose-500",
    dot: "bg-rose-500",
  },
  {
    id: "amber",
    label: "Amber",
    text: "text-amber-800 dark:text-amber-300",
    stripe: "border-l-amber-500",
    dot: "bg-amber-500",
  },
  {
    id: "lime",
    label: "Lime",
    text: "text-lime-800 dark:text-lime-300",
    stripe: "border-l-lime-500",
    dot: "bg-lime-500",
  },
  {
    id: "emerald",
    label: "Emerald",
    text: "text-emerald-800 dark:text-emerald-300",
    stripe: "border-l-emerald-500",
    dot: "bg-emerald-500",
  },
  {
    id: "cyan",
    label: "Cyan",
    text: "text-cyan-800 dark:text-cyan-300",
    stripe: "border-l-cyan-500",
    dot: "bg-cyan-500",
  },
];

export function projectColorById(
  id: string | null | undefined,
): ProjectColor | null {
  return PROJECT_COLORS.find((color) => color.id === id) ?? null;
}

/** The same palette color for the same d-tag, on every device. */
export function fallbackProjectColor(dtag: string): ProjectColor {
  let hash = 0x811c9dc5;
  for (let index = 0; index < dtag.length; index += 1) {
    hash = Math.imul(hash ^ dtag.charCodeAt(index), 0x01000193);
  }
  return PROJECT_COLORS[(hash >>> 0) % PROJECT_COLORS.length];
}

export function projectColorOf(
  project: Pick<Project, "color" | "dtag">,
): ProjectColor {
  return projectColorById(project.color) ?? fallbackProjectColor(project.dtag);
}

/**
 * A new project's color: the first in palette order that the owner's other
 * projects use least, so the first unused one, cycling once all are taken.
 */
export function nextProjectColor(
  projects: ReadonlyArray<Pick<Project, "color" | "dtag" | "owner">>,
  ownerPubkey: string | undefined,
): ProjectColor {
  const owner = ownerPubkey?.toLowerCase();
  const used = projects
    .filter((project) => project.owner.toLowerCase() === owner)
    .map((project) => projectColorOf(project).id);
  const uses = (color: ProjectColor) =>
    used.filter((id) => id === color.id).length;
  return PROJECT_COLORS.reduce((best, color) =>
    uses(color) < uses(best) ? color : best,
  );
}
