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
import {
  ProjectColorPicker,
  useQueuedColorSave,
} from "./ProjectColorPicker.tsx";

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
  const view = await mount({ onChange: (id) => picks.push(id), value: "pink" });
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
      ["pink"],
    );
    assert.ok(group.textContent.endsWith("Pink"), "the pick's name shows");

    // A label click and an arrow key both check a radio and fire `click`.
    await act(async () => radios[0].closest("label").click());
    await act(async () => radios[5].click());
    assert.deepEqual(picks, [PROJECT_COLORS[0].id, PROJECT_COLORS[5].id]);

    await view.rerender({ disabled: true, onChange() {}, value: "pink" });
    assert.equal(group.disabled, true);
  } finally {
    await view.unmount();
  }
});

test("arrowing while a save runs keeps focus and saves only the latest pick", async () => {
  const saves = [];
  function Setting() {
    const [project, setProject] = React.useState({
      color: "lime",
      dtag: "g",
      id: "p:g",
    });
    const color = useQueuedColorSave(project, (next, target) => {
      const done = Promise.withResolvers();
      saves.push({ color: next, target, done });
      return done.promise.then(() => {
        const saved = { ...target, color: next };
        setProject(saved);
        return saved;
      });
    });
    return React.createElement(ProjectColorPicker, {
      busy: color.saving,
      name: "color",
      onChange: color.pick,
      value: color.value,
    });
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(React.createElement(Setting)));
    const group = container.querySelector("fieldset");
    const radios = [...group.querySelectorAll("input")];
    const ids = PROJECT_COLORS.map((c) => c.id);
    // An arrow key moves focus to the next radio and checks it (a click).
    const arrowTo = (index) =>
      act(async () => {
        radios[index].focus();
        radios[index].click();
      });

    await arrowTo(1);
    assert.equal(saves.length, 1);
    assert.equal(group.getAttribute("aria-busy"), "true");
    assert.equal(radios[1].matches(":disabled"), false, "still operable");
    assert.equal(document.activeElement, radios[1], "focus stays put");
    await arrowTo(2);
    await arrowTo(3);
    assert.equal(document.activeElement, radios[3]);
    assert.equal(radios[3].checked, true, "the pick shows at once");
    assert.equal(saves.length, 1, "one save at a time");

    await act(async () => saves[0].done.resolve());
    assert.deepEqual(
      saves.map((save) => save.color),
      [ids[1], ids[3]],
      "the latest pick, not every step",
    );
    assert.equal(saves[1].target.color, ids[1], "on top of the saved head");
    await act(async () => saves[1].done.resolve());
    assert.equal(group.hasAttribute("aria-busy"), false);
    assert.equal(radios[3].checked, true);
    assert.equal(saves.length, 2);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

test("a pick stays on the project it was made on when the setting moves on", async () => {
  const saves = [];
  let show;
  function Setting() {
    const [projects, setProjects] = React.useState({
      a: { color: "lime", dtag: "a", id: "p:a" },
      b: { color: "sky", dtag: "b", id: "p:b" },
    });
    const [current, setCurrent] = React.useState("a");
    show = setCurrent;
    const color = useQueuedColorSave(projects[current], (next, target) => {
      const done = Promise.withResolvers();
      saves.push({ color: next, id: target.id, done });
      return done.promise.then(() => {
        const saved = { ...target, color: next };
        setProjects((all) => ({ ...all, [saved.dtag]: saved }));
        return saved;
      });
    });
    return React.createElement(ProjectColorPicker, {
      name: "color",
      onChange: color.pick,
      value: color.value,
    });
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const checked = () => container.querySelector("input:checked")?.value;
  const pick = (id) =>
    act(async () => container.querySelector(`input[value="${id}"]`).click());
  try {
    await act(async () => root.render(React.createElement(Setting)));
    await pick("red");
    await act(async () => show("b"));
    assert.equal(
      checked(),
      "sky",
      "B shows its own color, not A's pending pick",
    );
    await pick("teal");
    assert.equal(checked(), "teal");
    await act(async () => saves[0].done.resolve());
    await act(async () => saves[1]?.done.resolve());
    assert.deepEqual(
      saves.map((save) => `${save.color}->${save.id}`),
      ["red->p:a", "teal->p:b"],
    );
    assert.equal(checked(), "teal");
    await act(async () => show("a"));
    assert.equal(checked(), "red");
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
