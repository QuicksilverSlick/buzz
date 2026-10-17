import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    Element: dom.window.Element,
    Event: dom.window.Event,
    FocusEvent: dom.window.FocusEvent,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

/**
 * Mounts the composer after the surrounding chrome is already interactive and
 * focused, the order the huddle room produces when its chat pane loads late.
 * Returns how many times the hook asked the editor to take focus.
 */
async function autofocusCallsWhenFocusIsOn(placement) {
  const React = await import("react");
  const { act, render } = await import("@testing-library/react");
  const { useComposerAutofocus } = await import("./useComposerAutofocus.ts");
  const h = React.createElement;
  let focusCalls = 0;

  function Composer() {
    const formRef = React.useRef(null);
    useComposerAutofocus(
      () => {
        focusCalls += 1;
      },
      "channel-1",
      false,
      formRef,
    );
    return h("form", { ref: formRef });
  }

  function Layout({ composerMounted }) {
    const control =
      placement === "input"
        ? h("input", { "aria-label": "Focused control" })
        : h("button", { "aria-label": "Focused control", type: "button" });
    const composer = composerMounted ? h(Composer) : null;
    if (placement === "dialog-around-composer") {
      return h("div", { role: "dialog" }, control, composer);
    }
    const layer =
      placement === "button" || placement === "input"
        ? control
        : h("div", { role: placement }, control);
    return h(React.Fragment, null, layer, composer);
  }

  const view = render(h(Layout, { composerMounted: false }));
  view.getByLabelText("Focused control").focus();
  await act(async () => {
    view.rerender(h(Layout, { composerMounted: true }));
  });
  return focusCalls;
}

for (const [placement, expectedCalls] of [
  // A plain control, like the sidebar link that navigated here.
  ["button", 1],
  ["input", 0],
  // An open popover, menu, or dialog the user opened before the composer mounted.
  ["dialog", 0],
  ["alertdialog", 0],
  ["menu", 0],
  // The composer lives inside the overlay, like the video review dialog.
  ["dialog-around-composer", 1],
]) {
  test(`autofocus with focus on ${placement} calls focus ${expectedCalls}x`, async () => {
    assert.equal(await autofocusCallsWhenFocusIsOn(placement), expectedCalls);
  });
}
