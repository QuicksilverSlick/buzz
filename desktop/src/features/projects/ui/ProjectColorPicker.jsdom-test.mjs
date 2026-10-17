/**
 * The project color picker: one native radio group, so the browser gives
 * it one Tab stop and arrow keys (jsdom can't move them; what it pins is
 * the group the browser needs for that), every swatch named, the pick
 * checked and named in text, a visible focus ring, and a change per pick.
 */
import assert from "node:assert/strict";
import test from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { PROJECT_COLORS } from "../lib/projectColors.ts";
import { ProjectColorPicker } from "./ProjectColorPicker.tsx";

async function mount(props) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () =>
    root.render(
      React.createElement(ProjectColorPicker, { name: "color", ...props }),
    ),
  );
  return {
    container,
    rerender: (next) =>
      act(async () =>
        root.render(
          React.createElement(ProjectColorPicker, { name: "color", ...next }),
        ),
      ),
    async unmount() {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

test("eight named radios in one group, the pick checked and named", async () => {
  const picks = [];
  const view = await mount({ onChange: (id) => picks.push(id), value: "rose" });
  try {
    const group = view.container.querySelector("fieldset");
    assert.equal(group.querySelector("legend")?.textContent, "Color");
    const radios = [...group.querySelectorAll("input")];
    assert.equal(radios.length, PROJECT_COLORS.length);
    for (const [index, radio] of radios.entries()) {
      assert.equal(radio.type, "radio");
      assert.equal(radio.name, "color", "one group: one Tab stop, arrow keys");
      assert.equal(
        radio.closest("label")?.textContent,
        PROJECT_COLORS[index].label,
      );
      const swatch = radio.nextElementSibling;
      assert.ok(
        swatch.className.includes("peer-focus-visible:outline-2"),
        "keyboard focus shows on the swatch",
      );
    }
    assert.deepEqual(
      radios.filter((radio) => radio.checked).map((radio) => radio.value),
      ["rose"],
    );
    assert.ok(group.textContent.endsWith("Rose"), "the pick's name shows");

    // A label click and an arrow key both check a radio and fire `click`.
    await act(async () => radios[0].closest("label").click());
    await act(async () => radios[5].click());
    assert.deepEqual(picks, ["blue", PROJECT_COLORS[5].id]);

    await view.rerender({ disabled: true, onChange() {}, value: "rose" });
    assert.equal(group.disabled, true);
  } finally {
    await view.unmount();
  }
});
