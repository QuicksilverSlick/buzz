import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const RELAY = "a".repeat(64);
const OWNER = "b".repeat(64);
const AGENT = "c".repeat(64);
const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

function rawItem(id, pubkey, tags) {
  return {
    id,
    kind: 9,
    pubkey,
    content: id,
    created_at: 1,
    channel_id: CHANNEL_ID,
    channel_name: "reset",
    channel_type: "stream",
    tags: [["h", CHANNEL_ID], ...tags],
    category: "mention",
  };
}

// The engine p-tags the owner on every workflow post (workflow_sink.rs).
const workflowTags = [
  ["p", OWNER],
  ["buzz:workflow", "true"],
  ["buzz:workflow-owner", OWNER],
];

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  globalThis.__TAURI_INTERNALS__ = {
    invoke: (cmd) => {
      if (cmd === "get_identity") {
        return Promise.resolve({ pubkey: OWNER, display_name: "Owner" });
      }
      if (cmd === "get_relay_self") return Promise.resolve(RELAY);
      if (cmd === "get_feed") {
        return Promise.resolve({
          feed: {
            mentions: [
              rawItem("recheck-nudge", RELAY, [
                ...workflowTags,
                ["p", AGENT],
                ["buzz:workflow-mention", AGENT],
              ]),
              rawItem("owner-notice", RELAY, [
                ...workflowTags,
                ["buzz:workflow-mention", OWNER],
              ]),
              rawItem("person-mention", AGENT, [["p", OWNER]]),
            ],
            needs_action: [],
            activity: [rawItem("recheck-reply", RELAY, workflowTags)],
            agent_activity: [],
          },
          meta: { since: 0, total: 4, generated_at: 0 },
        });
      }
      return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
    },
    transformCallback: () => Math.random(),
  };
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

test("the home feed leaves out the viewer's own workflow nudges", async () => {
  const { cleanup, renderHook, waitFor } = await import(
    "@testing-library/react"
  );
  const React = (await import("react")).default;
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { useHomeFeedQuery } = await import("./hooks.ts");
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);

  const { result } = renderHook(() => useHomeFeedQuery(), { wrapper });
  try {
    // The filter waits for both the identity and the relay key.
    await waitFor(() =>
      assert.equal(result.current.data?.feed.activity.length, 0),
    );
    assert.deepEqual(
      result.current.data.feed.mentions.map((item) => item.id),
      ["owner-notice", "person-mention"],
    );
  } finally {
    cleanup();
    queryClient.clear();
  }
});
