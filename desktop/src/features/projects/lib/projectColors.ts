import type { Project } from "@/features/projects/projectModels";

/**
 * The eight project colors. A project saves one as its `color` tag; one
 * without a known id gets a fixed color from its d-tag. Text is the 800 in
 * light and the 300 in dark, 4.5:1 on the default themes (pinned by the
 * test); the stripe and the dot are the 600 in light and the 400 in dark.
 * Class names stay written out so Tailwind finds them. Color is never
 * alone: the name always shows.
 */
export type ProjectColor = {
  id: string;
  label: string;
  text: string;
  stripe: string;
  dot: string;
};

export const PROJECT_COLOR_TAG = "color";

/**
 * Palette order is the auto-pick order. None is an Inbox type's hue (amber,
 * violet, blue, emerald, fuchsia), and each stripe is at least 0.1 OKLab
 * from every type stripe on the default themes.
 */
export const PROJECT_COLORS: readonly ProjectColor[] = [
  {
    id: "lime",
    label: "Lime",
    text: "text-lime-800 dark:text-lime-300",
    stripe: "border-l-lime-600 dark:border-l-lime-400",
    dot: "bg-lime-600 dark:bg-lime-400",
  },
  {
    id: "sky",
    label: "Sky",
    text: "text-sky-800 dark:text-sky-300",
    stripe: "border-l-sky-600 dark:border-l-sky-400",
    dot: "bg-sky-600 dark:bg-sky-400",
  },
  {
    id: "red",
    label: "Red",
    text: "text-red-800 dark:text-red-300",
    stripe: "border-l-red-600 dark:border-l-red-400",
    dot: "bg-red-600 dark:bg-red-400",
  },
  {
    id: "teal",
    label: "Teal",
    text: "text-teal-800 dark:text-teal-300",
    stripe: "border-l-teal-600 dark:border-l-teal-400",
    dot: "bg-teal-600 dark:bg-teal-400",
  },
  {
    id: "orange",
    label: "Orange",
    text: "text-orange-800 dark:text-orange-300",
    stripe: "border-l-orange-600 dark:border-l-orange-400",
    dot: "bg-orange-600 dark:bg-orange-400",
  },
  {
    id: "indigo",
    label: "Indigo",
    text: "text-indigo-800 dark:text-indigo-300",
    stripe: "border-l-indigo-600 dark:border-l-indigo-400",
    dot: "bg-indigo-600 dark:bg-indigo-400",
  },
  {
    id: "pink",
    label: "Pink",
    text: "text-pink-800 dark:text-pink-300",
    stripe: "border-l-pink-600 dark:border-l-pink-400",
    dot: "bg-pink-600 dark:bg-pink-400",
  },
  {
    id: "yellow",
    label: "Yellow",
    text: "text-yellow-800 dark:text-yellow-300",
    stripe: "border-l-yellow-600 dark:border-l-yellow-400",
    dot: "bg-yellow-600 dark:bg-yellow-400",
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
