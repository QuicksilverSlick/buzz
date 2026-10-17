import assert from "node:assert/strict";
import test from "node:test";

import { assertTextContrast } from "@/shared/theme/themeContrastTestHelpers.mjs";
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

test("the palette is eight distinct colors with their classes written out", () => {
  assert.equal(PROJECT_COLORS.length, 8);
  assert.equal(new Set(PROJECT_COLORS.map((c) => c.id)).size, 8);
  for (const color of PROJECT_COLORS) {
    assert.equal(
      color.text,
      `text-${color.id}-800 dark:text-${color.id}-300`,
      color.id,
    );
    assert.equal(
      color.stripe,
      `border-l-${color.id}-600 dark:border-l-${color.id}-400`,
    );
    assert.equal(color.dot, `bg-${color.id}-600 dark:bg-${color.id}-400`);
    assert.ok(color.label);
  }
  assert.equal(projectColorById("sky")?.id, "sky");
  assert.equal(projectColorById("blue"), null);
  assert.equal(projectColorById(undefined), null);
});

test("project text reads at 4.5:1 on the default light and dark themes", async () => {
  await assertTextContrast(PROJECT_COLORS.map((color) => color.id));
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
  const event = created("teal");
  assert.deepEqual(
    event.tags.filter((tag) => tag[0] === "color"),
    [["color", "teal"]],
  );
  assert.equal(readOne(event).color, "teal");
  assert.equal(projectColorOf(readOne(event)).id, "teal");

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
    readOne(withTags(["color", "plaid"], ["color", "pink"])).color,
    "pink",
  );
  assert.equal(
    readOne(withTags(["color", "indigo"], ["color", "pink"])).color,
    "indigo",
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
    ...created("sky"),
    content: "kept verbatim",
    tags: [
      ...created("sky").tags,
      ["buzz-related-channel", "22222222-2222-4222-8222-222222222222"],
      ["x-future", "a", "b"],
      ["color", "pink"],
    ],
  };
  const patch = buildProjectColorPatchTemplate({
    color: "orange",
    liveHead: live,
  });
  assert.equal(patch.kind, 30621);
  assert.equal(patch.content, "kept verbatim");
  assert.deepEqual(patch.tags, [
    ...live.tags.filter((tag) => tag[0] !== "color"),
    ["color", "orange"],
  ]);
  assert.equal(readOne({ ...live, ...patch }).color, "orange");
  assert.throws(
    () => buildProjectColorPatchTemplate({ color: "plaid", liveHead: live }),
    /color is invalid/,
  );
});
