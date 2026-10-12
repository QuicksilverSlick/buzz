import assert from "node:assert/strict";
import test from "node:test";

import {
  GUEST_AGENT_MENTION_HINT,
  resolveWelcomeComposerHint,
} from "./welcomeComposerHint.ts";

test("resolveWelcomeComposerHint points guests at agents only when they can see some", () => {
  const cases = [
    { isGuest: false, hasVisibleAgents: false, expected: "welcome-team" },
    { isGuest: false, hasVisibleAgents: true, expected: "welcome-team" },
    { isGuest: true, hasVisibleAgents: true, expected: "guest-agents" },
    { isGuest: true, hasVisibleAgents: false, expected: null },
  ];
  for (const { expected, ...input } of cases) {
    assert.equal(
      resolveWelcomeComposerHint(input),
      expected,
      JSON.stringify(input),
    );
  }
});

test("the guest hint tells people how to ask an agent, without naming Fizz", () => {
  assert.equal(
    GUEST_AGENT_MENTION_HINT,
    "Type @ and an agent's name to ask it for something.",
  );
  assert.doesNotMatch(GUEST_AGENT_MENTION_HINT, /Fizz/);
});
