import assert from "node:assert/strict";
import test from "node:test";

import {
  agentIgnoresViewer,
  MENTION_AGENT_DETAIL_LIMIT,
  mentionAgentAudienceNote,
  mentionAgentDescription,
  mentionAgentDetailPubkeys,
  resolveAgentRespondPolicy,
  withMentionAgentDetails,
} from "./mentionAgentDetails.ts";

const VIEWER = "a".repeat(64);
const OWNER = "b".repeat(64);
const AGENT = "c".repeat(64);
const OTHER_AGENT = "d".repeat(64);
const OWNER_PROFILES = {
  [OWNER]: { displayName: "Russell", avatarUrl: null, nip05Handle: null },
};

function relayAgent(overrides = {}) {
  return {
    pubkey: AGENT,
    ownerPubkey: OWNER,
    name: "Reset Worker",
    agentType: "goose",
    channels: [],
    channelIds: [],
    capabilities: [],
    status: "online",
    respondTo: "anyone",
    respondToAllowlist: [],
    ...overrides,
  };
}

function managedAgent(overrides = {}) {
  return {
    pubkey: AGENT,
    name: "Reset Worker",
    personaId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

function policy(overrides = {}) {
  return {
    ownerPubkey: OWNER,
    respondTo: "anyone",
    respondToAllowlist: [],
    ...overrides,
  };
}

test("descriptions collapse to one trimmed line", () => {
  assert.equal(mentionAgentDescription(null), null);
  assert.equal(mentionAgentDescription(undefined), null);
  assert.equal(mentionAgentDescription("  \n "), null);
  assert.equal(
    mentionAgentDescription("  Plans your week.\n\nAsk me   for a check-in. "),
    "Plans your week. Ask me for a check-in.",
  );
});

test("an agent on this desktop is the viewer's own and its setting wins", () => {
  const resolved = resolveAgentRespondPolicy(AGENT.toUpperCase(), {
    currentPubkey: VIEWER,
    managedAgents: [managedAgent({ respondTo: "allowlist" })],
    relayAgents: [relayAgent({ respondTo: "owner-only" })],
  });
  assert.deepEqual(resolved, {
    ownerPubkey: VIEWER,
    respondTo: "allowlist",
    respondToAllowlist: [],
  });
});

test("an agent run elsewhere uses the owner's published setting", () => {
  assert.deepEqual(
    resolveAgentRespondPolicy(AGENT, {
      currentPubkey: VIEWER,
      managedAgents: [],
      relayAgents: [relayAgent({ respondTo: "allowlist" })],
    }),
    { ownerPubkey: OWNER, respondTo: "allowlist", respondToAllowlist: [] },
  );
  assert.equal(
    resolveAgentRespondPolicy(OTHER_AGENT, {
      currentPubkey: VIEWER,
      managedAgents: undefined,
      relayAgents: [relayAgent()],
    }),
    null,
  );
});

test("only a known setting that leaves the viewer out counts", () => {
  const cases = [
    [policy({ respondTo: "anyone" }), false],
    [policy({ respondTo: "owner-only" }), true],
    [policy({ respondTo: "allowlist", respondToAllowlist: [] }), true],
    [
      policy({ respondTo: "allowlist", respondToAllowlist: [OWNER, VIEWER] }),
      false,
    ],
    [
      policy({
        respondTo: "allowlist",
        respondToAllowlist: [VIEWER.toUpperCase()],
      }),
      false,
    ],
    [policy({ respondTo: null }), false],
    [policy({ ownerPubkey: VIEWER, respondTo: "owner-only" }), false],
    [policy({ ownerPubkey: VIEWER, respondTo: "allowlist" }), false],
    [null, false],
  ];
  for (const [value, expected] of cases) {
    assert.equal(
      agentIgnoresViewer(value, VIEWER),
      expected,
      JSON.stringify(value),
    );
  }
  assert.equal(
    agentIgnoresViewer(policy({ respondTo: "owner-only" }), null),
    false,
  );
});

test("the note names the owner the agent answers", () => {
  assert.equal(
    mentionAgentAudienceNote(
      policy({ respondTo: "owner-only" }),
      VIEWER,
      OWNER_PROFILES,
    ),
    "Answers only Russell",
  );
  assert.equal(
    mentionAgentAudienceNote(
      policy({ respondTo: "allowlist" }),
      VIEWER,
      OWNER_PROFILES,
    ),
    "Answers only Russell",
  );
  assert.match(
    mentionAgentAudienceNote(policy({ respondTo: "owner-only" }), VIEWER, {}),
    /^Answers only npub1/,
  );
  assert.equal(
    mentionAgentAudienceNote(
      policy({ ownerPubkey: null, respondTo: "owner-only" }),
      VIEWER,
      OWNER_PROFILES,
    ),
    null,
  );
  assert.equal(
    mentionAgentAudienceNote(policy(), VIEWER, OWNER_PROFILES),
    null,
  );
});

test("profiles are looked up only for agent identities, at most a few", () => {
  const suggestions = [
    { displayName: "Ada", pubkey: "e".repeat(64) },
    { displayName: "Team", kind: "team", isAgent: true, teamId: "t1" },
    { displayName: "Persona", kind: "persona", isAgent: true, personaId: "p1" },
    { displayName: "Worker", isAgent: true, pubkey: AGENT.toUpperCase() },
    { displayName: "Worker again", isAgent: true, pubkey: AGENT },
    ...Array.from({ length: MENTION_AGENT_DETAIL_LIMIT + 5 }, (_, index) => ({
      displayName: `Agent ${index}`,
      isAgent: true,
      pubkey: index.toString(16).padStart(64, "0"),
    })),
  ];
  const pubkeys = mentionAgentDetailPubkeys(suggestions);
  assert.equal(pubkeys.length, MENTION_AGENT_DETAIL_LIMIT);
  assert.equal(pubkeys[0], AGENT);
  assert.equal(new Set(pubkeys).size, pubkeys.length);
});

test("agent suggestions gain the About line and the audience note", () => {
  const person = { displayName: "Ada", pubkey: "e".repeat(64) };
  const worker = { displayName: "Reset Worker", isAgent: true, pubkey: AGENT };
  const persona = {
    displayName: "Planner",
    isAgent: true,
    kind: "persona",
    personaId: "p1",
  };
  const team = { displayName: "Crew", isAgent: true, kind: "team" };
  const next = withMentionAgentDetails([person, worker, persona, team], {
    aboutByPubkey: new Map([[AGENT, "Builds your\nplan."]]),
    currentPubkey: VIEWER,
    managedAgents: [],
    ownerProfiles: OWNER_PROFILES,
    personas: [{ id: "p1", description: "  Plans the week. " }],
    relayAgents: [relayAgent({ respondTo: "owner-only" })],
  });

  assert.equal(next[0], person);
  assert.deepEqual(next[1], {
    ...worker,
    audienceNote: "Answers only Russell",
    description: "Builds your plan.",
  });
  assert.deepEqual(next[2], {
    ...persona,
    audienceNote: null,
    description: "Plans the week.",
  });
  assert.equal(next[3], team);
});

test("an agent with no About falls back to its persona description", () => {
  const worker = {
    displayName: "Reset Worker",
    isAgent: true,
    personaId: "p1",
    pubkey: AGENT,
  };
  const [next] = withMentionAgentDetails([worker], {
    aboutByPubkey: new Map(),
    currentPubkey: VIEWER,
    managedAgents: [managedAgent({ personaId: "p1" })],
    personas: [{ id: "p1", description: "Builds your plan." }],
    relayAgents: [],
  });
  assert.equal(next.description, "Builds your plan.");
  assert.equal(next.audienceNote, null);
});

test("the list keeps its identity when nothing changes", () => {
  const suggestions = [
    { displayName: "Ada", pubkey: "e".repeat(64) },
    { displayName: "Reset Worker", isAgent: true, pubkey: AGENT },
  ];
  const options = {
    aboutByPubkey: new Map(),
    currentPubkey: VIEWER,
    managedAgents: [],
    personas: [],
    relayAgents: [relayAgent()],
  };
  assert.equal(withMentionAgentDetails(suggestions, options), suggestions);

  const described = withMentionAgentDetails(suggestions, {
    ...options,
    aboutByPubkey: new Map([[AGENT, "Builds your plan."]]),
  });
  assert.notEqual(described, suggestions);
  assert.equal(
    withMentionAgentDetails(described, {
      ...options,
      aboutByPubkey: new Map([[AGENT, "Builds your plan."]]),
    }),
    described,
  );
});
