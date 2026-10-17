import assert from "node:assert/strict";
import test from "node:test";

import { setProjectColor } from "./useSetProjectColor.ts";

const AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);

const project = {
  id: `30621:${AGENT}:reset`,
  dtag: "reset",
  name: "28 Day Reset",
  owner: AGENT,
  createdAt: 100,
  color: "lime",
};

const liveHead = {
  id: "f".repeat(64),
  kind: 30621,
  pubkey: AGENT,
  created_at: 100,
  content: "",
  tags: [
    ["d", "reset"],
    ["name", "28 Day Reset"],
  ],
  sig: "0".repeat(128),
};

function viaAgent(reply) {
  const sent = [];
  return {
    sent,
    deps: {
      fetchEvents: async () => [liveHead],
      publishOwnedAgentAnnouncements: async (agent, templates) => {
        sent.push(agent);
        return [{ ...liveHead, ...reply(templates[0]) }];
      },
      publishOwnerAnnouncement: async () => {
        throw new Error("must go through the owner agent");
      },
    },
  };
}

test("a color save never asks another project's agent to sign", async () => {
  const harness = viaAgent((template) => template);
  await assert.rejects(
    setProjectColor(
      { color: "teal", ownerControlAgentPubkey: OTHER_AGENT, project },
      harness.deps,
    ),
    /another owner/,
  );
  assert.deepEqual(harness.sent, [], "nothing was sent to sign");
});

test("an agent reply for another owner or project is refused, not cached", async () => {
  for (const reply of [
    (template) => ({ ...template, pubkey: OTHER_AGENT }),
    (template) => ({
      ...template,
      tags: template.tags.map((tag) => (tag[0] === "d" ? ["d", "other"] : tag)),
    }),
  ]) {
    await assert.rejects(
      setProjectColor(
        { color: "teal", ownerControlAgentPubkey: AGENT, project },
        viaAgent(reply).deps,
      ),
      /different project/,
    );
  }
});

test("the project's own agent saves the color", async () => {
  const harness = viaAgent((template) => ({ ...template, created_at: 101 }));
  const saved = await setProjectColor(
    { color: "teal", ownerControlAgentPubkey: AGENT, project },
    harness.deps,
  );
  assert.deepEqual(harness.sent, [AGENT]);
  assert.equal(saved.color, "teal");
  assert.equal(saved.createdAt, 101);
});
