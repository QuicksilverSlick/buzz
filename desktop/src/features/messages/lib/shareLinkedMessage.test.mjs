import assert from "node:assert/strict";
import test from "node:test";

import { getMentionOffsets } from "../../../shared/lib/mentionBoundaries.ts";
import {
  findLinkedMessageShare,
  formatSharedQuote,
  neutralizeMentions,
} from "./shareLinkedMessage.ts";

const OWNER = "a".repeat(64);
const AGENT = "b".repeat(64);
const GUEST = "c".repeat(64);
const NPUB = "npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg";

function channel(overrides) {
  return {
    channelType: "stream",
    id: "private-1",
    isMember: true,
    name: "planning",
    visibility: "private",
    ...overrides,
  };
}

const linkTo = (channelId, id = "e".repeat(64), thread = "") =>
  `buzz://message?channel=${channelId}&id=${id}${thread ? `&thread=${thread}` : ""}`;

function context(overrides = {}) {
  const calls = { members: [], messages: [] };
  return {
    calls,
    value: {
      channelId: "here",
      channels: [
        channel({}),
        channel({ id: "open-1", name: "general", visibility: "open" }),
        channel({ id: "not-mine", isMember: false }),
        channel({
          channelType: "dm",
          id: "dm-1",
          name: "Reset Worker",
          visibility: "open",
        }),
      ],
      memberPubkeys: new Set([OWNER, AGENT]),
      ownerPubkey: OWNER,
      getMemberPubkeys: async (id) => {
        calls.members.push(id);
        return [OWNER.toUpperCase()];
      },
      getMessage: async (link) => {
        calls.messages.push(link.messageId);
        return {
          kind: "ready",
          author: "Russell",
          content: "Ship it @Reset Worker",
          createdAt: 1_760_000_000,
        };
      },
      getDisplayName: (pubkey) => (pubkey === AGENT ? "Reset Worker" : pubkey),
      ...overrides,
    },
  };
}

test("prompts when a recipient is not in the linked private channel", async () => {
  const { value } = context();
  const share = await findLinkedMessageShare(
    `see ${linkTo("private-1")}`,
    value,
  );
  assert.deepEqual(share?.names, ["Reset Worker"]);
  assert.match(
    share?.quote ?? "",
    /^> Quoted from \*\*Russell\*\* in #planning, /,
  );
});

test("treats a DM as private and labels it", async () => {
  const { value } = context();
  const share = await findLinkedMessageShare(
    `<${linkTo("dm-1", "f".repeat(64), "d".repeat(64))}>`,
    value,
  );
  assert.match(share?.quote ?? "", /in a thread in DM with Reset Worker, /);
});

test("no prompt when every recipient can already open the link", async () => {
  const { value } = context({
    getMemberPubkeys: async () => [OWNER, AGENT],
  });
  assert.equal(await findLinkedMessageShare(linkTo("private-1"), value), null);
});

test("no prompt for public, same-channel, unknown or unreadable links", async () => {
  const { calls, value } = context();
  for (const href of [
    linkTo("open-1"),
    linkTo("here"),
    linkTo("unknown"),
    linkTo("not-mine"),
    "buzz://message?channel=private-1",
  ]) {
    assert.equal(await findLinkedMessageShare(`x ${href} y`, value), null);
  }
  assert.deepEqual(calls.members, []);

  const gone = context({ getMessage: async () => ({ kind: "deleted" }) });
  assert.equal(
    await findLinkedMessageShare(linkTo("private-1"), gone.value),
    null,
  );
});

test("one prompt covers several private links", async () => {
  const { value } = context({
    memberPubkeys: new Set([OWNER, AGENT, GUEST]),
    getMemberPubkeys: async (id) => (id === "dm-1" ? [OWNER, AGENT] : [OWNER]),
  });
  const share = await findLinkedMessageShare(
    `${linkTo("private-1")} and ${linkTo("dm-1")} and ${linkTo("private-1")}`,
    value,
  );
  assert.deepEqual(share?.names, ["Reset Worker", GUEST]);
  assert.equal(share?.quote.split("\n\n").length, 2);
});

test("quote format: labeled blockquote, attachments as link text", () => {
  const quote = formatSharedQuote(
    {
      author: "Reset Worker",
      content:
        "Done.\n\n![image](https://x.test/a.png)\n[report.pdf](https://x.test/r.pdf)",
      createdAt: 1_760_000_000,
    },
    "DM with Reset Worker",
  );
  const lines = quote.split("\n");
  assert.match(
    lines[0],
    /^> Quoted from \*\*Reset Worker\*\* in DM with Reset Worker, .+2025 at .+:$/,
  );
  assert.deepEqual(lines.slice(1), [
    ">",
    "> Done.",
    ">",
    "> [image](https://x.test/a.png)",
    "> [report.pdf](https://x.test/r.pdf)",
  ]);
});

test("mentions and nostr references inside a quote never ping", () => {
  const raw = `hey @Reset Worker and @russell, see nostr:${NPUB} or NOSTR:${NPUB}`;
  assert.deepEqual(getMentionOffsets(raw, "Reset Worker"), [4]);
  const quote = formatSharedQuote(
    { author: "@Russell", content: raw, createdAt: 0 },
    "#planning",
  );
  assert.deepEqual(getMentionOffsets(quote, "Reset Worker"), []);
  assert.deepEqual(getMentionOffsets(quote, "russell"), []);
  assert.doesNotMatch(quote, /nostr:/i);
  assert.doesNotMatch(quote, /(^|\s)@[A-Za-z]/);
  // Invisible: stripping the joiner gives back the original text.
  assert.equal(neutralizeMentions(raw).replaceAll("⁠", ""), raw);
});
