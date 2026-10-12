import assert from "node:assert/strict";
import test from "node:test";

import {
  INBOX_FILTER_OPTIONS,
  visibleInboxFilterOptions,
} from "./inboxFilterOptions.ts";

test("the Projects filter shows only while Projects is on", () => {
  assert.deepEqual(
    visibleInboxFilterOptions(true).map((option) => option.label),
    [
      "All",
      "Projects",
      "Mentions",
      "Threads",
      "Needs action",
      "Agents",
      "Reminders",
      "Drafts",
    ],
  );
  assert.deepEqual(
    visibleInboxFilterOptions(false).map((option) => option.value),
    INBOX_FILTER_OPTIONS.map((option) => option.value).filter(
      (value) => value !== "project",
    ),
  );
});
