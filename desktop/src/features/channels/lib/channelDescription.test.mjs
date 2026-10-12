import assert from "node:assert/strict";
import test from "node:test";

import {
  getChannelDescription,
  getChannelDetail,
  getChannelHeaderLine,
} from "./channelDescription.ts";

function makeChannel(overrides = {}) {
  return {
    archivedAt: null,
    channelType: "stream",
    description: "",
    isMember: true,
    purpose: "",
    topic: "",
    ...overrides,
  };
}

test("getChannelDescription falls back when channel is null", () => {
  assert.equal(
    getChannelDescription(null),
    "Connect to the relay to browse channels and read messages.",
  );
});

test("getChannelDescription is empty when no detail fields are set", () => {
  assert.equal(getChannelDescription(makeChannel()), "");
});

test("getChannelDetail provides one shared field order for every surface", () => {
  const channel = makeChannel({
    description: "Description paragraphs.\n\nKeep this structure.",
    purpose: "Legacy purpose",
    topic: "",
  });

  assert.equal(
    getChannelDetail(channel),
    "Description paragraphs.\n\nKeep this structure.",
  );
  assert.equal(getChannelDescription(channel), getChannelDetail(channel));
});

test("getChannelDescription returns single-line detail unchanged", () => {
  assert.equal(
    getChannelDescription(makeChannel({ description: "Team updates." })),
    "Team updates.",
  );
});

test("getChannelDescription preserves paragraph line breaks (AIDA-1980)", () => {
  const detail = "First paragraph.\n\nSecond paragraph with instructions.";
  assert.equal(
    getChannelDescription(makeChannel({ description: detail })),
    detail,
  );
});

test("getChannelDescription puts status prefixes on their own line", () => {
  const detail = "Line one.\nLine two.";
  assert.equal(
    getChannelDescription(
      makeChannel({
        archivedAt: "2026-01-01T00:00:00Z",
        description: detail,
        isMember: false,
      }),
    ),
    `Archived. Read-only until you join this open channel.\n${detail}`,
  );
});

test("getChannelDescription shows prefixes alone when no detail exists", () => {
  assert.equal(
    getChannelDescription(makeChannel({ archivedAt: "2026-01-01T00:00:00Z" })),
    "Archived.",
  );
});

test("getChannelHeaderLine covers every channel, detail and permission case", () => {
  const archivedAt = "2026-01-01T00:00:00Z";
  const cases = [
    { name: "no channel", channel: null, canManage: true, kind: "none" },
    {
      name: "direct message",
      channel: makeChannel({ channelType: "dm", description: "Hi" }),
      canManage: true,
      kind: "none",
    },
    {
      name: "described, member",
      channel: makeChannel({ description: "Ask here." }),
      canManage: false,
      kind: "description",
    },
    {
      name: "described, manager",
      channel: makeChannel({ description: "Ask here." }),
      canManage: true,
      kind: "description",
    },
    {
      name: "described, archived",
      channel: makeChannel({ archivedAt, description: "Ask here." }),
      canManage: true,
      kind: "description",
    },
    {
      name: "described forum",
      channel: makeChannel({ channelType: "forum", description: "Posts." }),
      canManage: false,
      kind: "description",
    },
    {
      name: "empty, member",
      channel: makeChannel(),
      canManage: false,
      kind: "none",
    },
    {
      name: "empty, manager",
      channel: makeChannel(),
      canManage: true,
      kind: "add-description",
    },
    {
      name: "whitespace only, manager",
      channel: makeChannel({ description: "  \n " }),
      canManage: true,
      kind: "add-description",
    },
    {
      name: "empty, archived, manager",
      channel: makeChannel({ archivedAt }),
      canManage: true,
      kind: "none",
    },
  ];

  for (const { name, channel, canManage, kind } of cases) {
    assert.equal(
      getChannelHeaderLine(channel, canManage).kind,
      kind,
      `${name} should be ${kind}`,
    );
  }
});

test("getChannelHeaderLine shows one line but keeps the full text for hover", () => {
  const description = "Your Reset room.\n\nType @Reset Worker to ask.";
  const line = getChannelHeaderLine(
    makeChannel({ description, isMember: false }),
    false,
  );

  assert.deepEqual(line, {
    kind: "description",
    text: "Your Reset room. Type @Reset Worker to ask.",
    title: `Read-only until you join this open channel.\n${description}`,
  });
});
