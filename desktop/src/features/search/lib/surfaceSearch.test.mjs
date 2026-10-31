import assert from "node:assert/strict";
import test from "node:test";

import { buildInboxItems } from "../../home/lib/inbox.ts";
import * as s from "./surfaceSearch.ts";

const ids = (hits) => hits.map((hit) => hit.id);

test("Inbox hits are the All view's rows, found by sender, #792 as a token", () => {
  const ch = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
  const [ana, bob, bot, mine] = ["a", "b", "c", "d"].map((c) => c.repeat(64));
  const ev = (id, category, content, more = {}) => ({
    ...{ id, kind: 9, pubkey: ana, content, createdAt: 1, channelId: ch },
    ...{ channelName: "general", tags: [["h", ch]], category, ...more },
  });
  const feed = {
    mentions: [
      ev("a", "mention", "Can you look at PR 792 today?"),
      ev("b", "mention", "CI: 7928/7929 passed on 792d3edd", { createdAt: 2 }),
      ev("c", "mention", "PR 792 waits for you", { createdAt: 3 }),
    ],
    needsAction: [ev("r", "needs_action", "PR 792", { kind: 40007 })],
    activity: [ev("d", "activity", "PR 792 shipped", { pubkey: bob })],
    agentActivity: [
      ev("e", "agent_activity", "PR 792 built", { pubkey: bot }),
      ev("f", "agent_activity", "PR 792 tested", { pubkey: mine }),
    ],
  };
  const items = buildInboxItems({
    channels: [{ id: ch, name: "general", channelType: "stream" }],
    feed: { feed, meta: { since: 0, total: 0, generatedAt: 0 } },
    profiles: { [ana]: { displayName: "Ana" } },
  });
  const classify = ({ id }) => ({
    type: "mention",
    benchCard: id === "c" ? {} : null,
  });
  // Not the reminder, Bob's channel post, an agent you don't own or a card.
  const find = (query, limit) =>
    s.inboxHits(items, classify, new Set([mine]), query, limit);
  assert.deepEqual(ids(find("#792")).sort(), ["a", "f"]);
  const [hit] = find("ana", 1);
  assert.deepEqual(
    [hit.kind, hit.meta, hit.target.item],
    ["mention", "Mentioned in #general", "b"],
  );
});

test("Projects find repositories, and cached pull requests by branch", () => {
  const repo = { id: "r1", name: "buzz", webUrl: "https://github.com/x/buzz" };
  const project = {
    id: "p1",
    dtag: "df",
    description: "The factory",
    repositories: [repo],
  };
  const legacy = {
    id: "p2",
    dtag: "relay",
    legacy: true,
    repositories: [{ id: "r2", name: "relay" }],
  };
  const pullRequest = {
    id: "pr1",
    title: "Updater",
    status: "Open",
    labels: [],
    branchName: "fix/updater-check-while-running",
    targetBranch: "main",
  };
  const work = [
    { pullRequests: { items: [{ project, repository: repo, pullRequest }] } },
  ];
  const [pr] = s.projectHits([project], work, "updater-check");
  assert.equal(pr.meta, "buzz · fix/updater-check-while-running → main");
  assert.deepEqual(pr.target, {
    to: "project",
    projectId: "p1",
    repositoryId: "r1",
    pullRequestId: "pr1",
  });
  const find = (projects, query) => ids(s.projectHits(projects, work, query));
  assert.deepEqual(find([project], "github.com/x"), ["repository:r1"]);
  assert.deepEqual(find([project], "factory"), ["project:p1"]);
  // A repository-only project shows once, as its repository.
  assert.deepEqual(find([legacy], "relay"), ["repository:r2"]);
});

test("Personas: the active library, by name and description, never prompt or env", () => {
  const persona = {
    id: "p1",
    displayName: "Release Captain",
    description: "Cuts releases",
    isActive: true,
    systemPrompt: "zebra",
    envVars: { T: "hunter2" },
    model: "opus",
    runtime: "claude",
  };
  const off = { ...persona, id: "p2", isActive: false };
  const find = (query) => s.personaHits([persona, off], query);
  assert.deepEqual(find("release captain")[0].chips, ["opus", "claude"]);
  assert.equal(find("cuts releases").length, 1);
  assert.equal(find("zebra").length, 0);
  assert.equal(find("hunter2").length, 0);
});
