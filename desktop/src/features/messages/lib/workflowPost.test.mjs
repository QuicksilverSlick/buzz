import assert from "node:assert/strict";
import test from "node:test";

import { isOwnWorkflowPost, isRelayWorkflowPost } from "./workflowPost.ts";

const RELAY = "a".repeat(64);
const OWNER = "b".repeat(64);
const AGENT = "c".repeat(64);
const OTHER = "d".repeat(64);

// The tags the relay workflow sink puts on every post (workflow_sink.rs).
function workflowPost(extraTags = [], pubkey = RELAY) {
  return {
    pubkey,
    tags: [
      ["p", OWNER],
      ["h", "channel-1"],
      ["buzz:workflow", "true"],
      ["buzz:workflow-owner", OWNER],
      ...extraTags,
    ],
  };
}

test("a relay-signed post with the workflow marker is a workflow post", () => {
  assert.equal(isRelayWorkflowPost(workflowPost(), RELAY), true);
  assert.equal(isRelayWorkflowPost(workflowPost(), RELAY.toUpperCase()), true);
});

test("the workflow tag on someone else's event is not a workflow post", () => {
  assert.equal(isRelayWorkflowPost(workflowPost([], OTHER), RELAY), false);
});

test("an unknown relay identity fails closed", () => {
  assert.equal(isRelayWorkflowPost(workflowPost(), null), false);
  assert.equal(isRelayWorkflowPost(workflowPost(), undefined), false);
});

test("a relay post without the marker is not a workflow post", () => {
  assert.equal(
    isRelayWorkflowPost({ pubkey: RELAY, tags: [["p", OWNER]] }, RELAY),
    false,
  );
});

test("the owner's own workflow nudge to an agent is an own workflow post", () => {
  const nudge = workflowPost([
    ["p", AGENT],
    ["buzz:workflow-mention", AGENT],
  ]);
  assert.equal(isOwnWorkflowPost(nudge, OWNER, RELAY), true);
});

test("a workflow template that names its owner still reaches the owner", () => {
  const notice = workflowPost([["buzz:workflow-mention", OWNER]]);
  assert.equal(isOwnWorkflowPost(notice, OWNER, RELAY), false);
});

test("another person's workflow post is not the viewer's own", () => {
  assert.equal(isOwnWorkflowPost(workflowPost(), OTHER, RELAY), false);
});

test("a forged owner tag from a non-relay author is never hidden", () => {
  assert.equal(isOwnWorkflowPost(workflowPost([], OTHER), OWNER, RELAY), false);
});
