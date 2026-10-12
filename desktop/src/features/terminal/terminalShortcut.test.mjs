import assert from "node:assert/strict";
import test from "node:test";

import { terminalShortcutLabel } from "./terminalShortcut.ts";

test("the terminal shortcut reads ⌘J on a Mac and Ctrl+J elsewhere", () => {
  assert.equal(terminalShortcutLabel(true), "⌘J");
  assert.equal(terminalShortcutLabel(false), "Ctrl+J");
});
