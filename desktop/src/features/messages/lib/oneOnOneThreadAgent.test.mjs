import assert from "node:assert/strict";
import test from "node:test";

import { buildAgentAddressMentionTags } from "./agentAddressMention.mjs";
import {
  selectOneOnOneThreadAgent,
  syncOneOnOneThreadAgent,
} from "./oneOnOneThreadAgent.ts";
import {
  addPersistentAgentAudienceMember,
  excludePersistentAgentAudienceMember,
  getPersistentAgentAudienceScope,
  getPersistentAgentAudienceSnapshot,
} from "./persistentAgentAudience.ts";

const me = "1".repeat(64);
const person = "2".repeat(64);
const agentA = "a".repeat(64);
const agentB = "b".repeat(64);
const isAgentPubkey = (pubkey) => pubkey === agentA || pubkey === agentB;

function decide(participants, replyToPubkeys = [participants?.[0]?.pubkey]) {
  return selectOneOnOneThreadAgent({
    currentPubkey: me,
    isAgentPubkey,
    participants,
    replyToPubkeys,
  });
}

function scopeFor(thread) {
  return getPersistentAgentAudienceScope({
    ownerPubkey: me,
    channelId: "channel-a",
    composerKey: `thread:${thread}`,
  });
}

function audience(scope) {
  return getPersistentAgentAudienceSnapshot().audiences[scope] ?? [];
}

test("an agent-rooted thread with only you addresses that agent", () => {
  assert.equal(
    decide([
      { pubkey: agentA, tags: [["p", me]] },
      { pubkey: me, tags: [["p", me]] },
    ]),
    agentA,
  );
});

test("a thread where another person posted or was tagged is shared", () => {
  const root = { pubkey: agentA, tags: [] };
  assert.equal(decide([root, { pubkey: person }]), "shared");
  assert.equal(decide([root, { pubkey: me, tags: [["p", person]] }]), "shared");
});

test("a thread with two agents is shared", () => {
  assert.equal(decide([{ pubkey: agentA }, { pubkey: agentB }]), "shared");
});

test("the agent must have written the root or the parent, and the thread be loaded", () => {
  const ownRoot = [{ pubkey: me, tags: [["p", agentA]] }, { pubkey: agentA }];
  assert.equal(decide(ownRoot, [me]), null);
  assert.equal(decide(ownRoot, [me, agentA]), agentA);
  assert.equal(decide([{ pubkey: person }]), null);
  assert.equal(decide(null, [agentA]), null);
});

test("the chip is pre-addressed, and removing it sends with no tag", () => {
  const scope = scopeFor("one-on-one");
  syncOneOnOneThreadAgent(scope, agentA);
  assert.deepEqual(audience(scope), [agentA]);
  assert.equal(
    buildAgentAddressMentionTags(audience(scope), [agentA]).length,
    1,
  );

  // The chip's remove button excludes the agent for this conversation.
  excludePersistentAgentAudienceMember(scope, agentA);
  syncOneOnOneThreadAgent(scope, agentA);
  assert.deepEqual(audience(scope), []);
  assert.deepEqual(buildAgentAddressMentionTags(audience(scope), [agentA]), []);
});

test("a shared thread hands back only the agent this rule added", () => {
  const scope = scopeFor("became-shared");
  syncOneOnOneThreadAgent(scope, agentA);
  assert.equal(syncOneOnOneThreadAgent(scope, "shared"), agentA);
  assert.equal(syncOneOnOneThreadAgent(scope, "shared"), null);

  const pinned = scopeFor("explicit-pin");
  addPersistentAgentAudienceMember(pinned, agentA);
  assert.equal(syncOneOnOneThreadAgent(pinned, "shared"), null);
  assert.deepEqual(audience(pinned), [agentA]);
});
