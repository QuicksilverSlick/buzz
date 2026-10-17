import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";

import { createThemeVars, luminance } from "@/shared/theme/adaptive-theme.ts";
import {
  extractThemeInfo,
  loadThemeData,
} from "@/shared/theme/theme-loader.ts";
import { buildProjectBootstrapTemplates } from "../projectCreation.ts";
import { buildProjectReadModels } from "../projectModels.ts";
import { buildProjectColorPatchTemplate } from "../projectRepositoryCreation.ts";
import {
  PROJECT_COLORS,
  fallbackProjectColor,
  nextProjectColor,
  projectColorById,
  projectColorOf,
} from "./projectColors.ts";

const OWNER = "a".repeat(64);
const OTHER = "b".repeat(64);
const CHANNEL = "11111111-1111-4111-8111-111111111111";

const tailwindTheme = readFileSync(
  createRequire(import.meta.url).resolve("tailwindcss/theme.css"),
  "utf8",
);

/** Tailwind's oklch shade to relative luminance, clipped to sRGB. */
function shadeLuminance(name) {
  const match = new RegExp(
    `--color-${name}:\\s*oklch\\(([\\d.]+)% ([\\d.]+) ([\\d.]+)\\)`,
  ).exec(tailwindTheme);
  assert.ok(match, `Tailwind has no ${name}`);
  const L = Number(match[1]) / 100;
  const h = (Number(match[3]) * Math.PI) / 180;
  const a = Number(match[2]) * Math.cos(h);
  const b = Number(match[2]) * Math.sin(h);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const clip = (v) => Math.min(1, Math.max(0, v));
  const r = clip(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s);
  const g = clip(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s);
  const bl = clip(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s);
  return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
}

/** A theme var ("H S% L%") to relative luminance. */
function hslLuminance(value) {
  const [h, s, l] = value.split(" ").map((part) => Number.parseFloat(part));
  const k = (n) => (n + h / 30) % 12;
  const amp = (s / 100) * Math.min(l / 100, 1 - l / 100);
  const channel = (n) =>
    Math.round(
      255 * (l / 100 - amp * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1))),
    );
  const hex = [0, 8, 4]
    .map((n) => channel(n).toString(16).padStart(2, "0"))
    .join("");
  return luminance(`#${hex}`);
}

function contrast(a, b) {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

test("the palette is eight distinct colors with their classes written out", () => {
  assert.equal(PROJECT_COLORS.length, 8);
  assert.equal(new Set(PROJECT_COLORS.map((c) => c.id)).size, 8);
  for (const color of PROJECT_COLORS) {
    assert.equal(
      color.text,
      `text-${color.id}-800 dark:text-${color.id}-300`,
      color.id,
    );
    assert.equal(color.stripe, `border-l-${color.id}-500`);
    assert.equal(color.dot, `bg-${color.id}-500`);
    assert.ok(color.label);
  }
  assert.equal(projectColorById("blue")?.id, "blue");
  assert.equal(projectColorById("red"), null);
  assert.equal(projectColorById(undefined), null);
});

test("project text reads at 4.5:1 on the default light and dark themes", async () => {
  for (const name of [
    "buzz",
    "buzz-dark",
    "catppuccin-latte",
    "catppuccin-macchiato",
  ]) {
    const info = extractThemeInfo(name, await loadThemeData(name));
    const { isDark, vars } = createThemeVars(info.bg, info.fg, info.comment);
    for (const surface of ["--background", "--muted", "--popover"]) {
      const back = hslLuminance(vars[surface]);
      for (const color of PROJECT_COLORS) {
        const shade = `${color.id}-${isDark ? 300 : 800}`;
        const ratio = contrast(shadeLuminance(shade), back);
        assert.ok(
          ratio >= 4.5,
          `${shade} on ${name} ${surface} is ${ratio.toFixed(2)}:1`,
        );
      }
    }
  }
});

test("a project with no known color gets the same one from its d-tag", () => {
  const reset = fallbackProjectColor("28-day-reset");
  assert.equal(fallbackProjectColor("28-day-reset"), reset);
  assert.equal(projectColorOf({ dtag: "28-day-reset" }), reset);
  assert.equal(projectColorOf({ dtag: "28-day-reset", color: "nope" }), reset);
  assert.equal(
    projectColorOf({ dtag: "28-day-reset", color: "lime" }).id,
    "lime",
  );
  const spread = new Set(
    Array.from({ length: 64 }, (_, i) => fallbackProjectColor(`p-${i}`).id),
  );
  assert.ok(spread.size >= 6, "fallbacks spread over the palette");
});

test("a new project takes the owner's first unused color, then cycles", () => {
  const ids = PROJECT_COLORS.map((c) => c.id);
  const mine = (color, dtag = color) => ({ color, dtag, owner: OWNER });
  assert.equal(nextProjectColor([], OWNER).id, ids[0]);
  assert.equal(nextProjectColor([mine(ids[0])], OWNER).id, ids[1]);
  assert.equal(
    nextProjectColor([mine(ids[1]), mine(ids[0]), mine(ids[3])], OWNER).id,
    ids[2],
  );
  // Someone else's projects don't count; the owner key's case doesn't matter.
  assert.equal(
    nextProjectColor([{ ...mine(ids[0]), owner: OTHER }], OWNER).id,
    ids[0],
  );
  assert.equal(
    nextProjectColor([mine(ids[0])], OWNER.toUpperCase()).id,
    ids[1],
  );
  // A project without a tag uses its fallback color.
  const fallback = fallbackProjectColor("old");
  assert.notEqual(
    nextProjectColor([mine(undefined, "old")], OWNER).id,
    fallback.id,
  );
  // All eight taken: back to the first; then the next one taken least.
  const all = ids.map((id) => mine(id));
  assert.equal(nextProjectColor(all, OWNER).id, ids[0]);
  assert.equal(nextProjectColor([...all, mine(ids[0])], OWNER).id, ids[1]);
});

function created(color) {
  const templates = buildProjectBootstrapTemplates({
    color,
    name: "28 Day Reset",
    ownerPubkey: OWNER,
    projectChannelId: CHANNEL,
  });
  return {
    id: "p1",
    pubkey: OWNER,
    created_at: 100,
    content: templates.project.content,
    kind: templates.project.kind,
    tags: templates.project.tags,
  };
}

function readOne(projectEvent) {
  const [project] = buildProjectReadModels({
    projectEvents: [projectEvent],
    repositoryEvents: [],
  });
  return project;
}

test("the color tag round-trips through create and read", () => {
  const event = created("emerald");
  assert.deepEqual(
    event.tags.filter((tag) => tag[0] === "color"),
    [["color", "emerald"]],
  );
  assert.equal(readOne(event).color, "emerald");
  assert.equal(projectColorOf(readOne(event)).id, "emerald");

  const untagged = created(undefined);
  assert.ok(!untagged.tags.some((tag) => tag[0] === "color"));
  assert.equal(readOne(untagged).color, undefined);
  assert.equal(
    projectColorOf(readOne(untagged)),
    fallbackProjectColor("28-day-reset"),
  );
  assert.throws(() => created("plaid"), /color is invalid/);
});

test("reading takes the first known color and never drops the project", () => {
  const base = created(undefined);
  const withTags = (...tags) => ({ ...base, tags: [...base.tags, ...tags] });
  assert.equal(readOne(withTags(["color", "plaid"])).color, undefined);
  assert.equal(
    readOne(withTags(["color", "plaid"], ["color", "rose"])).color,
    "rose",
  );
  assert.equal(
    readOne(withTags(["color", "cyan"], ["color", "rose"])).color,
    "cyan",
  );
});

test("a repository-only project gets its d-tag's color", () => {
  const [legacy] = buildProjectReadModels({
    projectEvents: [],
    repositoryEvents: [
      {
        id: "r1",
        kind: 30617,
        pubkey: OWNER,
        created_at: 1,
        content: "",
        tags: [["d", "notes"]],
      },
    ],
  });
  assert.equal(legacy.legacy, true);
  assert.equal(projectColorOf(legacy), fallbackProjectColor("notes"));
});

test("saving a color replaces only the color tag", () => {
  const live = {
    ...created("blue"),
    content: "kept verbatim",
    tags: [
      ...created("blue").tags,
      ["buzz-related-channel", "22222222-2222-4222-8222-222222222222"],
      ["x-future", "a", "b"],
      ["color", "rose"],
    ],
  };
  const patch = buildProjectColorPatchTemplate({
    color: "amber",
    liveHead: live,
  });
  assert.equal(patch.kind, 30621);
  assert.equal(patch.content, "kept verbatim");
  assert.deepEqual(patch.tags, [
    ...live.tags.filter((tag) => tag[0] !== "color"),
    ["color", "amber"],
  ]);
  assert.equal(readOne({ ...live, ...patch }).color, "amber");
  assert.throws(
    () => buildProjectColorPatchTemplate({ color: "plaid", liveHead: live }),
    /color is invalid/,
  );
});
