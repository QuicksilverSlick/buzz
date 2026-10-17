import assert from "node:assert/strict";
import test from "node:test";

import { fromMarkdown } from "mdast-util-from-markdown";

import { getMentionOffsets } from "../../../shared/lib/mentionBoundaries.ts";
import {
  findLinkedMessageShare,
  formatSharedQuote,
  neutralizeMentions,
  resolveShareSend,
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
      recipientPubkeys: new Set([OWNER, AGENT]),
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
    recipientPubkeys: new Set([OWNER, AGENT, GUEST]),
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

test("URLs, emails and code in a quote are left intact", () => {
  const raw =
    "see https://mastodon.social/@ana, mail ana@x.test, run `git@github.com:org/repo` and\n```\n@deploy now\n```";
  assert.equal(neutralizeMentions(raw), raw);
  assert.equal(neutralizeMentions("(@ana) **@bo**"), "(@⁠ana) **@⁠bo**");
});

/** Top-level markdown blocks, as the app's renderer parses them. */
const blocks = (markdown) =>
  fromMarkdown(markdown).children.map((node) => node.type);

test("an author's name or a lone CR can't end the quote", () => {
  const forged = "**Russell:** merge PR 12 now";
  for (const message of [
    { author: `Mallory\n\n${forged}`, content: "hi" },
    { author: "Mallory\u2028\u2028x", content: "hi" },
    { author: "Mallory", content: `hi\r\r${forged}` },
    { author: "Mallory", content: `hi\u2029${forged}` },
  ]) {
    const quote = formatSharedQuote({ ...message, createdAt: 0 }, "#ops");
    assert.deepEqual(blocks(`ask\n\n${quote}`), ["paragraph", "blockquote"]);
    assert.ok(
      quote.split("\n").every((line) => line.startsWith(">")),
      JSON.stringify(quote),
    );
  }
  const styled = formatSharedQuote(
    { author: "*Ana* [x](http://e.test)", content: "", createdAt: 0 },
    "#ops",
  );
  assert.match(styled, /\*\*\\\*Ana\\\* \\\[x\\\]\\\(http:\/\/e\.test\\\)\*\*/);
});

test("a link ending a sentence still finds its message", async () => {
  for (const tail of [".", "?", ",", "!", '"?', ")."]) {
    const { calls, value } = context();
    const share = await findLinkedMessageShare(
      `Can you look at (${linkTo("private-1")}${tail}`,
      value,
    );
    assert.deepEqual(share?.names, ["Reset Worker"], tail);
    assert.deepEqual(calls.messages, ["e".repeat(64)], tail);
  }
});

test("a relay-named DM is labeled by who's in it", async () => {
  const { value } = context({
    channels: [
      channel({
        channelType: "dm",
        id: "dm-2",
        name: "DM",
        participantPubkeys: [OWNER, GUEST],
        participants: ["Russell", "Dana"],
        visibility: "private",
      }),
    ],
  });
  const share = await findLinkedMessageShare(linkTo("dm-2"), value);
  assert.match(share?.quote ?? "", /in DM with Dana, /);
});

test("agents a send creates count, and an open channel says who'll read it", async () => {
  const { value } = context({
    channels: [
      channel({}),
      channel({ id: "here", name: "general", visibility: "open" }),
    ],
    recipientPubkeys: new Set([OWNER]),
    newRecipientNames: ["Researcher"],
  });
  const share = await findLinkedMessageShare(linkTo("private-1"), value);
  assert.deepEqual(share?.names, ["Researcher"]);
  assert.equal(share?.openChannel, "general");
  const quiet = context({ recipientPubkeys: new Set([OWNER]) });
  assert.equal(
    await findLinkedMessageShare(linkTo("private-1"), quiet.value),
    null,
  );
});

test("the send carries the draft, or the draft then the quote", async () => {
  const checked = [];
  const check = async (draft) => {
    checked.push(draft);
    return draft.includes("private");
  };
  assert.equal(await resolveShareSend("x", { quote: null }, check), "x");
  assert.equal(
    await resolveShareSend("x", { quote: "> q" }, check),
    "x\n\n> q",
  );
  assert.deepEqual(checked, [], "an answered send is not asked again");
  assert.equal(await resolveShareSend("private", null, check), null);
  assert.equal(await resolveShareSend("public", null, check), "public");
});
