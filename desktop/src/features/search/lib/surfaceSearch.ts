/**
 * Global search over what the app already holds for its other screens:
 * Inbox rows, Projects (projects, repositories and the Buzz Git pull
 * requests the Projects screen cached), Workflows, Pulse notes and the agent
 * library. Pure: each hit names where it opens; the dialog navigates.
 */

import {
  formatInboxTypeLabel,
  type InboxItem,
} from "@/features/home/lib/inbox";
import type { InboxClass } from "@/features/home/lib/inboxClass";
import {
  type ProjectColor,
  projectColorOf,
} from "@/features/projects/lib/projectColors";
import { matchesProjectsSearch } from "@/features/projects/lib/projectsSearch";
import type { Project } from "@/features/projects/projectModels";
import type { ProjectsWorkItemsResult } from "@/features/projects/projectWorkItems";
import { parsePrQuery, prMentionScore } from "@/features/search/lib/prNumber";
import {
  getWorkflowDescription,
  getWorkflowDisplayStatus,
  getWorkflowTriggerSummary,
  getWorkflowTriggerType,
} from "@/features/workflows/ui/workflowDefinition";
import type { AgentPersona } from "@/shared/api/personaTypes";
import type { UserNote } from "@/shared/api/socialTypes";
import type { Workflow } from "@/shared/api/workflowTypes";

export type SurfaceKey =
  | "inbox"
  | "projects"
  | "workflows"
  | "pulse"
  | "agent-library";

type ProjectIds = { repositoryId?: string; pullRequestId?: string };

export type SurfaceTarget =
  | { to: "inbox"; item: string }
  | ({ to: "project"; projectId: string } & ProjectIds)
  | { to: "workflow"; id: string }
  | { to: "profile"; pubkey: string }
  | { to: "persona"; personaId: string };

export type SurfaceHit = {
  surface: SurfaceKey;
  id: string;
  /** Inbox type, "project", "repository", "pull-request", the workflow's
   * trigger, "note" or "persona": the row's icon and label. */
  kind: string;
  title: string;
  meta: string | null;
  chips: string[];
  color: ProjectColor | null;
  target: SurfaceTarget;
};

type Values = ReadonlyArray<string | null | undefined>;
/** A hit and the text it is found by. */
type Candidate = SurfaceHit & { text: Values };

/**
 * PR-shaped queries match whole tokens ("#792" never 7928 or 792d3edd);
 * anything else matches every word, as the Projects search does.
 */
function matchesLocal(query: string, values: Values): boolean {
  const pr = parsePrQuery(query);
  if (!pr) return matchesProjectsSearch(query, values);
  const text = values.filter(Boolean).join("\n");
  return prMentionScore(text, pr.number) >= (pr.explicit ? 2 : 1);
}

/** The first `limit` matching hits, one per id. */
function take(candidates: Candidate[], query: string, limit: number) {
  const ids = new Set<string>();
  const hits = candidates.filter((candidate) => {
    if (ids.has(candidate.id) || !matchesLocal(query, candidate.text)) {
      return false;
    }
    ids.add(candidate.id);
    return true;
  });
  return hits.slice(0, limit);
}

// Rows truncate it.
const firstLine = (text: string) => text.trim().split("\n")[0] ?? "";
const joined = (...parts: Values) => parts.filter(Boolean).join(" · ") || null;

/** Inbox rows; a Bench card's post is left to the Bench cards section. */
export function inboxHits(
  items: readonly InboxItem[],
  classify: (item: InboxItem) => InboxClass,
  query: string,
  limit = 4,
): SurfaceHit[] {
  const candidates = items.flatMap((item): Candidate[] => {
    const { benchCard, color, type, workbench } = classify(item);
    if (benchCard) return [];
    const { channelLabel, preview, senderLabel, subject } = item;
    return [
      {
        surface: "inbox",
        id: item.id,
        kind: type,
        title: firstLine(preview) || subject,
        meta: formatInboxTypeLabel(item),
        chips: workbench ? [workbench] : [],
        color,
        target: { to: "inbox", item: item.id },
        text: [
          subject,
          preview,
          senderLabel,
          channelLabel,
          item.categoryLabel,
          ...item.mentionNames,
        ],
      },
    ];
  });
  return take(candidates, query, limit);
}

/**
 * Projects, their repositories, then cached Buzz Git pull requests, so a
 * branch name finds its PR. A repository-only project is its repository.
 */
export function projectHits(
  projects: readonly Project[],
  workItems: ReadonlyArray<ProjectsWorkItemsResult<Project>>,
  query: string,
  limit = 4,
): SurfaceHit[] {
  const hit = (
    project: Project,
    kind: string,
    found: Pick<Candidate, "id" | "title" | "meta" | "text"> & {
      chips?: string[];
    },
    ids: ProjectIds = {},
  ): Candidate => ({
    chips: [],
    ...found,
    surface: "projects",
    id: `${kind}:${found.id}`,
    kind,
    color: projectColorOf(project),
    target: { to: "project", projectId: project.id, ...ids },
  });
  const candidates: Candidate[] = [];
  for (const project of projects) {
    const { description, dtag, id, legacy, name } = project;
    if (!legacy) {
      const text = [name, description, dtag];
      const meta = description || null;
      candidates.push(hit(project, "project", { id, title: name, meta, text }));
    }
    for (const repo of project.repositories) {
      const text = [repo.name, repo.description, repo.dtag, repo.webUrl];
      const meta = joined(legacy ? null : name, repo.description);
      const found = { id: repo.id, title: repo.name, meta, text };
      candidates.push(
        hit(project, "repository", found, { repositoryId: repo.id }),
      );
    }
  }
  const pullRequests = workItems.flatMap((items) => items.pullRequests.items);
  for (const { project, repository, pullRequest: pr } of pullRequests) {
    const { branchName, labels, status, targetBranch, title } = pr;
    const branches = [branchName, targetBranch].filter(Boolean).join(" → ");
    const meta = joined(repository.name, branches);
    const text = [title, branchName, targetBranch, status, ...labels];
    const found = { id: pr.id, title, meta, text, chips: [status] };
    const ids = { repositoryId: repository.id, pullRequestId: pr.id };
    candidates.push(hit(project, "pull-request", found, ids));
  }
  return take(candidates, query, limit);
}

export function workflowHits(
  list: ReadonlyArray<{ workflow: Workflow; channelName: string }>,
  query: string,
  limit = 3,
): SurfaceHit[] {
  const candidates = list.map(({ workflow, channelName }): Candidate => {
    const { definition, id, name } = workflow;
    const summary = getWorkflowTriggerSummary(definition);
    return {
      surface: "workflows",
      id,
      kind: getWorkflowTriggerType(definition) ?? "workflow",
      title: name,
      meta: joined(summary, channelName && `#${channelName}`),
      chips: [getWorkflowDisplayStatus(workflow)],
      color: null,
      target: { to: "workflow", id },
      text: [name, getWorkflowDescription(definition), summary, channelName],
    };
  });
  return take(candidates, query, limit);
}

/** Pulse has no page per note: a note opens its author's profile. */
export function noteHits(
  notes: readonly UserNote[],
  query: string,
  limit = 3,
): SurfaceHit[] {
  const candidates = notes.map((note): Candidate => {
    const topics = note.tags.flatMap(([name, value]) =>
      name === "t" && value ? [value] : [],
    );
    return {
      surface: "pulse",
      id: note.id,
      kind: "note",
      title: firstLine(note.content),
      meta: topics.map((topic) => `#${topic}`).join(" ") || null,
      chips: [],
      color: null,
      target: { to: "profile", pubkey: note.pubkey },
      text: [note.content, ...topics],
    };
  });
  return take(candidates, query, limit);
}

/** Name and public description only: never the prompt, env or commands. */
export function personaHits(
  personas: readonly AgentPersona[],
  query: string,
  limit = 3,
): SurfaceHit[] {
  const candidates = personas.map(
    (persona): Candidate => ({
      surface: "agent-library",
      id: persona.id,
      kind: "persona",
      title: persona.displayName,
      meta: persona.description,
      chips: [persona.model, persona.runtime].flatMap((chip) =>
        chip ? [chip] : [],
      ),
      color: null,
      target: { to: "persona", personaId: persona.id },
      text: [persona.displayName, persona.description],
    }),
  );
  return take(candidates, query, limit);
}
