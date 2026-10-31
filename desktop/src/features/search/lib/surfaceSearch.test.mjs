import assert from "node:assert/strict";
import test from "node:test";

import * as s from "./surfaceSearch.ts";

const ids = (hits) => hits.map((hit) => hit.id);

test("Inbox rows keep their type, leave card posts to the Bench, match #792 as a token", () => {
  const row = (id, preview) => ({
    id,
    preview,
    subject: "Mention",
    senderLabel: "Ana",
    channelLabel: "general",
    mentionNames: [],
    groupItems: [],
    item: { id, kind: 9, tags: [], channelType: "stream", category: "mention" },
  });
  const items = [
    row("a", "Can you look at PR 792 today?"),
    row("b", "CI: 7928/7929 passed on 792d3edd"),
    row("c", "PR 792 waits for you"),
  ];
  const classify = ({ id }) => ({
    type: "mention",
    benchCard: id === "c" ? {} : null,
  });
  const hits = s.inboxHits(items, classify, "#792");
  assert.deepEqual(
    hits.map((hit) => [hit.id, hit.kind, hit.meta, hit.target.item]),
    [["a", "mention", "Mentioned in #general", "a"]],
  );
  assert.deepEqual(ids(s.inboxHits(items, classify, "ana", 1)), ["a"]);
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

test("Personas match name and description, never prompt or env", () => {
  const persona = {
    id: "p1",
    displayName: "Release Captain",
    description: "Cuts releases",
    systemPrompt: "zebra",
    envVars: { T: "hunter2" },
    model: "opus",
    runtime: "claude",
  };
  const find = (query) => s.personaHits([persona], query);
  assert.deepEqual(find("release captain")[0].chips, ["opus", "claude"]);
  assert.equal(find("cuts releases").length, 1);
  assert.equal(find("zebra").length, 0);
  assert.equal(find("hunter2").length, 0);
});
