import assert from "node:assert/strict";
import test from "node:test";

import {
  GUEST_AGENT_MENTION_HINT,
  resolveWelcomeComposerHint,
} from "./welcomeComposerHint.ts";

const BASE = {
  channelType: "stream",
  guestHintDone: false,
  hasMentionableAgents: true,
  isGuest: false,
  isWelcomeChannel: true,
};

test("people with the Welcome Team are pointed at @Fizz in the welcome channels only", () => {
  assert.equal(resolveWelcomeComposerHint(BASE), "welcome-team");
  assert.equal(
    resolveWelcomeComposerHint({ ...BASE, hasMentionableAgents: false }),
    "welcome-team",
  );
  assert.equal(
    resolveWelcomeComposerHint({ ...BASE, isWelcomeChannel: false }),
    null,
  );
});

test("a guest is told how to ask agents in any channel where one answers them", () => {
  const guest = { ...BASE, isGuest: true };
  assert.equal(resolveWelcomeComposerHint(guest), "guest-agents");
  assert.equal(
    resolveWelcomeComposerHint({ ...guest, isWelcomeChannel: false }),
    "guest-agents",
  );
});

test("a guest gets no hint where no agent answers, outside channels, or once done", () => {
  const guest = { ...BASE, isGuest: true, isWelcomeChannel: false };
  const cases = [
    { hasMentionableAgents: false },
    { channelType: "dm" },
    { channelType: "forum" },
    { channelType: null },
    { guestHintDone: true },
  ];
  for (const overrides of cases) {
    assert.equal(
      resolveWelcomeComposerHint({ ...guest, ...overrides }),
      null,
      JSON.stringify(overrides),
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
